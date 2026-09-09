#!/usr/bin/env python3
"""Controls for histories whose answers are independently known."""
import copy
import itertools
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import random
import unittest

from checker import History, Malformed, check, minimize_prefix, search, verify_witness
from workload import admission_refusal, parse_success


def header(kv=(), chunk=2):
    return {"type": "header", "version": 1, "range_chunk_size": chunk,
            "initial": {"keyspaces": [{"name": "test", "id": 1}],
                        "kv": [{"keyspace": 1, "key": k, "value": v} for k, v in kv]}}


def call(i, op, **args):
    if op != 'create_keyspace':
        args = {"keyspace": 1, **args}
    return {"type": "invoke", "id": i, "client": str(i % 3), "op": op, "args": args}


def returned(i, outcome='ok', **result):
    return {"type": "return", "id": i, "outcome": outcome, "result": result}


def history(*events, initial=None):
    return History.parse([initial or header(), *[{**e, 'seq': i} for i, e in enumerate(events)]])


class CheckerControls(unittest.TestCase):
    def test_differential_against_permutations_for_small_atomic_histories(self):
        # Independent oracle: enumerate subsets of unknown writes and every
        # total ordering, then check all real-time edges and ordinary dict I/O.
        rng = random.Random(120912)
        for _ in range(200):
            scheduled = []
            for i in range(4):
                kind = rng.choice(['put', 'get', 'delete'])
                args = {'key': rng.choice(['61', '62'])}
                if kind == 'put': args['value'] = rng.choice(['31', '32'])
                invocation = rng.randrange(10)
                response = invocation + rng.randrange(1, 8)
                outcome = 'unknown' if kind != 'get' and rng.randrange(3) == 0 else 'ok'
                result = {'value': rng.choice([None, '31', '32'])} if kind == 'get' else {}
                scheduled += [(invocation, 2*i, call(i, kind, **args)), (response, 2*i+1, returned(i, outcome, **result))]
            h = history(*(event for _, _, event in sorted(scheduled)))
            required = [o for o in h.operations if o.outcome == 'ok']
            optional = [o for o in h.operations if o.outcome == 'unknown']
            valid = False
            for mask in range(1 << len(optional)):
                selected = required + [o for i, o in enumerate(optional) if mask & (1 << i)]
                for order in itertools.permutations(selected):
                    positions = {o.id: i for i, o in enumerate(order)}
                    if any(a.outcome == 'ok' and a.response < b.invocation and positions[a.id] >= positions[b.id]
                           for a in selected for b in selected): continue
                    values = {}
                    for op in order:
                        key = op.args['key']
                        if op.kind == 'put': values[key] = op.args['value']
                        elif op.kind == 'delete': values.pop(key, None)
                        elif values.get(key) != op.result['value']: break
                    else:
                        valid = True
                        break
                if valid: break
            self.verdict(h, 'valid' if valid else 'invalid')

    def test_bounded_search_with_many_unknown_writes_and_late_observation(self):
        events = []
        for i in range(128):
            events += [call(i, 'put', key='04', value=f'{i:04x}'), returned(i, 'unknown')]
        for i in range(128, 256):
            events += [call(i, 'get', key='03'), returned(i, value=None)]
        events += [call(256, 'scan', start='', end='', limit=8), returned(256, rows=[['04', '007f']])]
        h = history(*events)
        result = check(h, max_states=1000)
        self.assertEqual(result['verdict'], 'valid', result)
        self.assertTrue(verify_witness(h, result['witness']))

    def test_recorder_refuses_malformed_success_and_receipts(self):
        for kind, output in [('put', ''), ('put', 'applied_term=1\napplied_index=0'),
                             ('scan', 'key_hex=61 value_hex=31\ncount=2'),
                             ('get', 'found=true'), ('create_keyspace', 'keyspace_id=3'),
                             ('delete_range', 'committed_chunks=1\nlast_applied_term=0\nlast_applied_index=0')]:
            with self.assertRaises(Malformed): parse_success(kind, output)

    def test_recorder_preserves_empty_values_and_empty_scans(self):
        self.assertEqual(parse_success('get', 'value_hex=\n')[0], {'value': ''})
        self.assertEqual(parse_success('get', 'found=false\n')[0], {'value': None})
        self.assertEqual(parse_success('scan', 'count=0\n')[0], {'rows': []})

    def verdict(self, h, wanted):
        result = check(h)
        self.assertEqual(result['verdict'], wanted, result)
        if wanted == 'valid':
            self.assertTrue(verify_witness(h, result['witness']))
        return result

    def test_acknowledged_write_then_read(self):
        self.verdict(history(call(0, 'put', key='61', value='31'), returned(0),
                             call(1, 'get', key='61'), returned(1, value='31')), 'valid')

    def test_lost_acknowledged_write(self):
        self.verdict(history(call(0, 'put', key='61', value='31'), returned(0),
                             call(1, 'get', key='61'), returned(1, value=None)), 'invalid')

    def test_unknown_write_may_commit_after_timeout(self):
        self.verdict(history(call(0, 'put', key='61', value='31'), returned(0, 'unknown'),
                             call(1, 'get', key='61'), returned(1, value=None),
                             call(2, 'get', key='61'), returned(2, value='31')), 'valid')

    def test_unknown_write_cannot_explain_read_before_invocation(self):
        self.verdict(history(call(1, 'get', key='61'), returned(1, value='31'),
                             call(0, 'put', key='61', value='31'), returned(0, 'unknown')), 'invalid')

    def test_unknown_write_may_have_no_effect(self):
        self.verdict(history(call(0, 'put', key='61', value='31'), returned(0, 'unknown'),
                             call(1, 'get', key='61'), returned(1, value=None)), 'valid')

    def test_admission_refusal_requires_exclusive_cli_evidence(self):
        for reason in ['request_count', 'encoded_bytes', 'request_too_large']:
            line = f'admission_refused=true reason={reason}\n'
            for trailer in ['', 'command terminated with exit code 1\n']:
                self.assertEqual(admission_refusal(1, '', line + trailer), reason)
            for code in [None, 0, 124, 137]:
                self.assertIsNone(admission_refusal(code, '', line))
            self.assertIsNone(admission_refusal(1, 'applied_index=7\n', line))
            for extra in ['partial_write=true\n', 'not_leader=true leader_node_id=1\n',
                          'read_unconfirmed=true phase=apply\n', line, 'transport error\n']:
                self.assertIsNone(admission_refusal(1, '', line + extra))
        for line in ['admission_refused=true reason=future\n', 'resource exhausted\n',
                     'admission_refused=false reason=request_count\n']:
            self.assertIsNone(admission_refusal(1, '', line))

    def test_guided_search_handles_refused_reads_without_values(self):
        for kind, args in [('get', {'key': '61'}), ('scan', {'start': '', 'end': '', 'limit': 8})]:
            h = history(call(0, 'put', key='61', value='31'), returned(0, 'unknown'),
                        call(1, kind, **args), returned(1, 'refused', proof='precommit'))
            try:
                result = search(h, guided_unknown=True)
            except KeyError as error:
                self.fail(f'refused reads have no observed value: {error}')
            self.assertEqual(result['verdict'], 'valid')
            self.assertTrue(verify_witness(h, result['witness']))

    def test_recent_unknown_write_avoids_old_range_frontier_explosion(self):
        events = []
        for i in range(64):
            events.extend([call(i, 'delete_range', start='61', end='63'), returned(i, 'unknown')])
        events.extend([call(64, 'delete', key='61'), returned(64, 'unknown'),
                       call(65, 'get', key='61'), returned(65, value=None),
                       call(66, 'get', key='62'), returned(66, value='32')])
        h = history(*events, initial=header(kv=[('61', '31'), ('62', '32')]))
        result = search(h, max_states=100, guided_unknown=True)
        self.assertEqual(result['verdict'], 'valid', 'old unresolved ranges exhausted a small known witness')
        self.assertTrue(verify_witness(h, result['witness']))
        # This checks witness ordering under a fixed resource budget. It cannot
        # classify a restricted or budget-exhausted search as invalid.

    def test_guided_search_orders_matching_overlapping_read_before_write(self):
        events = []
        for i in range(64):
            events.extend([call(i, 'delete_range', start='61', end='63'), returned(i, 'unknown')])
        events.extend([call(64, 'scan', start='61', end='62', limit=8),
                       call(65, 'put', key='61', value='31'), returned(65), returned(64, rows=[]),
                       call(66, 'get', key='61'), returned(66, value='31'),
                       call(67, 'get', key='62'), returned(67, value='32')])
        h = history(*events, initial=header(kv=[('62', '32')]))
        result = search(h, max_states=100, guided_unknown=True)
        self.assertEqual(result['verdict'], 'valid', 'overlapping read ordering hid a small known witness')
        self.assertTrue(verify_witness(h, result['witness']))

    def test_proven_refusal_cannot_create_a_value(self):
        self.verdict(history(call(0, 'put', key='61', value='31'), returned(0, 'refused', proof='precommit'),
                             call(1, 'get', key='61'), returned(1, value='31')), 'invalid')

    def test_completed_write_cannot_reappear_after_delete(self):
        self.verdict(history(call(0, 'put', key='61', value='31'), returned(0),
                             call(1, 'delete', key='61'), returned(1),
                             call(2, 'get', key='61'), returned(2, value='31')), 'invalid')

    def test_overlapping_reads_may_complete_out_of_order(self):
        self.verdict(history(call(0, 'get', key='61'), call(1, 'put', key='61', value='31'),
                             returned(1), call(2, 'get', key='61'), returned(2, value='31'),
                             returned(0, value=None)), 'valid')

    def test_cross_key_scan_is_one_atomic_observation(self):
        self.verdict(history(call(0, 'scan', start='', end='', limit=2),
                             call(1, 'put', key='61', value='31'), returned(1),
                             call(2, 'put', key='62', value='31'), returned(2),
                             returned(0, rows=[['61', '30'], ['62', '31']]),
                             initial=header([('61', '30'), ('62', '30')])), 'invalid')

    def test_scan_bounds_limit_and_empty_bytes(self):
        self.verdict(history(call(0, 'scan', start='61', end='63', limit=1),
                             returned(0, rows=[['61', '']]),
                             initial=header([('', '30'), ('61', ''), ('62', '31')])), 'valid')
        self.verdict(history(call(0, 'scan', start='', end='', limit=0), returned(0, rows=[])), 'valid')

    def test_duplicate_name_and_id_are_rejected(self):
        for name, kid in [('same', 3), ('other', 2)]:
            self.verdict(history(call(0, 'create_keyspace', name='same'), returned(0, id=2),
                                 call(1, 'create_keyspace', name=name), returned(1, id=kid)), 'invalid')

    def test_create_can_finish_out_of_order(self):
        self.verdict(history(call(0, 'create_keyspace', name='a'), call(1, 'create_keyspace', name='b'),
                             returned(1, id=3), returned(0, id=2)), 'valid')

    def test_unknown_create_can_explain_later_space(self):
        self.verdict(history(call(0, 'create_keyspace', name='new'), returned(0, 'unknown'),
                             call(1, 'put', keyspace=9, key='61', value='31'), returned(1)), 'valid')

    def test_range_selection_and_chunk_are_distinct(self):
        self.verdict(history(call(0, 'delete_range', start='', end=''),
                             call(1, 'put', key='62', value='31'), returned(1),
                             call(2, 'get', key='61'), returned(2, value='30'),
                             returned(0, committed_chunks=1),
                             call(3, 'scan', start='', end='', limit=10), returned(3, rows=[['62', '31']]),
                             initial=header([('61', '30')])), 'valid')

    def test_range_uses_one_snapshot_across_chunks(self):
        # New b must not enter the second chunk of the already selected {a,c}.
        events = [call(0, 'delete_range', start='', end=''), call(1, 'get', key='61'), returned(1, value=None),
                  call(2, 'put', key='62', value='31'), returned(2), returned(0, committed_chunks=2),
                  call(3, 'get', key='62')]
        for value, verdict in [('31', 'valid'), (None, 'invalid')]:
            self.verdict(history(*events, returned(3, value=value), initial=header([('61', '30'), ('63', '30')], chunk=1)), verdict)

    def test_range_receipt_cannot_omit_an_acknowledged_key(self):
        self.verdict(history(call(0, 'delete_range', start='', end=''), returned(0, committed_chunks=1),
                             call(1, 'get', key='61'), returned(1, value='30'),
                             initial=header([('61', '30')])), 'invalid')

    def test_partial_range_preserves_proven_prefix_and_allows_one_unknown_chunk(self):
        events = [call(0, 'delete_range', start='', end=''), returned(0, 'unknown', committed_chunks=1),
                  call(1, 'scan', start='', end='', limit=10)]
        for rows, verdict in [([['62', '30'], ['63', '30']], 'valid'), ([['63', '30']], 'valid'), ([], 'invalid'),
                              ([['61', '30'], ['62', '30'], ['63', '30']], 'invalid')]:
            self.verdict(history(*events, returned(1, rows=rows), initial=header([('61', '30'), ('62', '30'), ('63', '30')], chunk=1)), verdict)

    def test_missing_response_is_pending(self):
        self.verdict(history(call(0, 'put', key='61', value='31'), call(1, 'get', key='61'),
                             returned(1, value='31')), 'valid')

    def test_invalid_scan_order_and_duplicate_rows(self):
        for rows in [[['62', '30'], ['61', '30']], [['61', '30'], ['61', '30']]]:
            self.verdict(history(call(0, 'scan', start='', end='', limit=10), returned(0, rows=rows),
                                 initial=header([('61', '30'), ('62', '30')])), 'invalid')

    def test_budget_exhaustion_is_inconclusive(self):
        h = history(call(0, 'get', key='61'), returned(0, value=None))
        self.assertEqual(check(h, max_states=0)['verdict'], 'inconclusive')
        self.assertEqual(check(h, seconds=0)['verdict'], 'inconclusive')

    def test_witness_replay_refuses_fabricated_or_missing_steps(self):
        h = history(call(0, 'put', key='61', value='31'), returned(0), call(1, 'get', key='61'), returned(1, value='31'))
        witness = self.verdict(h, 'valid')['witness']
        with self.assertRaises(Malformed): verify_witness(h, witness[1:])
        bad = copy.deepcopy(witness); bad[0]['before_event'] = 0
        with self.assertRaises(Malformed): verify_witness(h, bad)

    def test_minimized_prefix_keeps_causal_writes(self):
        h = history(call(0, 'put', key='61', value='31'), returned(0), call(1, 'get', key='61'), returned(1, value=None),
                    call(2, 'put', key='62', value='32'), returned(2))
        minimized = History.parse(minimize_prefix(h))
        self.assertEqual(len(minimized.events), 4)
        self.verdict(minimized, 'invalid')

    def test_malformed_records_and_empty_histories_are_not_valid(self):
        for records in [[header()], [header(), {'type': 'return', 'seq': 0, 'id': 1, 'outcome': 'ok', 'result': {}}]]:
            with self.assertRaises(Malformed): History.parse(records)
        h = history(call(0, 'get', key='61'), returned(0, value=None))
        for change in ['duplicate', 'bad_hex', 'bad_sequence', 'bad_outcome']:
            events = copy.deepcopy(h.events)
            if change == 'duplicate': events.append({**events[1], 'seq': 2})
            if change == 'bad_hex': events[0]['args']['key'] = 'GG'
            if change == 'bad_sequence': events[1]['seq'] = 0
            if change == 'bad_outcome': events[1]['outcome'] = 'failed'
            with self.assertRaises(Malformed): History.parse([h.header, *events])

    def test_malformed_nested_fields_are_explicitly_rejected(self):
        base = [header(), {**call(0, 'get', key='61'), 'seq': 0}, {**returned(0, value=None), 'seq': 1}]
        cases = [[None, *base[1:]]]
        for field, value in [('initial', None), ('initial', {'keyspaces': [None]}),
                             ('initial', {'kv': [None]}), ('initial', {'keyspaces': {}}), ('version', True)]:
            cases.append([{**base[0], field: value}, *base[1:]])
        for field, value in [('op', []), ('args', None), ('phase', None)]:
            cases.append([base[0], {**base[1], field: value}, base[2]])
        for field, value in [('outcome', []), ('observation', None), ('observation', {'malformed': 'bad payload'})]:
            cases.append([*base[:2], {**base[2], field: value}])
        for case in cases:
            with self.subTest(case=case), self.assertRaises(Malformed): History.parse(case)

    def test_cli_requires_successful_put_and_read_in_each_fault_phase(self):
        h = history(call(0, 'put', key='61', value='31'), returned(0),
                    call(1, 'get', key='61'), returned(1, value='31'))
        with tempfile.TemporaryDirectory() as d:
            path, out = Path(d) / 'trace.jsonl', Path(d) / 'result.json'
            for phases, expected in [(['healing', 'healing'], 2), (['partition', 'healing'], 2),
                                     (['partition', 'partition'], 0)]:
                events = copy.deepcopy(h.events)
                events[0]['phase'], events[2]['phase'] = phases
                path.write_text(''.join(json.dumps(e)+'\n' for e in [h.header, *events]))
                run = subprocess.run([sys.executable, str(Path(__file__).with_name('checker.py')), str(path),
                                      '--output', str(out), '--acceptance', '--require-phase', 'partition'], capture_output=True)
                self.assertEqual(run.returncode, expected, run.stderr)
                result = json.loads(out.read_text())
                if expected: self.assertEqual(result['reason'], 'insufficient_fault_phase_coverage')

    def test_cli_cannot_accept_empty_all_error_or_missing_coverage(self):
        traces = [[], [header()], [header(), {**call(0, 'put', key='61', value='31'), 'seq': 0},
                                   {**returned(0, 'unknown'), 'seq': 1}],
                  [header(), {**call(0, 'get', key='61'), 'seq': 0}, {**returned(0, value=None), 'seq': 1}]]
        with tempfile.TemporaryDirectory() as d:
            for trace in traces:
                p = Path(d) / 'trace.jsonl'; out = Path(d) / 'result.json'
                p.write_text(''.join(json.dumps(e)+'\n' for e in trace))
                run = subprocess.run([sys.executable, str(Path(__file__).with_name('checker.py')), str(p), '--output', str(out), '--acceptance'], capture_output=True)
                self.assertEqual(run.returncode, 2, run.stderr)
                self.assertEqual(json.loads(out.read_text())['verdict'], 'inconclusive')


if __name__ == '__main__':
    unittest.main()
