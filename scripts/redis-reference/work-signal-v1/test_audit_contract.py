#!/usr/bin/env python3
"""Finite pure-function controls; never invoke the auditor or matched driver."""
import ast
import copy
import json
from pathlib import Path
import unittest

SOURCE = Path(__file__).with_name('audit.py')
tree = ast.parse(SOURCE.read_text(), filename=str(SOURCE))
constants = {'CLIENT_REV', 'OLD_REV', 'NEW_REV', 'REDIS_REV', 'PINS',
             'PROTOCOL_ID', 'CONCURRENCIES', 'ARMS', 'ROLE_PATHS', 'SERVER_MANIFESTS'}
functions = {'require', 'same_json', 'expected_cohorts', 'shared_config',
             'expected_descriptor', 'validate_matrix_protocol',
             'validate_requested_config', 'paired_cases', 'histogram'}
nodes = [node for node in tree.body
         if (isinstance(node, ast.FunctionDef) and node.name in functions)
         or (isinstance(node, ast.Assign) and len(node.targets) == 1
             and isinstance(node.targets[0], ast.Name) and node.targets[0].id in constants)]
audit = {'json': json}
exec(compile(ast.Module(body=nodes, type_ignores=[]), str(SOURCE), 'exec'), audit)

# This independent fixture does not call expected_cohorts/shared_config or import
# the driver to manufacture the values it expects the auditor to accept.
ARM_ROWS = [('old-point', 'old', 'point_get'), ('old-batch1', 'old', 'batch_get'),
            ('new-point', 'new', 'point_get'), ('new-batch1', 'new', 'batch_get'),
            ('redis-mget1', None, 'mget'), ('redis-get1', None, 'get')]


def config(repeat, workers, server, api):
    c = dict(run_id=f'p{repeat}{workers:04d}', seed=71, workers=workers,
             keys=4096, batch_size=1, value_bytes=128, read_percent=100,
             warmup_calls=128, measure_ms=10000, max_calls=10_000_000,
             load={'kind': 'closed_loop'})
    if server:
        c.update(version=2, rpc_transport='tonic_stream', read_api=api,
                 client=dict(version=1, peers=[{'node_id': i, 'address': f'127.0.0.1:{20000+i}'}
                                               for i in (1, 2, 3)],
                             keyspace_id=100, epoch_conf_ver=1, epoch_version=1,
                             max_in_flight=workers, max_attempts=6,
                             deadline_ms=1500, retry_backoff_ms=5))
    else:
        c.update(version=2, read_api=api, address='127.0.0.1:25000', deadline_ms=1500)
    return c


def matrix():
    rows = []
    for repeat, workers, indices in [(0, 1, range(6)), (0, 64, range(6)),
                                     (1, 64, range(5, -1, -1)), (1, 1, range(5, -1, -1)),
                                     (2, 64, range(5, -1, -1)), (2, 1, range(5, -1, -1)),
                                     (3, 1, range(6)), (3, 64, range(6))]:
        for i in indices:
            arm, server, api = ARM_ROWS[i]
            full = config(repeat, workers, server, api)
            shared = {k: v for k, v in full.items()
                      if k not in {'version', 'rpc_transport', 'read_api', 'client', 'address', 'deadline_ms'}}
            rows.append(dict(repeat=repeat, arm=arm, server_role=server, read_api=api,
                             mix='read', target='kv9' if server else 'redis-memory', shared=shared))
    return dict(version=2, protocol_id='kv9-work-signal-c1-c64-v1',
                concurrency_points=[1, 64], smoke=False, fixed_rates=None,
                attempts=[{} for _ in rows], cohort_inventory=rows)


