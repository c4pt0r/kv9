#!/usr/bin/env python3
"""Pure independent v3 auditor contracts; never execute a fixture or auditor main."""
import ast
import copy
import json
from pathlib import Path
import unittest

SOURCE = Path(__file__).with_name('audit.py')
tree = ast.parse(SOURCE.read_text(), filename=str(SOURCE))
constants = {'SOURCE','CLIENT_REV','OLD_REV','NEW_REV','REDIS_REV','REDIS_SOURCE','PINS',
             'PROTOCOL_ID','CONCURRENCIES','WORKLOADS','ROLES','ROLE_PATHS','SERVER_MANIFESTS','STORAGE_GUARDS'}
functions = {'require','same_json','expected_cohorts','shared_config','expected_descriptor',
             'validate_matrix_protocol','validate_requested_config','paired_cases','histogram',
             'mix','value','dataset_check','validate_phase_apis','operation_statistics','validate_storage'}
nodes = [node for node in tree.body
         if (isinstance(node, ast.FunctionDef) and node.name in functions)
         or (isinstance(node, ast.Assign) and len(node.targets)==1 and
             isinstance(node.targets[0],ast.Name) and node.targets[0].id in constants)]
audit = {'json':json,'Path':Path}
exec(compile(ast.Module(body=nodes,type_ignores=[]),str(SOURCE),'exec'),audit)

WORKLOAD_ROWS = [('point-r000',1,0),('batch64-r000',64,0)]


def config(repeat,workers,workload,server):
    _,batch,reads=next(row for row in WORKLOAD_ROWS if row[0]==workload)
    c=dict(run_id=f'p{repeat}{workers:04d}',seed=71,workers=workers,keys=4096,
           batch_size=batch,value_bytes=128,read_percent=reads,warmup_calls=128,
           measure_ms=10000,max_calls=10000000,load={'kind':'closed_loop'})
    if server:
        c.update(version=3,rpc_transport='tonic_stream',read_api='point_get' if batch==1 else 'batch_get',
                 write_api='point_put' if batch==1 else 'batch_put',
                 client=dict(version=1,peers=[dict(node_id=i,address=f'127.0.0.1:{20000+i}') for i in (1,2,3)],
                    keyspace_id=100,epoch_conf_ver=1,epoch_version=1,max_in_flight=workers,
                    max_attempts=6,deadline_ms=1500,retry_backoff_ms=5))
    else:
        c.update(version=3,read_api='get' if batch==1 else 'mget',write_api='set' if batch==1 else 'mset',
                 address='127.0.0.1:25000',deadline_ms=1500)
    return c


def matrix():
    rows=[]
    # Independently enumerate both physical orders. Do not call auditor or driver
    # descriptor functions to manufacture their own expected inputs.
    for repeat in (0,1):
        for workers in ((1,64) if repeat==0 else (64,1)):
            workloads=WORKLOAD_ROWS if repeat==0 else list(reversed(WORKLOAD_ROWS))
            for workload,batch,reads in workloads:
                for role in (('old','new','redis') if repeat==0 else ('redis','new','old')):
                    server=None if role=='redis' else role
                    c=config(repeat,workers,workload,server)
                    shared={k:v for k,v in c.items() if k not in
                            {'version','rpc_transport','read_api','write_api','client','address','deadline_ms'}}
                    rows.append(dict(repeat=repeat,arm=f'{role}-{workload}',server_role=server,
                        workload=workload,read_api=c['read_api'],write_api=c['write_api'],
                        mix={0:'write',50:'mixed',100:'read'}[reads],target='kv9' if server else 'redis-memory',shared=shared))
    return dict(version=2,protocol_id='kv9-owned-proposal-write-screen-c1-c64-v1',concurrency_points=[1,64],
                smoke=False,fixed_rates=None,attempts=[{} for _ in rows],cohort_inventory=rows,
                workload_cells=[[name.split('-r')[0],batch,reads] for name,batch,reads in WORKLOAD_ROWS],
                storage_guards={'preflight':{'tmpfs':32*1024**3,'retention':96*1024**3},
                                'runtime':{'tmpfs':16*1024**3,'retention':64*1024**3}})


def oracle_mix(n):
    mask=(1<<64)-1
    n=(n+0x9e3779b97f4a7c15)&mask
    n=((n^(n>>30))*0xbf58476d1ce4e5b9)&mask
    n=((n^(n>>27))*0x94d049bb133111eb)&mask
    return n^(n>>31)


