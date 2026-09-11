#!/usr/bin/env python3
"""Check the declared longer c1 study against unchanged client validators."""
import copy
import importlib.util
from pathlib import Path
import sys
import unittest

sys.dont_write_bytecode = True
sys.path.insert(0, '/tmp/kv9-point-batch1-measurement/scripts')
import batch_benchmark_report as native


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


driver = module('direct_peer_driver', Path(__file__).with_name('matched-driver.py'))
redis = module('redis_validator', '/tmp/kv9-redis-point-get-reference/scripts/redis_batch_report.py')
peers = [dict(node_id=n, address=f'127.0.0.1:{20000+n}') for n in (1, 2, 3)]


class DriverContract(unittest.TestCase):
    def pairs(self, smoke=False):
        return [driver.configs(row, peers, 1, '127.0.0.1:21000')
                for row in driver.make_plan(smoke)]

    def test_full_matrix_accepted_by_original_client_contracts(self):
        pairs = self.pairs()
        self.assertEqual(len(pairs), 24)
        for n, r in pairs:
            native.config_check(n)
            redis.config_check(r)
            redis.paired_configuration(r, n)
            self.assertEqual(n['workers'], n['client']['max_in_flight'])
            self.assertEqual(n['client']['max_attempts'], 6)
            self.assertEqual(n['client']['deadline_ms'], 1500)

    def test_fixed_width_dataset_prefix_and_four_counterbalanced_repeats(self):
        rows = driver.make_plan(False)
        arms = ['old-point', 'old-batch1', 'new-point', 'new-batch1',
                'redis-mget1', 'redis-get1']
        for repeat, expected in enumerate([arms, list(reversed(arms)), list(reversed(arms)), arms]):
            group = rows[repeat*6:(repeat+1)*6]
            self.assertEqual([r['arm'] for r in group], expected)
            self.assertEqual({r['repeat'] for r in group}, {repeat})
            self.assertEqual({r['shared']['workers'] for r in group}, {1})
            self.assertEqual({r['shared']['measure_ms'] for r in group}, {30_000})
            self.assertEqual({r['shared']['run_id'] for r in group}, {f'p{repeat}0001'})
        self.assertEqual({len(r['shared']['run_id']) for r in rows}, {6})
        self.assertEqual(sum(r['shared']['measure_ms'] for r in rows), 720_000)

    def test_optional_smoke_contains_each_c1_arm(self):
        rows = driver.make_plan(True)
        self.assertEqual([(r['shared']['workers'], r['arm']) for r in rows],
                         [(1, arm) for arm in
                          ('old-point', 'old-batch1', 'new-point', 'new-batch1',
                           'redis-mget1', 'redis-get1')])
        for n, r in self.pairs(True):
            redis.paired_configuration(r, n)

    def test_mismatched_api_concurrency_or_dataset_rejected(self):
        n, r = self.pairs()[0]
        for field, value in [('read_api', 'mget'), ('workers', 8), ('run_id', 'other1')]:
            changed = copy.deepcopy(r)
            changed[field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                redis.paired_configuration(changed, n)

    def test_native_capacity_excess_is_rejected(self):
        n, _ = self.pairs()[-1]
        n['workers'] = 256
        n['client']['max_in_flight'] = 128
        with self.assertRaises(ValueError):
            native.config_check(n)

    def test_point_get_cannot_silently_become_batch_work(self):
        n, r = self.pairs()[0]
        n['batch_size'] = r['batch_size'] = 2
        with self.assertRaises(ValueError):
            native.config_check(n)
        with self.assertRaises(ValueError):
            redis.config_check(r)


if __name__ == '__main__':
    unittest.main(verbosity=2)