class AuditContract(unittest.TestCase):
    def test_syntax_and_no_runtime_import_for_controls(self):
        compile(SOURCE.read_text(), str(SOURCE), 'exec')
        self.assertEqual(set(audit) & {'ROOT', 'OUT', 'args', 'result'}, set())

    def test_exact_48_order_and_run_ids(self):
        m = matrix()
        audit['validate_matrix_protocol'](m)
        expected = [(d['repeat'], d['shared']['workers'], d['arm'], d['server_role'], d['read_api'])
                    for d in m['cohort_inventory']]
        self.assertEqual(audit['expected_cohorts'](), expected)
        self.assertEqual(len(m['cohort_inventory']), 48)
        self.assertEqual([m['cohort_inventory'][i]['shared']['run_id'] for i in (0, 6, 12, 18, 24, 30, 36, 42)],
                         ['p00001', 'p00064', 'p10064', 'p10001', 'p20064', 'p20001', 'p30001', 'p30064'])

    def test_reject_reversing_only_arms_and_duplicate_concurrency(self):
        m = matrix()
        m['cohort_inventory'][12:24] = m['cohort_inventory'][18:24] + m['cohort_inventory'][12:18]
        with self.assertRaisesRegex(ValueError, 'inventory/order'):
            audit['validate_matrix_protocol'](m)
        m = matrix()
        m['cohort_inventory'][6:12] = copy.deepcopy(m['cohort_inventory'][:6])
        with self.assertRaisesRegex(ValueError, 'inventory/order'):
            audit['validate_matrix_protocol'](m)

    def test_reject_smoke_missing_arm_and_protocol_substitution(self):
        for field, value in [('smoke', True), ('version', 1), ('version', True),
                             ('protocol_id', 'other'), ('concurrency_points', [True, 64]),
                             ('fixed_rates', [1]), ('attempts', [{}] * 12)]:
            with self.subTest(field=field, value=value):
                m = matrix(); m[field] = value
                with self.assertRaises(ValueError):
                    audit['validate_matrix_protocol'](m)

    def test_reject_relabelled_api_and_bool_worker(self):
        for field, value in [('read_api', 'batch_get'), ('shared', dict(matrix()['cohort_inventory'][0]['shared'], workers=True))]:
            m = matrix(); m['cohort_inventory'][0][field] = value
            with self.assertRaisesRegex(ValueError, 'inventory/order'):
                audit['validate_matrix_protocol'](m)

    def test_all_native_and_redis_exact_configs(self):
        for repeat in (0, 1, 2, 3):
            for workers in (1, 64):
                for _, server, api in ARM_ROWS:
                    audit['validate_requested_config'](config(repeat, workers, server, api), repeat, workers, server, api)

    def test_reject_native_c1_max_in_flight64_and_changed_limits(self):
        for field, value in [('max_in_flight', 64), ('max_in_flight', True),
                             ('max_attempts', 7), ('deadline_ms', 3000), ('retry_backoff_ms', 0),
                             ('epoch_version', 2)]:
            with self.subTest(field=field):
                c = config(0, 1, 'old', 'point_get'); c['client'][field] = value
                with self.assertRaisesRegex(ValueError, 'exact request config'):
                    audit['validate_requested_config'](c, 0, 1, 'old', 'point_get')

    def test_reject_shared_changes_and_redis_get_mget_substitution(self):
        for field, value in [('workers', 64), ('measure_ms', 1500), ('max_calls', 1_000_000),
                             ('read_api', 'mget'), ('deadline_ms', 5000), ('run_id', 'p00064'),
                             ('value_bytes', 64), ('unexpected', 1)]:
            with self.subTest(field=field):
                c = config(0, 1, None, 'get'); c[field] = value
                with self.assertRaisesRegex(ValueError, 'exact request config'):
                    audit['validate_requested_config'](c, 0, 1, None, 'get')

    def test_pairing_never_crosses_concurrency_or_repeat(self):
        cases = [dict(repeat=r, concurrency=c, arm=a, marker=(r, c, a))
                 for r in (0, 1, 2, 3) for c in (1, 64) for a in ('old-point', 'new-point')]
        for r in (0, 1, 2, 3):
            for c in (1, 64):
                old, new = audit['paired_cases'](cases, r, c, 'point')
                self.assertEqual((old['marker'], new['marker']), ((r, c, 'old-point'), (r, c, 'new-point')))
        missing = [x for x in cases if x['marker'] != (0, 1, 'new-point')]
        with self.assertRaisesRegex(ValueError, 'missing/duplicate'):
            audit['paired_cases'](missing, 0, 1, 'point')
        with self.assertRaisesRegex(ValueError, 'missing/duplicate'):
            audit['paired_cases'](cases + [cases[0]], 0, 1, 'point')

    def test_reject_wrong_full_repeat_direction_missing_and_duplicate_repeats(self):
        for repeat in range(4):
            start = repeat * 12
            m = matrix()
            m['cohort_inventory'][start:start+12] = reversed(m['cohort_inventory'][start:start+12])
            with self.subTest(wrong_direction=repeat):
                with self.assertRaisesRegex(ValueError, 'inventory/order'):
                    audit['validate_matrix_protocol'](m)
            m = matrix()
            del m['cohort_inventory'][start:start+12]
            m['attempts'] = [{} for _ in m['cohort_inventory']]
            with self.subTest(omitted_repeat=repeat):
                with self.assertRaisesRegex(ValueError, 'inventory/order'):
                    audit['validate_matrix_protocol'](m)
            m = matrix()
            replacement = ((repeat + 1) % 4) * 12
            m['cohort_inventory'][start:start+12] = copy.deepcopy(m['cohort_inventory'][replacement:replacement+12])
            with self.subTest(duplicated_repeat=repeat):
                with self.assertRaisesRegex(ValueError, 'inventory/order'):
                    audit['validate_matrix_protocol'](m)

    def test_reject_old_two_repeat_five_second_study_and_changed_duration(self):
        m = matrix()
        m['cohort_inventory'] = m['cohort_inventory'][:24]
        m['attempts'] = [{}] * 24
        for descriptor in m['cohort_inventory']:
            descriptor['shared']['measure_ms'] = 5000
        with self.assertRaisesRegex(ValueError, 'inventory/order'):
            audit['validate_matrix_protocol'](m)
        for duration in (5000, 30000, True):
            m = matrix()
            for descriptor in m['cohort_inventory']:
                descriptor['shared']['measure_ms'] = duration
            with self.subTest(duration=duration):
                with self.assertRaisesRegex(ValueError, 'inventory/order'):
                    audit['validate_matrix_protocol'](m)
            for server, api in [('old', 'point_get'), (None, 'get')]:
                c = config(0, 1, server, api); c['measure_ms'] = duration
                with self.assertRaisesRegex(ValueError, 'exact request config'):
                    audit['validate_requested_config'](c, 0, 1, server, api)
        for repeat in (-1, 4, True):
            with self.assertRaisesRegex(ValueError, 'invalid repeat/concurrency'):
                audit['shared_config'](repeat, 1)

    def test_exact_candidate_and_control_pins_are_distinct(self):
        self.assertEqual(audit['PINS']['new'], ('1b70dba62e492cd8e10dd20f643fee74d39abc57',
                         'ab05a16fd520d4c612b01cedea1a514411e607eb0bc645ce28fce24b3299d1ef'))
        self.assertEqual(audit['SERVER_MANIFESTS']['new'],
                         '800e74e36f7de539bb38d3cfe097e356ed2ebd1e33d16d7b48435eb6f2bc93f3')
        self.assertEqual(audit['PINS']['old'], ('57ff6851e40ed63c837189d6eb0a11190704725a',
                         '6d76779a3e421388692b01ce08bd289477283fc05fd28b734eaed9138e0f7e30'))
        self.assertEqual(audit['SERVER_MANIFESTS']['old'],
                         '7e5179f856b8828c26a6bf68ce0056755e16e12bbdc35c9a11845178712caabd')
        self.assertNotEqual(audit['PINS']['old'], audit['PINS']['new'])
        self.assertEqual(set(audit['ROLE_PATHS']), {'old', 'new', 'client', 'redis'})


if __name__ == '__main__':
    unittest.main(verbosity=2)