def phase_report(c,native):
    report=dict(version=3,workload_model='bounded_native_api_performance' if native else 'bounded_redis_api_performance',
                measured_issued=3,metrics={})
    for phase in ['initialization','warmup','measurement','verification']:
        traffic=phase in ('warmup','measurement')
        labels=[c['read_api'].removeprefix('point_'),c['write_api'].removeprefix('point_')] if traffic else (
            ['batch_get','batch_put'] if native else ['mget','mset'])
        counts=[0,0]
        if phase=='warmup':
            reads=sum(oracle_mix(c['seed']^n)%100<c['read_percent'] for n in range(1,c['warmup_calls']+1))
            counts=[reads,c['warmup_calls']-reads]
        if phase=='measurement':
            counts={0:[0,3],50:[2,1],100:[3,0]}[c['read_percent']]
        report['metrics'][phase]=dict(operations=labels,statistics=[dict(populations=[dict(calls=n)]) for n in counts])
    return report


def population(samples,items=1):
    buckets=[0]*3776
    for n in samples:
        shift=max(0,n.bit_length()-7)
        index=n if n<64 else 64+64*shift+(n>>shift)-64
        buckets[index]+=1
    return dict(calls=len(samples),input_items=len(samples)*items,completed_before_cutoff=len(samples),
                whole_call={'raw':dict(count=len(samples),sum_ns=sum(samples),buckets=buckets)})


def dataset_fixture():
    c=dict(run_id='data',keys=4,value_bytes=16,warmup_calls=128,max_calls=1000,seed=71,batch_size=1,read_percent=50)
    values={f'data:{i:016x}'.encode():i.to_bytes(8,'big')+bytes(8) for i in range(5)}
    nonce=next(n for n in range(1,1000) if oracle_mix(c['seed']^n)%100>=50)
    key=oracle_mix(c['seed']^nonce^0xc6275c213842315b)%4
    values[f'data:{key:016x}'.encode()]=key.to_bytes(8,'big')+nonce.to_bytes(8,'big')
    return c,values,nonce,key


