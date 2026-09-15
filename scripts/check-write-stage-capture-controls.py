#!/usr/bin/env python3
"""Focused synthetic controls for boundary capture; never launch a fixture."""
import argparse
import ast
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import patch

HERE = Path('/mnt/data/kv9-work/write-stage-capture-preparation-20260915-first')


def load(name,path):
    spec=importlib.util.spec_from_file_location(name,path)
    result=importlib.util.module_from_spec(spec);spec.loader.exec_module(result)
    return result


capture=load('capture_delta',HERE/'capture.py')
hooks=capture.write_stage_capture
driver=load('inherited_delta',HERE/'inherited-driver.py')
synthetic=load('existing_synthetic_shapes',Path(__file__).with_name('check-write-stage-trace-controls.py'))
OUTPUT=None


def raw(node,total=0,exports=10,invalid=False):
    trace=synthetic.snapshot(total)
    if invalid:
        trace['dropped_recording_calls']=1
    return synthetic.status(trace,dict(node_id=str(node),pid=str(100+node)))+(
        f'metrics_export_successes={exports}\nwrite_path_diagnostics={{}}\n').encode()


class Controls(unittest.TestCase):
    def directory(self):
        out=OUTPUT/self._testMethodName;out.mkdir();return out

    def fixture(self):
        return SimpleNamespace(out=Path('/unused-owned-fixture'),
            nodes={n:SimpleNamespace(pid=100+n,poll=lambda:None) for n in [1,2,3]},
            identities={100+n:dict(pid=100+n,start_ticks=456,boot_id='12345678-1234-1234-1234-123456789abc') for n in [1,2,3]})

    def test_exact_four_rows_and_unchanged_native_functions(self):
        rows=capture.plan_rows(driver)
        self.assertEqual([r['mode'] for r in rows],['default','instrumented','instrumented','default'])
        for row in rows:
            self.assertEqual((row['write_api'],row['read_api'],row['shared']['workers'],row['shared']['batch_size']),
                             ('batch_put','batch_get',64,64))
            self.assertEqual([row['shared'][k] for k in ['keys','value_bytes','seed','warmup_calls','measure_ms','max_calls']],
                             [4096,128,71,128,2000,10_000_000])
        old=Path('/mnt/data/kv9-work/upper-bound-requalified-preparation-20260915-first/matched-driver.py').read_bytes()
        self.assertEqual(old,(HERE/'inherited-driver.py').read_bytes())
        self.assertEqual((HERE/'health.py').read_bytes(),(HERE/'origin/health.py').read_bytes())
        self.assertEqual((HERE/'isolate-and-run.py').read_bytes(),(HERE/'origin/isolate-and-run.py').read_bytes())

    def test_actual_streaming_tmpfs_snapshot_method_dispatch(self):
        source=Path('/mnt/data/kv9-work/performance-input-recovery-20260915-first/rebuild-first/source/scripts')
        sys.path.insert(0,str(source))
        try:
            import benchmark
            support=load('actual_streaming_snapshot_mro',source/'native-batch-e2e.py')
            tmpfs=load('actual_tmpfs_snapshot_mro',source/'tmpfs-redis-diagnostic.py')
            class Guarded:
                pass
            class Volatile(Guarded,support.StreamingFixture,tmpfs.TmpfsFixture):
                pass
            self.assertIs(Volatile.snapshot,tmpfs.TmpfsFixture.snapshot)
            mro=Volatile.__mro__;self.assertIs(mro[mro.index(tmpfs.TmpfsFixture)+1],benchmark.Fixture)
            marker=lambda *a:None
            with patch.object(benchmark.Fixture,'snapshot',marker):
                self.assertIs(getattr(super(tmpfs.TmpfsFixture,Volatile.__new__(Volatile)),'snapshot').__func__,marker)
        finally:
            sys.path.remove(str(source))

    def test_exact_freshness_boundary_and_bad_counter(self):
        self.assertFalse(hooks.fresh({'metrics_export_successes':'10'},{'metrics_export_successes':'11'}))
        self.assertTrue(hooks.fresh({'metrics_export_successes':'10'},{'metrics_export_successes':'12'}))
        for value in ['', 'true', '-1', str(1<<64), '1.0']:
            with self.subTest(value=value), self.assertRaises(ValueError):
                hooks.exports({'metrics_export_successes':value})
        with self.assertRaises(ValueError):hooks.exports({})

    def test_first_fresh_invalid_rows_retained_without_quality_retry(self):
        out=self.directory();h=hooks.Hooks(driver,SimpleNamespace(),out,True,lambda:None,HERE/'check-write-stage-trace.py')
        observations=[raw(n) for n in [1,2,3]]+[raw(n,exports=12,invalid=True) for n in [1,2,3]]
        with patch.object(hooks,'read_live',side_effect=observations) as reads, self.assertRaises(ValueError):
            h.capture(self.fixture(),'before')
        self.assertEqual(reads.call_count,6)
        receipt=json.loads((out/'raw-status/before/capture.json').read_text())
        self.assertFalse(receipt['complete']);self.assertEqual(len(receipt['voters']),3)
        for n in [1,2,3]:self.assertEqual((out/f'raw-status/before/node-{n}.status').read_bytes(),raw(n,exports=12,invalid=True))

    def test_owned_lifetime_mismatch_is_preserved_refusal(self):
        out=self.directory();h=hooks.Hooks(driver,SimpleNamespace(),out,True,lambda:None,HERE/'check-write-stage-trace.py')
        observations=[raw(n) for n in [1,2,3]]+[raw(1,exports=12).replace(b'process_start_ticks=456',b'process_start_ticks=457')]
        with patch.object(hooks,'read_live',side_effect=observations),self.assertRaisesRegex(ValueError,'identity'):
            h.capture(self.fixture(),'before')
        self.assertFalse(json.loads((out/'raw-status/before/capture.json').read_text())['complete'])

    def test_hooks_call_originals_and_restore_even_on_error(self):
        calls=[]
        class Fixture:
            def snapshot(self,directory,phase):calls.append(('snapshot',phase))
        bench=SimpleNamespace(Fixture=Fixture)
        original_observe=lambda *a,**k:calls.append(('observe',)) or 'report'
        original_drain=lambda f,d,l:calls.append(('drain',l))
        d=SimpleNamespace(observe_client=original_observe,fresh_drain=original_drain)
        old_snapshot=Fixture.snapshot
        h=hooks.Hooks(d,bench,Path('/unused'),True,lambda:None,HERE/'check-write-stage-trace.py')
        def record(f,p):h.fixture=f;calls.append(('capture',p))
        h.capture=record;fixture=Fixture()
        with self.assertRaisesRegex(ValueError,'synthetic'):
            with h:
                fixture.snapshot(None,'before');self.assertEqual(d.observe_client(),'report')
                d.fresh_drain(fixture,None,'post-client');d.fresh_drain(fixture,None,'post-readback')
                fixture.snapshot(None,'after');raise ValueError('synthetic')
        self.assertIs(Fixture.snapshot,old_snapshot);self.assertIs(d.observe_client,original_observe);self.assertIs(d.fresh_drain,original_drain)
        self.assertEqual(calls,[('snapshot','before'),('capture','before'),('observe',),('capture','post-client'),
            ('drain','post-client'),('capture','post-drain'),('drain','post-readback'),('capture','post-readback'),('snapshot','after')])

    def test_changed_post_drain_tail_reported_and_not_erased(self):
        out=self.directory()
        for k,phase in enumerate(hooks.POLICY['phases']):
            root=out/'raw-status'/phase;root.mkdir(parents=True)
            voters={}
            for n in [1,2,3]:
                data=raw(n,total=k,exports=10+k*2);p=root/f'node-{n}.status';p.write_bytes(data)
                voters[str(n)]=dict(path=str(p),bytes=len(data),sha256=hashlib.sha256(data).hexdigest())
            hooks.save(root/'capture.json',dict(complete=True,voters=voters))
        hooks.compare_retained(out,True,HERE/'check-write-stage-trace.py')
        result=json.loads((out/'raw-status/comparison.json').read_text())
        self.assertTrue(result['complete']);self.assertFalse(result['complete_history'])
        delta=result['nodes']['1']['post-client_to_post-drain']
        self.assertFalse(delta['group_ring_unchanged']);self.assertFalse(delta['inspection_ring_unchanged'])
        self.assertEqual(delta['counter_delta']['group_commands_seen'],1)
        self.assertEqual(delta['coverage']['groups']['new_recorded_rows'],1)

    def test_output_and_filesystem_policy_bounds(self):
        self.assertEqual(capture.POLICY['max_campaign_decrease_bytes'],13*1024**3)
        self.assertEqual(capture.POLICY['max_cohort_payload_bytes'],3*1024**3)
        floor=capture.POLICY['host_floor_bytes'];cap=capture.POLICY['max_campaign_decrease_bytes']
        capture.check_space(floor,floor+cap)
        with self.assertRaises(ValueError):capture.check_space(floor-1,floor+cap)
        with self.assertRaises(ValueError):capture.check_space(floor,floor+cap+1)
        self.assertEqual(hooks.POLICY['max_raw_bytes_per_cohort'],4*3*hooks.POLICY['max_status_bytes'])


def main():
    global OUTPUT
    parser=argparse.ArgumentParser();parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args();OUTPUT=args.output.resolve()
    if not OUTPUT.is_relative_to(Path('/mnt/data/kv9-work')):raise ValueError('data-volume output required')
    OUTPUT.mkdir()
    result=unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(Controls))
    pins={str(p):dict(bytes=p.stat().st_size,sha256=hashlib.sha256(p.read_bytes()).hexdigest()) for p in
          [Path(__file__),HERE/'capture.py',HERE/'write_stage_capture.py',HERE/'inherited-driver.py',HERE/'check-write-stage-trace.py']}
    hooks.save(OUTPUT/'result.json',dict(complete=result.wasSuccessful(),tests=result.testsRun,failures=len(result.failures),
        errors=len(result.errors),scope='Synthetic changed capture boundaries and arithmetic only; no fixture, workload or old control replay.',source_pins=pins))
    return 0 if result.wasSuccessful() else 1


if __name__=='__main__':raise SystemExit(main())
