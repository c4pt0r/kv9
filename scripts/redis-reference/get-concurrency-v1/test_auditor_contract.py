#!/usr/bin/env python3
"""Offline tests of auditor-only pure contracts; never runs the audit body."""
import ast
from copy import deepcopy
import json
from pathlib import Path
import unittest


SOURCE = Path(__file__).with_name('audit.py')
PURE_FUNCTIONS = {'require', 'histogram', 'expected_cohorts', 'expected_descriptor',
                  'check_curve_inventory', 'check_native_config', 'check_fixed_capacity'}
CONSTANTS = {'PROTOCOL_ID', 'POINTS', 'PINS', 'OLD_REV', 'CLIENT_REV', 'REDIS_REV'}
tree = ast.parse(SOURCE.read_text(), filename=str(SOURCE))
nodes = [node for node in tree.body if
         (isinstance(node, ast.FunctionDef) and node.name in PURE_FUNCTIONS) or
         (isinstance(node, ast.Assign) and any(isinstance(t, ast.Name) and t.id in CONSTANTS
                                             for t in node.targets))]
assert {n.name for n in nodes if isinstance(n, ast.FunctionDef)} == PURE_FUNCTIONS
contract = {'json': json}
exec(compile(ast.Module(body=nodes, type_ignores=[]), str(SOURCE), 'exec'), contract)


def matrix():
    return dict(version=2, protocol_id='kv9-point-get-concurrency-v1',
                concurrency_points=[1, 8, 32, 64, 128, 256],
                role_bindings={'old': {}, 'client': {}, 'redis': {}},
                attempts=[{} for _ in range(24)],
                cohort_inventory=[contract['expected_descriptor'](*row)
                                  for row in contract['expected_cohorts']()])


def native_config(workers):
    return {'client': dict(version=1, peers=[{'node_id': 1, 'address': 'http://127.0.0.1:1'}],
                           keyspace_id='raw', epoch_conf_ver=1, epoch_version=1,
                           max_in_flight=workers, max_attempts=6, deadline_ms=1500,
                           retry_backoff_ms=5)}


def population(count=0, total=0, bucket=None):
    buckets = [0] * 3776
    if bucket is not None:
        buckets[bucket] = count
    return {'whole_call': {'raw': {'count': count, 'sum_ns': total, 'buckets': buckets}}}


class InventoryTests(unittest.TestCase):
    def test_complete_inventory_and_whole_reversal(self):
        value = matrix()
        contract['check_curve_inventory'](value)
        first, second = value['cohort_inventory'][:12], value['cohort_inventory'][12:]
        self.assertEqual([(r['shared']['workers'], r['arm']) for r in first],
                         [(c, arm) for c in (1, 8, 32, 64, 128, 256)
                          for arm in ('kv9-point', 'redis-get')])
        self.assertEqual([(r['shared']['workers'], r['arm']) for r in second],
                         list(reversed([(r['shared']['workers'], r['arm']) for r in first])))
        self.assertTrue(all(len(r['shared']['run_id']) == 6 for r in first + second))

    def test_reverse_targets_only_is_rejected(self):
        value = matrix()
        value['cohort_inventory'][12:] = sorted(value['cohort_inventory'][12:],
                                                key=lambda r: r['shared']['workers'])
        with self.assertRaisesRegex(ValueError, 'inventory/order'):
            contract['check_curve_inventory'](value)

    def test_missing_and_duplicated_cohorts_are_rejected(self):
        value = matrix()
        value['attempts'].pop()
        with self.assertRaisesRegex(ValueError, '24-cohort'):
            contract['check_curve_inventory'](value)
        value = matrix()
        value['cohort_inventory'][2] = deepcopy(value['cohort_inventory'][0])
        with self.assertRaisesRegex(ValueError, 'inventory/order'):
            contract['check_curve_inventory'](value)

    def test_wrong_protocol_version_and_role_set_are_rejected(self):
        for key, changed in [('version', True), ('version', 1), ('protocol_id', 'matched-v1'),
                             ('role_bindings', {'old': {}, 'new': {}, 'client': {}, 'redis': {}}),
                             ('role_bindings', {'old': {}, 'client': {}})]:
            with self.subTest(key=key, changed=changed):
                value = matrix()
                value[key] = changed
                with self.assertRaises(ValueError):
                    contract['check_curve_inventory'](value)

    def test_points_and_descriptors_reject_numeric_type_substitution(self):
        for changed in ([True, 8, 32, 64, 128, 256], [1, 8, 32, 64, 128, 128]):
            value = matrix()
            value['concurrency_points'] = changed
            with self.assertRaisesRegex(ValueError, 'point inventory'):
                contract['check_curve_inventory'](value)
        for changed in (True, 1.0):
            value = matrix()
            value['cohort_inventory'][0]['shared']['workers'] = changed
            with self.assertRaisesRegex(ValueError, 'inventory/order'):
                contract['check_curve_inventory'](value)

    def test_mget_or_shared_workload_change_is_rejected(self):
        for key, changed in [('measure_ms', 1500), ('warmup_calls', 32), ('keys', 64),
                             ('max_calls', 1_000_000), ('batch_size', 8), ('run_id', 'g00064')]:
            value = matrix()
            value['cohort_inventory'][0]['shared'][key] = changed
            with self.assertRaisesRegex(ValueError, 'inventory/order'):
                contract['check_curve_inventory'](value)
        value = matrix()
        value['cohort_inventory'][1]['read_api'] = 'mget'
        with self.assertRaisesRegex(ValueError, 'inventory/order'):
            contract['check_curve_inventory'](value)


