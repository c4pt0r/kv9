"""Focused continuation metadata controls only; no original payload or child execution."""
import ast
import copy
import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch
import campaign as c


class ContinuationControls(unittest.TestCase):
    def setUp(self):
        base = c.P / 'controls-data'; base.mkdir(exist_ok=True)
        self.root = Path(tempfile.mkdtemp(prefix=self._testMethodName + '-', dir=base))

    def test_actual013_tool_child_result_and_false_receipts(self):
        tool = c.js(c.RECONCILED_ROOT / 'tool-terminal.json')
        child = c.js(c.RECONCILED_ROOT / 'launch-first/ordinal013_finish-child-terminal.json')
        cmd = c.js(c.RECONCILED_ROOT / 'commands.json')['ordinal013_finish']
        c.reconciled_terminal_check(tool, child, cmd['argv'], cmd['result'], child['result_sha256'])
        for key, value in [('exit_code', 1), ('chunk_id', 'wrong')]:
            bad = copy.deepcopy(tool); bad['terminal'][key] = value
            with self.assertRaises(RuntimeError):
                c.reconciled_terminal_check(bad, child, cmd['argv'], cmd['result'], child['result_sha256'])
        for key, value in [('complete', False), ('exit_code', 1), ('result_path', '/wrong'), ('result_sha256', 'wrong'), ('argv', ['wrong'])]:
            bad = copy.deepcopy(child); bad[key] = value
            same = copy.deepcopy(tool); same['terminal']['output'] = json.dumps(bad)
            with self.assertRaises(RuntimeError):
                c.reconciled_terminal_check(same, bad, cmd['argv'], cmd['result'], child['result_sha256'])

    def test_independent013_success_failure_scope_and_authority(self):
        summary = c.js(c.INDEPENDENT_ROOT / 'accepted-summary.json')
        args = [summary['selection_sha256'], summary['corrected_reader_binding'], summary['actual_tool_terminal'], summary['actual_child_terminal'], summary['phase_hashes']['retire-2/result.json']]
        c.independent013_check(summary, *args)
        for key, value in [('complete', False), ('state', 'RESTORED'), ('targets', 208), ('objects', 343), ('original_failed_readback_preserved', False), ('charged_decoded_bytes', 31155519021), ('corrected_reader_binding', {}), ('actual_child_terminal', {})]:
            bad = dict(summary); bad[key] = value
            with self.subTest(key=key), self.assertRaises(RuntimeError): c.independent013_check(bad, *args)
        bad = dict(summary, lifetimes=[*summary['lifetimes'][:-1], dict(summary['lifetimes'][-1], same_lifetime_present=True)])
        with self.assertRaises(RuntimeError): c.independent013_check(bad, *args)

    def test_continuation_prefix_begins014_and_refuses_unknown(self):
        self.assertEqual(c.resume_prefix(self.root), ([], 14))
        p = self.root / '014'; p.mkdir()
        with self.assertRaises(RuntimeError): c.resume_prefix(self.root)
        (p / 'complete.json').write_text(json.dumps(dict(complete=True, ordinal=14)))
        self.assertEqual(c.resume_prefix(self.root)[1], 15)
        (self.root / '016').mkdir()
        with self.assertRaises(RuntimeError): c.resume_prefix(self.root)

    def test_old_prefix_cannot_be_invented_inside_new_root(self):
        (self.root / '013').mkdir()
        (self.root / '013/complete.json').write_text(json.dumps(dict(complete=True, ordinal=13)))
        with self.assertRaises(RuntimeError): c.resume_prefix(self.root)

    def test_combined_global_prefix_and_later_handle_refusal(self):
        execution = self.root / 'exec'; execution.mkdir()
        rows = [dict(ordinal=i, root=str(self.root / f'tx{i}')) for i in range(16)]
        for i in range(14):
            Path(rows[i]['root']).mkdir(); (execution / f'{i:03d}').mkdir()
        with patch.object(c, 'EXEC', execution):
            c.global_prefix(dict(rows=rows), 13)
            (execution / '015').mkdir()
            with self.assertRaises(RuntimeError): c.global_prefix(dict(rows=rows), 13)

    def test_new_stop_equality_below_and_historical_plan_separation(self):
        self.assertEqual(c.STOP, 85000000000)
        self.assertEqual(c.HISTORICAL_STOP, 100000000000)
        self.assertIsNone(c.stop_reason(c.STOP - 1, 14))
        self.assertEqual(c.stop_reason(c.STOP, 14), 'actual_available_target')
        self.assertEqual(c.stop_reason(c.STOP - 1, 96), 'finite_exhaustion')

    def test_all_extra_roots_reduce_net_without_historical_target_relabel(self):
        original = SimpleNamespace(account=lambda g: dict(conservative_net_allocated_change_bytes=1000, target_reached=False, actual_available_bytes=c.STOP, scope='original'))
        extras = [dict(path=str(p), allocated_bytes=i + 1, cap_bytes=cap) for i, (p, cap) in enumerate(c.extra_accounting_roots())]
        with patch.object(c, 'extra_allocation', return_value=extras):
            r = c.accounting(original, {})
        self.assertEqual(r['conservative_net_allocated_change_bytes'], 1000 - sum(x['allocated_bytes'] for x in extras))
        self.assertTrue(r['target_reached'])
        self.assertFalse(r['historical_plan_target_reached'])
        self.assertFalse(r['benchmark_runtime_ready'])
        self.assertEqual(len({x['path'] for x in extras}), len(extras))
        self.assertIn(str(c.PRIOR_OUT), {x['path'] for x in extras})
        self.assertIn(str(c.FAILURE_ROOT), {x['path'] for x in extras})
        self.assertIn(str(c.REPAIR), {x['path'] for x in extras})

    def test_missing_overcap_and_duplicate_extra_allocation_refused(self):
        execution = self.root / 'exec'; execution.mkdir()
        existing = self.root / 'metadata'; existing.mkdir()
        with patch.object(c, 'EXEC', execution):
            with patch.object(c, 'extra_accounting_roots', return_value=[(existing, 10)]), patch.object(c, 'allocated', return_value=11), self.assertRaises(RuntimeError): c.extra_allocation()
            with patch.object(c, 'extra_accounting_roots', return_value=[(self.root / 'absent', 10)]), self.assertRaises(RuntimeError): c.extra_allocation()
            with patch.object(c, 'extra_accounting_roots', return_value=[(existing, 10), (existing, 10)]), patch.object(c, 'allocated', return_value=0), self.assertRaises(RuntimeError): c.extra_allocation()

    def test_finish_only_uses_frozen_corrected_adapter(self):
        a = c.finish_argv(14, 'actual-verification')
        self.assertEqual(a[1], str(c.REPAIR / 'reconcile.py'))
        for flag, value in [('--ordinal', '14'), ('--code-pins-sha256', c.CODE), ('--release-verified-sha256', 'actual-verification'), ('--preparation-inventory-sha256', c.REPAIR_PIN), ('--reader-manifest-sha256', c.READER_PIN)]:
            self.assertEqual(a[a.index(flag) + 1], value)
        for i in [13, 96]:
            with self.assertRaises(RuntimeError): c.finish_argv(i, 'v')

    def test_original_release_bootstrap_and_old_completed_bodies_unchanged(self):
        old = ast.parse((c.P / 'originals/campaign.py').read_text())
        new = ast.parse((c.P / 'campaign.py').read_text())
        def body(tree, name):
            node = next(n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name == name)
            return ast.dump(ast.Module(body=node.body, type_ignores=[]), include_attributes=False)
        for name in ['verify_release', 'zero_gate', 'zero_audit_binding', 'global_prefix', 'codec_roles', 'codec_receipt', 'sidecar_closure']:
            self.assertEqual(body(old, name), body(new, name), name)
        self.assertEqual(body(old, 'completed'), body(new, 'completed_original'))

    def test_corrected_completed_checks_effective_reader_before_cold(self):
        root = self.root; pair = dict(id='t', target='t', base='b')
        member = dict(id='t', path=str(root / 'absent-target'))
        base = dict(id='b', path=str(root / 'base'), identity={})
        s = dict(root=str(root), pairs=[pair], members=[base, member], metadata=[], target_count=1)
        row = dict(selection=dict(sha256='selection'))
        rbind = dict(readback_reader={'sha256': 'corrected'}, original_selected_reader={'sha256': 'old'})
        calls = []
        restore = dict(rows=[dict(id='t', identity={}, original_bytes_exact=True)], all_children_reaped=True, children=[dict(output=str(root / f'r{i}')) for i in range(3)])
        stage = dict(rows=[dict(patch=dict(path='/patch', identity={}), children=[dict(output=str(root / f's{i}')) for i in range(3)])])
        verify = dict(staged_sha256='stage', children=[dict(output=str(root / f'v{i}')) for i in range(3)])
        children = [*restore['children'], *stage['rows'][0]['children'], *verify['children']]
        sidecars = {x['output'] + '.child.json': x for x in children}
        def fake_js(p):
            if str(p) in sidecars: return sidecars[str(p)]
            return stage if Path(p).name == 'staged.json' else verify
        def fake_pin(p):
            return dict(sha256='stage' if Path(p).name == 'staged.json' else 'verified' if Path(p).parent.name == 'verification-first' else 'hash')
        def phase(i, name, path, expected):
            calls.append((name, str(path), expected))
            return restore if name == 'restore-1' else dict(rows=[dict(id='t')]) if name == 'retire-2' else dict(complete=True)
        auth = SimpleNamespace(reader_binding=lambda s: rbind, repaired_reader_ok=lambda *a: True, reconciliation_binding=lambda *a: dict(corrected='hash'))
        with patch.object(c, 'selected', return_value=(row, s)), patch.object(c, 'phase_receipt', side_effect=phase), patch.object(c, 'AUTHORITY', auth), patch.object(c, 'pin', side_effect=fake_pin), patch.object(c, 'js', side_effect=fake_js), patch.object(c, 'identity', return_value={}), patch.object(c, 'pair_proofs'), patch.object(c, 'sidecar_closure'), patch.object(c, 'outcome'):
            result = c.completed({}, 14, 'verified')
            self.assertEqual(calls[1][0], 'readback-1-repaired')
            self.assertEqual(calls[1][2]['original_selected_reader'], rbind['original_selected_reader'])
            self.assertEqual(calls[2][2]['reconciliation'], dict(corrected='hash'))
            self.assertIn('readback-1-repaired-first', result['readback_path'])
            calls.clear(); auth.repaired_reader_ok = lambda *a: (_ for _ in ()).throw(RuntimeError('incomplete corrected reader'))
            with self.assertRaises(RuntimeError): c.completed({}, 14, 'verified')
            self.assertEqual([r[0] for r in calls], ['restore-1', 'readback-1-repaired'])


if __name__ == '__main__': unittest.main(verbosity=2)
