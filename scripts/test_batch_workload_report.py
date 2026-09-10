#!/usr/bin/env python3
"""Synthetic schema/corruption controls, never executable or fault acceptance.

The tiny retained "binary" and Cargo rows below are explicitly synthetic. They
exercise the validator's byte/shape bindings but cannot attest compiler output,
executing PIDs, server receipts, storage durability, or injected fault effects.
Set KV9_BATCH_CONTROL_ARTIFACTS to retain every synthetic control directory.
"""

import copy
from collections import Counter
import hashlib
import itertools
import json
import os
from pathlib import Path
import re
import shutil
import tempfile
import unittest

import batch_workload_report as validator


OUTCOMES = ['success', 'error', 'aborted', 'released', 'replaced', 'rejected', 'unconfirmed']
REVISION = '1' * 40
NEXT_CONTROL = itertools.count()


def encoded(value):
    return (json.dumps(value, sort_keys=True, separators=(',', ':')) + '\n').encode()


def digest(data):
    return hashlib.sha256(data).hexdigest()


def make_histogram(samples, outcome):
    # Compute the expected recorder histogram independently of validator helpers.
    buckets = [0] * 65
    for sample in samples:
        buckets[sample.bit_length()] += 1
    result = dict(outcome=outcome, buckets=buckets, count=len(samples), sum_ns=sum(samples),
                  min_ns=min(samples) if samples else None, max_ns=max(samples) if samples else None,
                  count_saturated=False, sum_saturated=False, duration_clamped=False)
    for percentile in (50, 95, 99):
        bound = None
        if samples:
            observed = sorted(samples)[(len(samples) * percentile + 99) // 100 - 1]
            index = observed.bit_length()
            bound = dict(lower_ns=0 if index == 0 else 2 ** (index - 1), upper_ns=2 ** index - 1)
        result[f'p{percentile}'] = bound
    return result


def make_metrics(records):
    calls, rows, samples = {}, {}, {}
    for event in records[1:]:
        if event['type'] == 'invoke':
            calls[event['id']] = event
            continue
        call = calls[event['id']]
        identity = call['phase'], call['op']
        args, operation = call['args'], call['op']
        count = len(args['keys']) if operation == 'batch_get' else len(args['pairs']) if operation == 'batch_put' else 1
        row = rows.setdefault(identity, dict(phase=identity[0], operation=operation, calls=0,
                                            input_items=0, successful_items=0, unknown_write_items=0,
                                            refused_items=0, attempts=0, reasons=Counter()))
        observed = event['observation']
        reason = observed['reason']
        row['calls'] += 1
        row['input_items'] += count
        row['attempts'] += len(observed['attempts'])
        row['reasons']['success' if reason is None else reason['kind']] += 1
        read = operation in ('get', 'batch_get')
        if event['outcome'] == 'ok':
            row['successful_items'] += count
            group = 0
        elif event['outcome'] == 'refused':
            row['refused_items'] += count
            group = 3 if not observed['attempts'] else 5
        else:
            if not read:
                row['unknown_write_items'] += count
            group = 1 if read else 6
        populations = samples.setdefault(identity, {level: [[] for _ in OUTCOMES] for level in ('logical', 'attempt')})
        populations['logical'][group].append(observed['elapsed_ns'])
        for attempt in observed['attempts']:
            reason = attempt['failure']
            group = 0 if reason is None else 5 if reason['kind'] in (
                'not_leader', 'admission_count', 'admission_bytes', 'admission_oversize') else 1 if read else 6
            populations['attempt'][group].append(attempt['elapsed_ns'])
    for identity, row in rows.items():
        for level in ('logical', 'attempt'):
            row[level + '_latency'] = dict(valid=True, outcomes=[
                make_histogram(samples[identity][level][i], outcome) for i, outcome in enumerate(OUTCOMES)])
    return list(rows.values())


class SyntheticArtifacts:
    def __init__(self, root):
        self.root = root
        self.run, self.retained = root / 'run', root / 'build'
        self.run.mkdir(); self.retained.mkdir()
        (root / 'SYNTHETIC.txt').write_text('Schema controls only. No server, client, compiler, or fault was executed.\n')
        self.config = dict(version=1, client=dict(version=1,
            peers=[dict(node_id=1, address='127.0.0.1:19001'), dict(node_id=2, address='127.0.0.1:19002')],
            keyspace_id=1, epoch_conf_ver=1, epoch_version=1, max_in_flight=2, max_attempts=3,
            deadline_ms=1500, retry_backoff_ms=1), rpc_transport='tonic_stream', run_id='synthetic',
            keyspace_name='synthetic', workers=1, seed=40, keys=1, batch_size=2, value_bytes=16,
            mix=[0, 0, 0, 50, 50], max_calls=10, history_bytes=1024*1024,
            measure_ms=1000, interval_ms=0)
        binary = b'SYNTHETIC CONTROL BYTES: NOT AN EXECUTABLE\n'
        (self.retained / 'kv9-batch-workload').write_bytes(binary)
        sources = {'synthetic-input.txt': digest(b'synthetic source only')}
        tree = digest(json.dumps(sources, sort_keys=True, separators=(',', ':')).encode())
        self.build = dict(version=1, revision=REVISION, dirty=False, source_tree_sha256=tree,
                          binary_sha256=digest(binary), profile='debug', rustc='synthetic compiler identity')
        self.inventory = dict(version=1, revision=REVISION, dirty=False, sources=sources,
            command=['cargo', 'build', '--locked', '-p', 'kv9-server', '--bin', 'kv9-batch-workload', '--message-format=json-render-diagnostics'],
            build_environment={}, source_tree_sha256=tree, binary_sha256=digest(binary))
        self.cargo = [dict(reason='compiler-artifact', target=dict(name=name, kind=['bin' if name == 'kv9-batch-workload' else 'lib']),
                           features=[], executable='/synthetic/kv9-batch-workload' if name == 'kv9-batch-workload' else None)
                      for name in ('kv9-batch-workload', 'kv9_server', 'kv9_engine', 'kv9_raft')]
        key = lambda i: f'synthetic:{i:016x}'.encode().hex()
        value = lambda nonce, item: (nonce.to_bytes(8, 'big') + item.to_bytes(8, 'big')).hex()
        data_key, sentinel = key(0), key(1)
        self.records = [dict(type='header', version=2, range_chunk_size=1024,
            generator='kv9-native-batch-workload', configuration=copy.deepcopy(self.config),
            initial=dict(keyspaces=[dict(name='synthetic', id=1)], kv=[]))]

        def call(phase, nonce, operation, args, result, start):
            oid = (len(self.records) - 1) // 2
            self.records.append(dict(type='invoke', seq=oid*2, monotonic_ns=start, id=oid,
                                     client='0', phase=phase, nonce=nonce, op=operation, args=dict(keyspace=1, **args)))
            receipt = None if operation == 'batch_get' else dict(applied_term=7, applied_index=oid+1)
            observation = dict(attempts=[dict(ordinal=1, node_id=1, elapsed_ns=1, failure=None)],
                               elapsed_ns=1, stop='terminal', reason=None, receipt=receipt, malformed=None)
            self.records.append(dict(type='return', seq=oid*2+1, monotonic_ns=start+2, id=oid,
                                     outcome='ok', result=result, observation=observation))
        call('initialization', None, 'batch_get', dict(keys=[data_key, sentinel]), dict(values=[None, None]), 200)
        call('initialization', None, 'batch_put', dict(pairs=[[data_key, value(0, 0)], [sentinel, '']]), {}, 300)
        current = value(0, 0)
        # Independently specified first seven operations for seed 40/keys 1.
        # Duplicate batch keys deliberately test last-pair-wins/item accounting.
        for nonce, operation in enumerate(['batch_put', 'batch_get', 'batch_get', 'batch_put',
                                            'batch_put', 'batch_get', 'batch_put']):
            if operation == 'batch_put':
                args, result = dict(pairs=[[data_key, value(nonce+1, 0)], [data_key, value(nonce+1, 1)]]), {}
                current = value(nonce+1, 1)
            else:
                args, result = dict(keys=[data_key, data_key]), dict(values=[current, current])
            call('measure', nonce, operation, args, result, 1100+nonce*10)
        call('verify', None, 'batch_get', dict(keys=[data_key, sentinel]), dict(values=[current, '']), 2200)
        self.report = dict(version=1, complete=True, failure=None, configuration=copy.deepcopy(self.config),
            config_sha256='', build=copy.deepcopy(self.build), build_sha256='', history_sha256='',
            process_id=12345, wall_anchor_unix_ns=1700000000000000000, wall_anchor_monotonic_ns=10,
            elapsed_ns=2400, runtime_threads=2, workload_model='closed_loop_correctness', independently_checked=False,
            stages=dict(initialization=dict(start_ns=100,end_ns=500),
                        measurement_and_drain=dict(start_ns=1000,end_ns=2000),
                        verification=dict(start_ns=2100,end_ns=2300)),
            stop=dict(reason='operation_limit',monotonic_ns=2000),
            history=dict(issued=10, terminal=10, events=20, bytes=0, peak_in_flight=1,
                         accounting_complete=True, failure=None, metrics=make_metrics(self.records)))
        self.refresh_metrics = False
        self.final_newline = True

    def save(self):
        config_data, build_data = encoded(self.config), encoded(self.build)
        data = b''.join(encoded(record) for record in self.records)
        if not self.final_newline:
            data = data[:-1]
        self.report.update(config_sha256=digest(config_data), build_sha256=digest(build_data), history_sha256=digest(data))
        self.report['history']['bytes'] = len(data)
        if self.refresh_metrics:
            self.report['history']['metrics'] = make_metrics(self.records)
        (self.run/'config.json').write_bytes(config_data)
        (self.run/'build.json').write_bytes(build_data)
        (self.retained/'build.json').write_bytes(build_data)
        (self.retained/'sources.json').write_bytes(encoded(self.inventory))
        (self.retained/'cargo.jsonl').write_bytes(b''.join(encoded(row) for row in self.cargo))
        (self.run/'history.jsonl').write_bytes(data)
        (self.run/'report.json').write_bytes(encoded(self.report))

    def terminal(self, oid):
        return next(record for record in self.records[1:] if record['type'] == 'return' and record['id'] == oid)

    def row(self, phase='initialization', operation='batch_put'):
        return next(row for row in self.report['history']['metrics'] if (row['phase'], row['operation']) == (phase, operation))


class BatchReportControls(unittest.TestCase):
    def fixture(self, label):
        retained = os.environ.get('KV9_BATCH_CONTROL_ARTIFACTS')
        if retained:
            parent = Path(retained); parent.mkdir(parents=True, exist_ok=True)
            root = parent / f'{next(NEXT_CONTROL):03d}-{label}'
            root.mkdir()
        else:
            root = Path(tempfile.mkdtemp(prefix='kv9-synthetic-batch-control-'))
            self.addCleanup(shutil.rmtree, root)
        fixture = SyntheticArtifacts(root)
        fixture.save()
        return fixture

    def accepted(self, fixture):
        result = validator.validate(fixture.run, fixture.retained, REVISION, seconds=2)
        self.assertTrue(result['accepted'])
        self.assertTrue(result['full_history_independently_checked'])
        return result

    def reject(self, label, mutate, message):
        fixture = self.fixture(label)
        self.accepted(fixture)
        mutate(fixture)
        fixture.save()
        try:
            validator.validate(fixture.run, fixture.retained, REVISION, seconds=2)
        except ValueError as error:
            (fixture.root/'control-result.json').write_bytes(encoded(dict(synthetic=True, rejected=True, reason=str(error), expected=message)))
            self.assertRegex(str(error), re.escape(message))
        else:
            (fixture.root/'control-result.json').write_bytes(encoded(dict(synthetic=True, rejected=False, expected=message)))
            self.fail(f'{label}: corrupted synthetic artifacts were accepted')

    def test_synthetic_baseline_has_complete_atomic_history_and_whole_batch_units(self):
        fixture = self.fixture('baseline')
        result = self.accepted(fixture)
        self.assertEqual(result['traffic_successes'], dict(batch_get=3, batch_put=4))
        self.assertEqual(result['history']['coverage']['operations'], 10)
        row = fixture.row('measure', 'batch_put')
        self.assertEqual((row['calls'], row['input_items'], row['successful_items']), (4, 8, 8))
        self.assertEqual(row['logical_latency']['outcomes'][0]['count'], 4)

    def test_bool_and_float_substitutions_in_copied_config_and_build_are_rejected(self):
        cases = [
            ('report-config-version', lambda f: f.report['configuration'].__setitem__('version', True)),
            ('report-config-epoch', lambda f: f.report['configuration']['client'].__setitem__('epoch_version', 1.0)),
            ('report-config-workers', lambda f: f.report['configuration'].__setitem__('workers', True)),
            ('report-build-version', lambda f: f.report['build'].__setitem__('version', True)),
            ('report-build-dirty', lambda f: f.report['build'].__setitem__('dirty', 0)),
        ]
        for label, change in cases:
            with self.subTest(label=label):
                self.reject(label, change, 'native report config/build identity differs')
        self.reject('input-config-bool', lambda f: f.config.__setitem__('workers', True), 'invalid bounded integer')
        self.reject('build-bool-version', lambda f: f.build.__setitem__('version', True), 'invalid bounded integer')
        self.reject('build-integer-dirty', lambda f: f.build.__setitem__('dirty', 0), 'invalid build identity')

    def test_header_copies_cannot_hide_type_substitutions(self):
        for label, change in [
            ('header-float-version', lambda f: f.records[0].__setitem__('version', 2.0)),
            ('header-bool-config', lambda f: f.records[0]['configuration'].__setitem__('version', True)),
            ('header-bool-keyspace', lambda f: f.records[0]['initial']['keyspaces'][0].__setitem__('id', True)),
            ('header-float-chunk', lambda f: f.records[0].__setitem__('range_chunk_size', 1024.0)),
        ]:
            with self.subTest(label=label):
                self.reject(label, change, 'native header differs')

    def test_source_inventory_and_feature_identity_controls(self):
        for label, change, message in [
            ('source-dirty-integer', lambda f: f.inventory.__setitem__('dirty', 0), 'source inventory dirty is not boolean'),
            ('source-version-bool', lambda f: f.inventory.__setitem__('version', True), 'invalid bounded integer'),
            ('source-bad-digest', lambda f: f.inventory['sources'].__setitem__('synthetic-input.txt', False), 'invalid source inventory digest'),
            ('source-clean-deletion', lambda f: f.inventory['sources'].__setitem__('synthetic-input.txt', None), 'invalid source inventory digest'),
            ('source-false-hash', lambda f: f.inventory['sources'].__setitem__('synthetic-input.txt', '0'*64), 'source inventory hash differs'),
            ('truncated-bin-flag', lambda f: f.inventory.__setitem__('command', ['cargo', 'build', '--bin']), 'wrong workload build command'),
            ('nonobject-cargo', lambda f: f.cargo.append(True), 'invalid Cargo record shape'),
            ('ambiguous-cargo', lambda f: f.cargo.append(copy.deepcopy(f.cargo[0])), 'missing or ambiguous native build artifact'),
            ('testing-engine', lambda f: f.cargo[2].__setitem__('features', ['testing']), 'unexpected runtime build features'),
            ('mismatched-server-features', lambda f: f.cargo[1].__setitem__('features', ['rpc-experiment']), 'native server/client feature graphs differ'),
            ('empty-executable', lambda f: f.cargo[0].__setitem__('executable', ''), 'native artifact is not executable'),
            ('string-executable-kind', lambda f: f.cargo[0]['target'].__setitem__('kind', 'bin'), 'native artifact is not executable'),
            ('wrong-retained-binary', lambda f: (f.retained/'kv9-batch-workload').write_bytes(b'changed synthetic bytes'), 'retained executable differs'),
        ]:
            with self.subTest(label=label):
                self.reject(label, change, message)

    def test_metric_reason_counts_require_positive_integers_and_exact_populations(self):
        for label, change, message in [
            ('bool-reason-count', lambda f: f.row()['reasons'].__setitem__('success', True), 'invalid bounded integer'),
            ('float-reason-count', lambda f: f.row()['reasons'].__setitem__('success', 1.0), 'invalid bounded integer'),
            ('zero-extra-reason', lambda f: f.row()['reasons'].__setitem__('deadline', 0), 'invalid bounded integer'),
            ('reason-over-call-count', lambda f: f.row()['reasons'].__setitem__('success', 2), 'invalid bounded integer'),
            ('wrong-reason-population', lambda f: f.row().__setitem__('reasons', {'deadline': 1}), 'native RPC/item/reason counts differ'),
            ('collapsed-duplicate-items', lambda f: f.row('measure','batch_put').__setitem__('input_items', 4), 'native RPC/item/reason counts differ'),
        ]:
            with self.subTest(label=label):
                self.reject(label, change, message)

    def test_attempt_ordinals_and_node_identities_are_strict_integers(self):
        for field in ('ordinal', 'node_id'):
            for replacement in (True, 1.0):
                with self.subTest(field=field, replacement=replacement):
                    self.reject(f'attempt-{field}-{type(replacement).__name__}',
                        lambda f, field=field, replacement=replacement: f.terminal(2)['observation']['attempts'][0].__setitem__(field, replacement),
                        'invalid bounded integer')
        self.reject('unconfigured-node', lambda f: f.terminal(2)['observation']['attempts'][0].__setitem__('node_id', 99), 'native attempt identity mismatch')
        self.reject('skipped-ordinal', lambda f: f.terminal(2)['observation']['attempts'][0].__setitem__('ordinal', 2), 'native attempt identity mismatch')

    @staticmethod
    def redirect(fixture):
        observed = fixture.terminal(9)['observation']
        observed['attempts'] = [dict(ordinal=1,node_id=1,elapsed_ns=0,failure=dict(kind='not_leader',leader=2)),
                                dict(ordinal=2,node_id=2,elapsed_ns=1,failure=None)]
        fixture.refresh_metrics = True

    def test_safe_redirects_are_accepted_but_unknown_retries_and_wrong_targets_are_rejected(self):
        fixture = self.fixture('safe-redirect')
        self.redirect(fixture); fixture.save(); self.accepted(fixture)
        def unknown_retry(f):
            self.redirect(f)
            f.terminal(9)['observation']['attempts'][0]['failure'] = dict(kind='rpc_status',code=14)
        self.reject('unknown-replay', unknown_retry, 'native retry lacks exclusive NotLeader refusal')
        def wrong_target(f):
            self.redirect(f)
            f.terminal(9)['observation']['attempts'][1]['node_id'] = 1
        self.reject('wrong-redirect-peer', wrong_target, 'native redirect does not follow its fixed peer mapping')

    def test_histogram_observer_percentiles_and_whole_call_samples_are_strict(self):
        for label, change, message in [
            ('histogram-invalid', lambda f: f.row()['logical_latency'].__setitem__('valid', False), 'invalid native histogram observer'),
            ('histogram-integer-valid', lambda f: f.row()['logical_latency'].__setitem__('valid', 1), 'invalid native histogram observer'),
            ('histogram-bool-count', lambda f: f.row()['logical_latency']['outcomes'][0].__setitem__('count', True), 'invalid bounded integer'),
            ('histogram-bool-bound', lambda f: f.row()['logical_latency']['outcomes'][0]['p50'].__setitem__('lower_ns', True), 'invalid bounded integer'),
            ('histogram-float-bound', lambda f: f.row()['logical_latency']['outcomes'][0]['p99'].__setitem__('upper_ns', 1.0), 'invalid bounded integer'),
            ('histogram-saturation', lambda f: f.row()['logical_latency']['outcomes'][0].__setitem__('sum_saturated', True), 'saturated/clamped histogram'),
            ('one-sample-per-item', lambda f: f.row()['logical_latency']['outcomes'].__setitem__(0, make_histogram([1,1], 'success')), 'native histogram is not one sample per whole call/attempt'),
            ('reordered-histogram', lambda f: f.row()['logical_latency']['outcomes'][0].__setitem__('outcome', 'error'), 'native histogram outcome reordered'),
        ]:
            with self.subTest(label=label):
                self.reject(label, change, message)

    def test_stop_and_event_time_fields_are_bounded_without_rewriting_combined_drain_semantics(self):
        for label, change, message in [
            ('float-stop-time', lambda f: f.report['stop'].__setitem__('monotonic_ns', 2000.0), 'invalid bounded integer'),
            ('boolean-stop-time', lambda f: f.report['stop'].__setitem__('monotonic_ns', True), 'invalid bounded integer'),
            ('stop-outside-run', lambda f: f.report['stop'].__setitem__('monotonic_ns', 2401), 'invalid bounded integer'),
            ('moved-stop-boundary', lambda f: f.report['stop'].__setitem__('monotonic_ns', 1999), 'native stop/drain boundary differs'),
            ('backward-event-time', lambda f: f.terminal(2).__setitem__('monotonic_ns', 0), 'invalid bounded integer'),
            ('latency-outside-interval', lambda f: f.terminal(2)['observation'].__setitem__('elapsed_ns', 3), 'invalid bounded integer'),
        ]:
            with self.subTest(label=label):
                self.reject(label, change, message)
        for reason in ('stop_file', 'duration'):
            fixture = self.fixture('recorded-stop-' + reason)
            fixture.report['stop']['reason'] = reason
            fixture.save(); self.accepted(fixture)

    def test_unknown_batch_keeps_whole_item_uncertainty_and_cannot_fabricate_a_receipt(self):
        def unknown(f):
            response = f.terminal(2)
            response['outcome'] = 'unknown'
            response['observation'].update(reason=dict(kind='rpc_status',code=14), receipt=None)
            response['observation']['attempts'][0]['failure'] = dict(kind='rpc_status',code=14)
            f.refresh_metrics = True
        fixture = self.fixture('unknown-whole-batch')
        unknown(fixture); fixture.save(); self.accepted(fixture)
        row = fixture.row('measure','batch_put')
        self.assertEqual((row['unknown_write_items'],row['successful_items']), (2,6))
        def fabricate(f):
            unknown(f)
            f.terminal(2)['observation']['receipt'] = dict(applied_term=7,applied_index=3)
        self.reject('receipt-on-unknown', fabricate, 'unexpected batch write receipt')
        def wrong_final(f):
            unknown(f)
            f.terminal(2)['observation']['attempts'][0]['failure']['code'] = 4
        self.reject('mismatched-final-attempt', wrong_final, 'native final attempt differs from terminal')

    def test_incomplete_history_and_traffic_invariants_are_not_loosened(self):
        self.reject('missing-final-newline', lambda f: setattr(f, 'final_newline', False), 'complete native history lacks its final newline')
        self.reject('missing-terminal', lambda f: f.records.pop(), 'native complete-history ledger differs')
        self.reject('duplicate-nonce', lambda f: f.records[7].__setitem__('nonce', 0), 'duplicate nonce or altered generated batch')
        self.reject('changed-generated-value', lambda f: f.records[5]['args']['pairs'][0].__setitem__(1, '00'*16), 'duplicate nonce or altered generated batch')
        self.reject('partial-read-vector', lambda f: f.terminal(3)['result']['values'].pop(), 'batch read result cardinality')
        self.reject('changed-empty-sentinel', lambda f: f.terminal(9)['result']['values'].__setitem__(1, None), 'native empty sentinel changed')

    def test_invalid_search_budgets_are_rejected_before_search(self):
        fixture = self.fixture('search-budget')
        for seconds in (True, 0, -1, float('nan'), float('inf')):
            with self.subTest(seconds=seconds):
                with self.assertRaisesRegex(ValueError, 'invalid native checker time budget'):
                    validator.validate(fixture.run, fixture.retained, seconds=seconds)


if __name__ == '__main__':
    unittest.main()