def storage(phase='runtime'):
    available={'tmpfs':32*1024**3,'retention':96*1024**3}
    return dict(started_unix_ns=10,completed_unix_ns=20,filesystems={name:dict(
        path='/dev/shm' if name=='tmpfs' else '/owned/cohort',device=1 if name=='tmpfs' else 2,
        fragment_bytes=4096,blocks=available[name]//4096+100,free_blocks=available[name]//4096,
        available_blocks=available[name]//4096,available_bytes=available[name]) for name in available})


class AuditContract(unittest.TestCase):
    def test_syntax_without_runtime_import(self):
        compile(SOURCE.read_text(),str(SOURCE),'exec')
        self.assertEqual(set(audit)&{'ROOT','OUT','args','result'},set())

    def test_exact_24_entire_reverse_order_and_run_ids(self):
        m=matrix();audit['validate_matrix_protocol'](m)
        expected=[(d['repeat'],d['shared']['workers'],d['workload'],d['server_role']) for d in m['cohort_inventory']]
        self.assertEqual(audit['expected_cohorts'](),expected)
        self.assertEqual(len(expected),24)
        self.assertEqual([(d['repeat'],d['arm'],d['shared']['run_id']) for d in
                          [m['cohort_inventory'][i] for i in (0,6,12,18,23)]],
                         [(0,'old-point-r000','p00001'),(0,'old-point-r000','p00064'),
                          (1,'redis-batch64-r000','p10064'),(1,'redis-batch64-r000','p10001'),
                          (1,'old-point-r000','p10001')])

    def test_reject_partial_reverse_missing_duplicate_and_old_schedule(self):
        bads=[]
        m=matrix();m['cohort_inventory'][12:]=m['cohort_inventory'][18:]+m['cohort_inventory'][12:18];bads.append(m)
        m=matrix();m['cohort_inventory'][6:12]=copy.deepcopy(m['cohort_inventory'][:6]);bads.append(m)
        m=matrix();m['cohort_inventory']=m['cohort_inventory'][:12];m['attempts']=[{}]*12;bads.append(m)
        m=matrix();m['cohort_inventory'][12:]=copy.deepcopy(m['cohort_inventory'][:12]);bads.append(m)
        m=matrix();m['attempts']=[{}]*72;bads.append(m)
        for m in bads:
            with self.assertRaisesRegex(ValueError,'inventory/order'):audit['validate_matrix_protocol'](m)

    def test_reject_smoke_protocol_scope_and_wrong_timed_duration(self):
        for field,value in [('smoke',True),('version',1),('version',True),('protocol_id','other'),
                            ('concurrency_points',[True,64]),('fixed_rates',[1])]:
            m=matrix();m[field]=value
            with self.subTest(field=field),self.assertRaises(ValueError):audit['validate_matrix_protocol'](m)
        for duration in [2000,5000,30000,True]:
            m=matrix()
            for descriptor in m['cohort_inventory']:descriptor['shared']['measure_ms']=duration
            with self.assertRaisesRegex(ValueError,'inventory/order'):audit['validate_matrix_protocol'](m)

    def test_reject_relabelled_write_selector_and_workload(self):
        for field,value in [('write_api','batch_put'),('read_api','batch_get'),('workload','batch64-r000'),
                            ('shared',dict(matrix()['cohort_inventory'][0]['shared'],workers=True))]:
            m=matrix();m['cohort_inventory'][0][field]=value
            with self.assertRaisesRegex(ValueError,'inventory/order'):audit['validate_matrix_protocol'](m)

    def test_all_native_redis_v3_configs(self):
        for repeat in (0,1):
            for workers in (1,64):
                for workload,_,_ in WORKLOAD_ROWS:
                    for server in ('old','new',None):
                        audit['validate_requested_config'](config(repeat,workers,workload,server),repeat,workers,workload,server)

    def test_reject_changed_limits_and_v2_or_null_selector(self):
        for field,value in [('max_in_flight',64),('max_in_flight',True),('max_attempts',1),
                            ('deadline_ms',3000),('retry_backoff_ms',0),('epoch_version',2)]:
            c=config(0,1,'point-r000','old');c['client'][field]=value
            with self.assertRaisesRegex(ValueError,'exact request config'):audit['validate_requested_config'](c,0,1,'point-r000','old')
        for field,value in [('version',2),('write_api',None),('read_api','mget'),('write_api','mset'),
                            ('workers',64),('measure_ms',2000),('max_calls',100000),('read_percent',100),
                            ('run_id','p00064'),('value_bytes',64),('unexpected',1)]:
            c=config(0,1,'point-r000',None);c[field]=value
            with self.assertRaisesRegex(ValueError,'exact request config'):audit['validate_requested_config'](c,0,1,'point-r000',None)
        for repeat in (-1,2,True):
            with self.assertRaisesRegex(ValueError,'invalid repeat/concurrency'):audit['shared_config'](repeat,1,'point-r000')

    def test_pairing_never_crosses_workload_concurrency_repeat(self):
        cases=[dict(repeat=r,concurrency=c,workload=w,arm=role+'-'+w,marker=(r,c,w,role))
               for r in (0,1) for c in (1,64) for w,_,_ in WORKLOAD_ROWS for role in ('old','new')]
        for r in (0,1):
            for c in (1,64):
                for w,_,_ in WORKLOAD_ROWS:
                    old,new=audit['paired_cases'](cases,r,c,w)
                    self.assertEqual((old['marker'],new['marker']),((r,c,w,'old'),(r,c,w,'new')))
        with self.assertRaisesRegex(ValueError,'missing/duplicate'):audit['paired_cases'](cases[1:],0,1,'point-r000')
        with self.assertRaisesRegex(ValueError,'missing/duplicate'):audit['paired_cases'](cases+[cases[0]],0,1,'point-r000')

    def test_phase_api_labels_and_mixed_population(self):
        for workload,_,_ in WORKLOAD_ROWS:
            for server in ('old',None):
                c=config(0,1,workload,server);r=phase_report(c,server is not None)
                audit['validate_phase_apis'](r,c,server is not None)
                for phase in ['initialization','warmup','measurement','verification']:
                    bad=copy.deepcopy(r);bad['metrics'][phase]['operations'][1]='wrong'
                    with self.assertRaisesRegex(ValueError,'vocabulary'):audit['validate_phase_apis'](bad,c,server is not None)
                bad=copy.deepcopy(r);bad['version']=2
                with self.assertRaisesRegex(ValueError,'v3 workload'):audit['validate_phase_apis'](bad,c,server is not None)
        c=config(0,1,'point-r000','old');c['read_percent']=50;r=phase_report(c,True)
        r['metrics']['measurement']['statistics'][0]['populations'][0]['calls']=3
        r['metrics']['measurement']['statistics'][1]['populations'][0]['calls']=0
        with self.assertRaisesRegex(ValueError,'mixed measurement'):audit['validate_phase_apis'](r,c,True)

    def test_measurement_does_not_invent_exact_issued_nonce_mix(self):
        c=config(0,1,'point-r000','old');c['read_percent']=50;r=phase_report(c,True)
        # Both possible mixed populations remain admissible without an issued
        # nonce ledger. Warmup still has its exact sequential oracle.
        for calls in ([1,2],[2,1]):
            for op,n in zip(r['metrics']['measurement']['statistics'],calls):op['populations'][0]['calls']=n
            audit['validate_phase_apis'](r,c,True)
        r['metrics']['warmup']['statistics'][0]['populations'][0]['calls']+=1
        with self.assertRaisesRegex(ValueError,'warmup deterministic'):audit['validate_phase_apis'](r,c,True)

    def test_combined_histogram_weights_counts_and_empty_operation(self):
        combined=audit['histogram']([population([1]*99),population([1024])])
        self.assertEqual(combined['count'],100)
        self.assertEqual(combined['sum_ns'],1123)
        self.assertEqual(combined['mean_ns'],11.23)
        self.assertEqual(combined['p99'],dict(lower_ns=1,upper_ns=1))
        self.assertEqual(audit['histogram']([population([])]),dict(count=0,sum_ns=0,mean_ns=None,p50=None,p95=None,p99=None))

    def test_operation_rates_and_failure_population_are_preserved(self):
        empty=population([])
        metrics=dict(operations=['get','set'],outcomes=['success','unknown_write','read_failure'],
                     reasons=['success','deadline','io'],statistics=[])
        for populations,reasons in [([population([64,128]),empty,empty],[2,0,0]),
                                    ([empty,population([256]),empty],[0,1,0])]:
            metrics['statistics'].append(dict(populations=populations,reasons=reasons,
                command_attempts=sum(reasons),connection_attempts=0,connection_failures=0))
        rows=audit['operation_statistics'](metrics,1000000000,False)
        self.assertEqual(rows[0]['completed_calls_per_second'],2)
        self.assertEqual(rows[1]['outcomes']['unknown_write'],1)
        self.assertEqual(rows[1]['successful_calls_per_second'],0)
        self.assertIsNone(rows[1]['successful_whole_call_latency']['p99'])
        self.assertEqual(rows[1]['whole_call_latency']['count'],1)

    def test_mutable_dataset_budget_write_key_and_sentinel(self):
        c,data,nonce,key=dataset_fixture();checked=audit['dataset_check'](c,data)
        self.assertEqual(checked['changed_keys'],1)
        self.assertFalse(checked['exact_issued_nonce_set_checked'])
        for change in ['read_nonce','wrong_key','sentinel','budget','body','missing']:
            bad=copy.deepcopy(data);changed=copy.deepcopy(c)
            if change=='read_nonce':changed['read_percent']=100
            elif change=='wrong_key':
                wrong=(key+1)%4;bad[f'data:{wrong:016x}'.encode()]=wrong.to_bytes(8,'big')+nonce.to_bytes(8,'big')
            elif change=='sentinel':bad[b'data:0000000000000004']=(4).to_bytes(8,'big')+nonce.to_bytes(8,'big')
            elif change=='budget':changed.update(warmup_calls=0,max_calls=0)
            elif change=='body':bad[f'data:{key:016x}'.encode()]=bytes(8)+nonce.to_bytes(8,'big') if key else (1).to_bytes(8,'big')+nonce.to_bytes(8,'big')
            else:del bad[b'data:0000000000000004']
            with self.subTest(change=change),self.assertRaises((ValueError,KeyError)):audit['dataset_check'](changed,bad)

    def test_storage_threshold_arithmetic_identity_and_scope(self):
        for phase in ['preflight','runtime']:
            audit['validate_storage'](storage(),'/owned/cohort',phase)
        for change in ['below','arithmetic','device','path','missing','interval','bool']:
            s=storage();row=s['filesystems']['tmpfs']
            if change=='below':row.update(available_blocks=1,available_bytes=4096)
            elif change=='arithmetic':row['available_bytes']-=1
            elif change=='device':row['device']=3
            elif change=='path':row['path']='/wrong'
            elif change=='missing':del s['filesystems']['retention']
            elif change=='interval':s['completed_unix_ns']=9
            else:row['device']=True
            with self.subTest(change=change),self.assertRaises(ValueError):
                audit['validate_storage'](s,'/owned/cohort','runtime',{'tmpfs':1,'retention':2})
        m=matrix();m['storage_guards']['runtime']['tmpfs']=8*1024**3
        with self.assertRaisesRegex(ValueError,'storage guard'):audit['validate_matrix_protocol'](m)

    def test_exact_source_and_client_role_pins(self):
        self.assertEqual(audit['PINS']['old'],('ca0002c7f8e9ee6f595efcc9f4151085ccce87cb',
            'b33b5d302f901aac5cf6a95449b9bb5dd68473747e011220dbed7aed2e591d13'))
        self.assertEqual(audit['PINS']['new'],('71c9d996e1dcc0897da14be1886e669f0c3ea773',
            'b740e3d3c7ca1ad6cac2ed901d2c8a52b128c7c6456f95407f479bb61f414033'))
        self.assertEqual(audit['CLIENT_REV'],audit['REDIS_REV'])
        self.assertEqual(audit['CLIENT_REV'],'0be806d9671e2c50701a64aa7889c8859b7648ba')
        self.assertEqual(set(audit['ROLE_PATHS']),{'old','new','client','redis'})


if __name__=='__main__':
    unittest.main(verbosity=2)