class LimitTests(unittest.TestCase):
    def test_client_concurrency_is_exact_with_unchanged_deadline_and_retry(self):
        for c in (1, 8, 32, 64, 128, 256):
            contract['check_native_config'](native_config(c), c)
        for key, changed in [('max_in_flight', 64), ('deadline_ms', 3000),
                             ('max_attempts', 1), ('retry_backoff_ms', 0), ('version', True)]:
            value = native_config(256)
            value['client'][key] = changed
            with self.assertRaisesRegex(ValueError, 'concurrency/deadline/retry'):
                contract['check_native_config'](value, 256)

    def test_server_capacity_does_not_scale_with_workers(self):
        fixed = dict(public_rpc_limit_requests='64', public_rpc_limit_encoded_bytes='16777216',
                     raft_async_read_limit='128')
        contract['check_fixed_capacity'](fixed)
        for key, changed in [('public_rpc_limit_requests', '256'),
                             ('public_rpc_limit_encoded_bytes', '67108864'),
                             ('raft_async_read_limit', '256')]:
            value = dict(fixed, **{key: changed})
            with self.assertRaisesRegex(ValueError, 'admission capacity'):
                contract['check_fixed_capacity'](value)


class HistogramTests(unittest.TestCase):
    def test_zero_success_is_null_not_a_fake_zero_latency(self):
        result = contract['histogram']([population(), population()])
        self.assertEqual(result, dict(count=0, sum_ns=0, mean_ns=None,
                                      p50=None, p95=None, p99=None))
        self.assertIsNone(json.loads(json.dumps(result))['p99'])

    def test_all_failure_population_still_has_latency(self):
        success, failures = population(), population(3, 3060, 319)
        self.assertIsNone(contract['histogram']([success])['mean_ns'])
        merged = contract['histogram']([success, failures])
        self.assertEqual(merged['count'], 3)
        self.assertEqual(merged['mean_ns'], 1020)
        self.assertEqual(merged['p99'], {'lower_ns': 1016, 'upper_ns': 1023})

    def test_nonempty_histogram_retains_integer_sum_and_bucket_intervals(self):
        result = contract['histogram']([population(1, 63, 63), population(1, 129, 128)])
        self.assertEqual(result['sum_ns'], 192)
        self.assertEqual(result['mean_ns'], 96)
        self.assertEqual(result['p50'], {'lower_ns': 63, 'upper_ns': 63})
        self.assertEqual(result['p99'], {'lower_ns': 128, 'upper_ns': 129})

    def test_empty_nonzero_sum_and_count_mismatch_are_rejected(self):
        for value in (population(0, 1), population(1, 1)):
            with self.assertRaises(ValueError):
                contract['histogram']([value])


if __name__ == '__main__':
    unittest.main(verbosity=2)
