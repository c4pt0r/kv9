"""Finite driver contracts; no server, workload, container or benchmark launch."""
import ast
import copy
import importlib.util
from pathlib import Path
import sys
import types
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('v3_driver', HERE/'matched-driver.py')
d = importlib.util.module_from_spec(spec)
spec.loader.exec_module(d)
SOURCE = Path('/tmp/kv9-point-write-measurement-v3')
sys.path.insert(0, str(SOURCE/'scripts'))
import batch_benchmark_report as native
import redis_batch_report as redis
resp = d.module('bounded_resp_check', Path('/tmp/kv9-redis-batch-reference-preparation/run_fixture.py'))


class Protocol(unittest.TestCase):
    def test_independent_72_order_and_exact_36_smoke(self):
        expected = [(c, f'{kind}-r{reads:03d}', batch, reads, role)
                    for c in [1, 64] for kind, batch in [('point', 1), ('batch64', 64)]
                    for reads in [0, 50, 100] for role in ['old', 'new', 'redis']]
        plan = d.make_plan(False)
        self.assertEqual(len(plan), 72)
        self.assertEqual(d.PROTOCOL_ID, 'kv9-crc-broad-workloads-c1-c64-v1')
        for repeat in (0, 1):
            for row, (c, workload, batch, reads, role) in zip(plan[repeat*36:(repeat+1)*36],
                    expected if repeat == 0 else list(reversed(expected))):
                point = batch == 1
                self.assertEqual(row, dict(repeat=repeat, arm=role+'-'+workload,
                    server_role=None if role == 'redis' else role, workload=workload,
                    read_api=('get' if point else 'mget') if role == 'redis' else ('point_get' if point else 'batch_get'),
                    write_api=('set' if point else 'mset') if role == 'redis' else ('point_put' if point else 'batch_put'),
                    mix={0:'write',50:'mixed',100:'read'}[reads], target='redis-memory' if role == 'redis' else 'kv9',
                    shared=dict(run_id=f'p{repeat}{c:04d}', seed=71, workers=c, keys=4096, batch_size=batch,
                        value_bytes=128, read_percent=reads, warmup_calls=128, measure_ms=10000,
                        max_calls=10_000_000, load={'kind':'closed_loop'})))
        smoke = d.make_plan(True)
        self.assertEqual(len(smoke), 36)
        for row, wanted in zip(smoke, plan[:36]):
            self.assertEqual(row['shared']['measure_ms'], 2000)
            self.assertEqual(row | {'shared':dict(row['shared'],measure_ms=10000)}, wanted)

    def test_real_validators_and_each_role_pair_same_config(self):
        peers = [dict(node_id=n,address=f'127.0.0.1:{20159+n}') for n in (1,2,3)]
        for smoke in (True, False):
            groups = {}
            for row in d.make_plan(smoke):
                c, r = d.configs(row, peers, 1, '127.0.0.1:6379')
                native.config_check(c); redis.config_check(r); redis.paired_configuration(r,c)
                self.assertEqual(c['version'],3);self.assertEqual(r['version'],3)
                self.assertEqual(c['client']['max_attempts'],6)
                self.assertEqual(c['client']['max_in_flight'],row['shared']['workers'])
                self.assertEqual(c['client']['deadline_ms'],1500)
                key = (row['repeat'],row['shared']['workers'],row['workload'])
                if key in groups:
                    self.assertEqual((c,r),groups[key])
                groups[key] = (c,r)
                wire = native.config_check(c)[1]
                self.assertLessEqual(row['shared']['workers']*max(wire['get_request_bytes'],wire['put_request_bytes']),16*1024**2)
                wrong = copy.deepcopy(r)
                wrong['write_api'] = 'mset' if r['write_api']=='set' else 'set'
                with self.assertRaises(ValueError):redis.paired_configuration(wrong,c)
                if c['batch_size']==64:
                    wrong=copy.deepcopy(c);wrong['write_api']='point_put'
                    with self.assertRaises(ValueError):native.config_check(wrong)

    def test_unmodified_lifecycle_functions_and_scan(self):
        original = Path('/tmp/kv9-work-signal-comparison-preparation/matched-driver.py').read_text()
        updated = (HERE/'matched-driver.py').read_text()
        def functions(text):
            return {n.name:ast.get_source_segment(text,n) for n in ast.parse(text).body if isinstance(n,ast.FunctionDef)}
        a,b=functions(original),functions(updated)
        for name in ['placement','affinity','background_inventory','host_sample','source_binding','cargo_server',
                     'verify_client_cargo','fresh_drain','allocator_provenance']:
            self.assertEqual(a[name],b[name],name)
        self.assertEqual(a['native_readback'].replace('readonly_dataset(', 'final_dataset('), b['native_readback'])
        self.assertEqual(a['summary_from_report'].replace('point/batch1 call','point/batch call'),b['summary_from_report'])

    def test_storage_boundaries_and_unavailable_evidence(self):
        for phase,limits in {'preflight':{'tmpfs':32,'retention':96},'runtime':{'tmpfs':16,'retention':64}}.items():
            row={'filesystems':{k:{'available_bytes':v*1024**3} for k,v in limits.items()}}
            d.require_storage(row,phase)
            for key in limits:
                bad=copy.deepcopy(row);bad['filesystems'][key]['available_bytes']-=1
                with self.assertRaisesRegex(ValueError,'storage guard intervention'):d.require_storage(bad,phase)
        with patch.object(d.os,'statvfs',side_effect=OSError('retained filesystem unavailable')):
            with self.assertRaisesRegex(ValueError,'storage observation failed'):d.storage_observation(HERE)

    def test_failed_outcomes_preserved_and_health_separate(self):
        row=d.make_plan(False)[3]  # new? actual mixed cell selected explicitly below
        row=next(r for r in d.make_plan(False) if r['server_role']=='old' and r['shared']['read_percent']==50)
        c,_=d.configs(row,[dict(node_id=1,address='127.0.0.1:20160')],1,'127.0.0.1:6379')
        op=lambda counts,attempts:dict(populations=[dict(calls=n) for n in counts],
                                      attempts=[dict(raw=dict(count=n)) for n in attempts])
        report={'measured_issued':5,'metrics':{'measurement':{'operations':['get','put'],
            'statistics':[op([1,0,0,1,0],[1,0,1]),op([1,0,2,0,0],[1,0,2])]}}}
        observed=d.population_observations(report,c,False)['measurement']
        self.assertEqual(observed['calls'],[2,3]);self.assertEqual(observed['successes'],[1,1])
        self.assertFalse(observed['all_success_single_attempt'])
        wrong=copy.deepcopy(report);wrong['metrics']['measurement']['operations']=['batch_get','batch_put']
        with self.assertRaises(ValueError):d.population_observations(wrong,c,False)
        # A fully successful operation with an extra SDK attempt is retained but
        # does not satisfy the healthy single-attempt suitability flag.
        healthy=copy.deepcopy(report)
        healthy['metrics']['measurement']['statistics']=[op([2,0,0,0,0],[2,0,0]),op([3,0,0,0,0],[3,1,0])]
        observed=d.population_observations(healthy,c,False)['measurement']
        self.assertTrue(observed['all_success']);self.assertFalse(observed['all_success_single_attempt'])

    def test_write_dataset_membership_sentinel_and_nonce_controls(self):
        row=next(r for r in d.make_plan(False) if r['server_role']=='old' and r['shared']['batch_size']==64
                 and r['shared']['read_percent']==0)
        c,_=d.configs(row,[dict(node_id=1,address='127.0.0.1:20160')],1,'127.0.0.1:6379')
        def value(index,nonce):
            result=index.to_bytes(8,'big')+nonce.to_bytes(8,'big')
            for word in range((c['value_bytes']-16+7)//8):
                result+=resp.mix(c['seed']^resp.mix(index)^resp.mix(nonce)^word).to_bytes(8,'big')
            return result[:c['value_bytes']]
        key=lambda i:f'{c["run_id"]}:{i:016x}'.encode()
        data={key(i):value(i,0) for i in range(c['keys']+1)}
        nonce=c['warmup_calls']+c['max_calls']
        first=resp.mix(c['seed']^nonce^0xc6275c213842315b)%c['keys']
        data[key(first)]=value(first,nonce)
        got=d.final_dataset(c,data,resp)
        self.assertEqual(got['changed_keys'],1)
        self.assertFalse(got['exact_issued_nonce_set_checked'])
        for kind in ('missing','sentinel','wrong-key','bad-bytes','nonce-over-bound'):
            bad=dict(data)
            if kind=='missing':bad.pop(key(0))
            if kind=='sentinel':bad[key(c['keys'])]=value(c['keys'],1)
            if kind=='wrong-key':
                index=(first+c['batch_size'])%c['keys'];bad[key(index)]=value(index,nonce)
            if kind=='bad-bytes':bad[key(first)]=bytes(128)
            if kind=='nonce-over-bound':bad[key(first)]=value(first,nonce+1)
            with self.assertRaises(ValueError,msg=kind):d.final_dataset(c,bad,resp)

    def test_frozen_role_bindings_without_build_or_runtime(self):
        args=types.SimpleNamespace(old_server_source=Path('/tmp/kv9-rpc-worker-pair'),
            old_server_build=Path('/tmp/kv9-rpc-pair-release-first'),new_server_source=Path('/tmp/kv9-wal-crc32-table'),
            new_server_build=Path('/tmp/kv9-wal-crc32-release-first'),client_source=SOURCE,redis_client_source=SOURCE,
            client_build=Path('/tmp/kv9-point-write-v3-release-first/native'),
            redis_client_build=Path('/tmp/kv9-point-write-v3-release-first/redis'),
            expected_client_revision='0be806d9671e2c50701a64aa7889c8859b7648ba')
        roles=d.verify_inputs(args);cargo=d.verify_client_cargo(args,native)
        self.assertEqual(set(roles),{'old','new','client','redis'})
        self.assertEqual(roles['client']['source'],roles['redis']['source'])
        self.assertEqual([cargo[k]['features'] for k in ('client','redis')],[[],[]])


if __name__ == '__main__':
    unittest.main()
