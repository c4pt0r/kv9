"""Bounded write-screen scope and inherited arithmetic checks; no runtime."""
import ast
import copy
import importlib.util
import json
from pathlib import Path
import unittest

HERE = Path(__file__).resolve().parent
BASE = Path('/tmp/kv9-owned-batch-broad-workloads-preparation')


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


driver = module('screen_driver', HERE / 'matched-driver.py')
contracts = module('screen_audit_contract', HERE / 'test_audit_contract.py')
core = module('screen_statistics_core', HERE / 'statistics/core.py')


class ScreenContract(unittest.TestCase):
    def test_all_three_independent_inventories_agree_without_runtime(self):
        protocol = json.loads((HERE / 'protocol.json').read_text())
        independently_enumerated = contracts.matrix()['cohort_inventory']
        self.assertEqual(driver.make_plan(False), independently_enumerated)
        self.assertEqual(protocol['timed_inventory'], independently_enumerated)
        self.assertEqual(protocol['smoke_inventory'], driver.make_plan(True))
        self.assertEqual(protocol['workload_cells'], [['point', 1, 0], ['batch64', 64, 0]])
        self.assertFalse(protocol['read_or_mixed_workloads_accepted'])
        self.assertEqual(len(independently_enumerated), 24)
        self.assertEqual({row['shared']['workers'] for row in independently_enumerated}, {1, 64})
        self.assertTrue(all(row['mix'] == 'write' and row['shared']['read_percent'] == 0
                            for row in independently_enumerated))

    def test_read_mixed_or_c64_only_schedule_is_rejected(self):
        for reads in (50, 100):
            changed = contracts.matrix()
            for row in changed['cohort_inventory']:
                row['shared']['read_percent'] = reads
                row['mix'] = 'mixed' if reads == 50 else 'read'
            with self.assertRaisesRegex(ValueError, 'inventory/order'):
                contracts.audit['validate_matrix_protocol'](changed)
        changed = contracts.matrix()
        changed['cohort_inventory'] = [row for row in changed['cohort_inventory']
                                       if row['shared']['workers'] == 64]
        changed['attempts'] = [{}] * len(changed['cohort_inventory'])
        with self.assertRaisesRegex(ValueError, 'inventory/order'):
            contracts.audit['validate_matrix_protocol'](changed)
        contracts.audit['validate_matrix_protocol'](contracts.matrix())

    def test_lifecycle_storage_histogram_and_wrapper_bytes_are_inherited(self):
        def functions(path):
            tree = ast.parse(path.read_text())
            return {node.name: ast.dump(node, include_attributes=False)
                    for node in tree.body if isinstance(node, ast.FunctionDef)}
        for name, exceptions in [('matched-driver.py', {'make_plan', 'main'}),
                                 ('audit.py', {'validate_matrix_protocol'})]:
            old, new = functions(BASE / name), functions(HERE / name)
            self.assertEqual(set(old), set(new))
            for key in old.keys() - exceptions:
                self.assertEqual(old[key], new[key], (name, key))
        for name in ['isolate-and-run.py', 'statistics/core.py']:
            self.assertEqual((BASE / name).read_bytes(), (HERE / name).read_bytes(), name)

    def test_statistics_keep_weighted_whole_call_histograms_and_empty_reads(self):
        def raw(values):
            return contracts.population(values)['whole_call']['raw']
        result = core.merge_histograms([raw([1] * 99), raw([1024])])
        self.assertEqual(result['count'], 100)
        self.assertEqual(result['sum_ns'], 1123)
        self.assertEqual(result['mean_ns'], 11.23)
        self.assertEqual(result['p99'], {'lower_ns': 1, 'upper_ns': 1})
        empty = core.merge_histograms([raw([])])
        self.assertEqual(empty, dict(count=0, sum_ns=0, mean_ns=None,
                                     p50=None, p95=None, p99=None))
        # An inactive read operation cannot alter the write call distribution.
        self.assertEqual(core.merge_histograms([raw([]), raw([1] * 99), raw([1024])]), result)
        self.assertEqual(core.add_maps([{'success': 3, 'unknown_write': 2},
                                         {'success': 4, 'refused': 1}]),
                         {'success': 7, 'unknown_write': 2, 'refused': 1})


if __name__ == '__main__':
    unittest.main(verbosity=2)
