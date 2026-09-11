#!/usr/bin/env python3
"""Independent 24-cohort v3 write-only selection-screen readback; no workload execution."""
import argparse
import base64
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
from pathlib import Path
import sys
import subprocess
import stat
import time

if not __debug__:
    raise RuntimeError("audit requires PYTHONOPTIMIZE=0")
sys.dont_write_bytecode = True
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--run', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--expected-driver-sha256', required=True)
parser.add_argument('--root-session', type=int, required=True)
parser.add_argument('--root-exit-code', type=int, choices=(0,), required=True)
parser.add_argument('--successful-arms', type=int, choices=(24,), required=True)
args = parser.parse_args()
ROOT = args.run.resolve()
OUT = args.output.resolve()
if not OUT.is_relative_to(Path(__file__).parent) or OUT.exists():
    raise ValueError('audit output must be a new directory within the owned preparation')
OUT.mkdir()
SOURCE = Path('/tmp/kv9-point-write-measurement-v3')
CLIENT_REV = '0be806d9671e2c50701a64aa7889c8859b7648ba'
OLD_REV = 'ca0002c7f8e9ee6f595efcc9f4151085ccce87cb'
NEW_REV = '71c9d996e1dcc0897da14be1886e669f0c3ea773'
REDIS_REV = CLIENT_REV
REDIS_SOURCE = SOURCE
PINS = {
 'old': (OLD_REV, 'b33b5d302f901aac5cf6a95449b9bb5dd68473747e011220dbed7aed2e591d13'),
 'new': (NEW_REV, 'b740e3d3c7ca1ad6cac2ed901d2c8a52b128c7c6456f95407f479bb61f414033'),
 'client': (CLIENT_REV, '8da9af469f962a938027d1970141bbe4622f7d42b2b795f720e288fb3f8d5957'),
 'redis': (REDIS_REV, '5a8ac274b8cc6548a072f5305b04936a08cc4d1ba84d50150f675bab563049af'),
}
PROTOCOL_ID = 'kv9-owned-proposal-write-screen-c1-c64-v1'
CONCURRENCIES = (1, 64)
WORKLOADS = (('point-r000',1,0), ('batch64-r000',64,0))
ROLES = ('old', 'new', None)
ROLE_PATHS = {
 'old': ('/tmp/kv9-wal-crc32-table', '/tmp/kv9-wal-crc32-release-first', 'kv9'),
 'new': ('/tmp/kv9-owned-proposal-buffers', '/tmp/kv9-owned-proposal-release-first', 'kv9'),
 'client': ('/tmp/kv9-point-write-measurement-v3', '/tmp/kv9-point-write-v3-release-first/native', 'kv9-batch-benchmark'),
 'redis': ('/tmp/kv9-point-write-measurement-v3', '/tmp/kv9-point-write-v3-release-first/redis', 'kv9-redis-batch-reference'),
}
SERVER_MANIFESTS = {
 'old': '8c2ea115afc1ec8b6c82224dc6449d1d2439f5f27b5758e45090589752d186e8',
 'new': 'b3cd4c20bd5033bf3651ec9c2bbee008ecdee10a81b724f40a9e513fd91448b3',
}
STORAGE_GUARDS = {'preflight': {'tmpfs':32*1024**3,'retention':96*1024**3},
                  'runtime': {'tmpfs':16*1024**3,'retention':64*1024**3}}
records = {}
def require(value, message):
    if not value:
        raise ValueError(message)
def sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()
def read(path):
    path = Path(path)
    raw = path.read_bytes()
    records[str(path)] = dict(bytes=len(raw),sha256=hashlib.sha256(raw).hexdigest())
    return json.loads(raw)
def save(name, value):
    (OUT/name).write_text(json.dumps(value,indent=2)+'\n')

def same_json(actual, expected):
    # Equality must not silently accept bool in place of a protocol integer.
    return json.dumps(actual,sort_keys=True,separators=(',',':')) == json.dumps(expected,sort_keys=True,separators=(',',':'))

def expected_cohorts():
    forward = [(concurrency,workload,server) for concurrency in CONCURRENCIES
               for workload,_,_ in WORKLOADS for server in ROLES]
    return [(repeat,*row) for repeat in (0,1)
            for row in (forward if repeat==0 else reversed(forward))]

def shared_config(repeat, concurrency, workload):
    require(type(repeat) is int and repeat in (0,1) and
            type(concurrency) is int and concurrency in CONCURRENCIES, 'invalid repeat/concurrency')
    selected=[row for row in WORKLOADS if row[0]==workload]
    require(len(selected)==1, 'invalid workload')
    _,batch,reads=selected[0]
    return dict(run_id=f'p{repeat}{concurrency:04d}',seed=71,workers=concurrency,keys=4096,
                batch_size=batch,value_bytes=128,read_percent=reads,warmup_calls=128,
                measure_ms=10000,max_calls=10_000_000,load={'kind':'closed_loop'})

def expected_descriptor(repeat, concurrency, workload, server):
    require(server in ROLES, 'invalid server role')
    shared=shared_config(repeat,concurrency,workload)
    point=shared['batch_size']==1
    read_api,write_api=(('point_get','point_put') if point else ('batch_get','batch_put')) if server else (
        ('get','set') if point else ('mget','mset'))
    mix={0:'write',50:'mixed',100:'read'}[shared['read_percent']]
    return dict(repeat=repeat,arm=f'{server or "redis"}-{workload}',server_role=server,workload=workload,
                read_api=read_api,write_api=write_api,mix=mix,
                target='kv9' if server else 'redis-memory',shared=shared)

def validate_matrix_protocol(matrix):
    require(type(matrix['version']) is int and matrix['version']==2 and
            matrix['protocol_id']==PROTOCOL_ID and
            same_json(matrix['concurrency_points'],list(CONCURRENCIES)), 'c1/c64 protocol scope differs')
    require(matrix['smoke'] is False and matrix['fixed_rates'] is None, 'smoke/fixed-rate is not timed protocol')
    require(same_json(matrix['storage_guards'],STORAGE_GUARDS), 'storage guard thresholds differ')
    require(same_json(matrix['workload_cells'],[[name.partition('-r')[0],batch,reads]
                                              for name,batch,reads in WORKLOADS]), 'workload cells differ')
    expected=[expected_descriptor(*row) for row in expected_cohorts()]
    require(len(matrix['attempts'])==24 and same_json(matrix['cohort_inventory'],expected),
            'twenty-four-cohort inventory/order differs')

def validate_requested_config(config, repeat, concurrency, workload, server):
    descriptor=expected_descriptor(repeat,concurrency,workload,server)
    expected=dict(descriptor['shared'])
    if server is not None:
        client=config['client']
        expected.update(version=3,rpc_transport='tonic_stream',read_api=descriptor['read_api'],
                        write_api=descriptor['write_api'],
                        client=dict(version=1,peers=client['peers'],keyspace_id=client['keyspace_id'],
                                    epoch_conf_ver=1,epoch_version=1,max_in_flight=concurrency,
                                    max_attempts=6,deadline_ms=1500,retry_backoff_ms=5))
    else:
        expected.update(version=3,read_api=descriptor['read_api'],write_api=descriptor['write_api'],
                        address=config['address'],deadline_ms=1500)
    require(same_json(config,expected), 'independent exact request config differs')

def paired_cases(cases, repeat, concurrency, workload):
    result=[]
    for arm in ('old-'+workload,'new-'+workload):
        selected=[c for c in cases if c['repeat']==repeat and c['concurrency']==concurrency and
                  c['workload']==workload and c['arm']==arm]
        require(len(selected)==1, 'missing/duplicate concurrency-specific comparison case')
        result.append(selected[0])
    return tuple(result)

def histogram(populations):
    buckets = [0] * 3776
    count = total = 0
    for p in populations:
        raw = p['whole_call']['raw']
        count += raw['count']; total += raw['sum_ns']
        for i, n in enumerate(raw['buckets']):
            buckets[i] += n
    require(sum(buckets) == count, 'merged histogram count differs')
    def quantile(percent):
        if count==0:
            return None
        rank = (count * percent + 99) // 100
        seen = 0
        for i, n in enumerate(buckets):
            seen += n
            if seen >= rank:
                if i < 64:
                    return dict(lower_ns=i, upper_ns=i)
                shift = (i - 64) // 64
                lower = (64 + (i - 64) % 64) << shift
                return dict(lower_ns=lower, upper_ns=lower + (1 << shift) - 1)
        raise ValueError('missing quantile rank')
    return dict(count=count, sum_ns=total, mean_ns=total / count if count else None,
                **{f'p{p}': quantile(p) for p in (50, 95, 99)})

def mix(n):
    n = (n + 0x9e3779b97f4a7c15) % 2**64
    n = ((n ^ (n >> 30)) * 0xbf58476d1ce4e5b9) % 2**64
    n = ((n ^ (n >> 27)) * 0x94d049bb133111eb) % 2**64
    return n ^ (n >> 31)

def value(seed, size, index, nonce):
    result = index.to_bytes(8, 'big') + nonce.to_bytes(8, 'big')
    for word in range((size - 16 + 7) // 8):
        result += mix(seed ^ mix(index) ^ mix(nonce) ^ word).to_bytes(8, 'big')
    return result[:size]

def dataset_check(config, dataset):
    require(len(dataset) == config['keys'] + 1, 'final dataset cardinality differs')
    changed=0
    for index in range(config['keys'] + 1):
        actual = dataset[f'{config["run_id"]}:{index:016x}'.encode()]
        require(actual is not None and len(actual) == config['value_bytes'], 'final value length/missing differs')
        nonce = int.from_bytes(actual[8:16], 'big')
        require(nonce <= config['warmup_calls'] + config['max_calls'] and actual == value(config['seed'], config['value_bytes'], index, nonce), 'independent value body differs')
        require(index != config['keys'] or nonce == 0, 'sentinel changed')
        if nonce:
            require(mix(config['seed'] ^ nonce) % 100 >= config['read_percent'], 'final nonce selected a read')
            first=mix(config['seed'] ^ nonce ^ 0xc6275c213842315b) % config['keys']
            require((index-first) % config['keys'] < config['batch_size'], 'final write nonce/key membership differs')
            changed+=1
    return dict(keys=len(dataset),changed_keys=changed,all_nonce_zero=changed==0,sentinel_unchanged=True,
                deterministic_values_valid=True,deterministic_write_key_membership_checked=True,
                maximum_configured_nonce=config['warmup_calls']+config['max_calls'],
                exact_issued_nonce_set_checked=False,full_history_checked=False)

def validate_phase_apis(report, config, is_native):
    require(report['version']==config['version']==3 and report['workload_model']==
            ('bounded_native_api_performance' if is_native else 'bounded_redis_api_performance'),
            'genuine v3 workload identity differs')
    for phase in ('initialization','warmup','measurement','verification'):
        metrics=report['metrics'][phase]
        traffic=phase in ('warmup','measurement')
        expected=([config['read_api'].removeprefix('point_'),config['write_api'].removeprefix('point_')]
                  if traffic else ['batch_get','batch_put'] if is_native else ['mget','mset'])
        require(metrics['operations']==expected,'selected phase API vocabulary differs')
        calls=[sum(p['calls'] for p in op['populations']) for op in metrics['statistics']]
        if phase=='warmup':
            reads=sum(mix(config['seed'] ^ nonce)%100<config['read_percent']
                      for nonce in range(1,config['warmup_calls']+1))
            require(calls==[reads,config['warmup_calls']-reads],'warmup deterministic operation mix differs')
        elif phase=='measurement':
            require(sum(calls)==report['measured_issued']>0,'measurement population differs')
            if config['read_percent'] in (0,100):
                require(calls[0 if config['read_percent']==0 else 1]==0,'pure measurement mix differs')
            else:
                require(all(calls),'mixed measurement omitted an operation')
            # Allocation precedes the final cutoff check. Aggregate counts do
            # not identify a contiguous issued nonce set; do not invent one.

def operation_statistics(metrics, elapsed_ns, is_native):
    rows=[]
    for name,op in zip(metrics['operations'],metrics['statistics']):
        populations=op['populations'];calls=sum(p['calls'] for p in populations)
        items=sum(p['input_items'] for p in populations)
        attempts=sum(op['attempt_reasons']) if is_native else op['command_attempts']
        rows.append(dict(operation=name,calls=calls,input_items=items,
            completed_calls_per_second=calls*1e9/elapsed_ns,completed_input_items_per_second=items*1e9/elapsed_ns,
            successful_calls_per_second=populations[0]['calls']*1e9/elapsed_ns,
            successful_input_items_per_second=populations[0]['input_items']*1e9/elapsed_ns,
            attempts=attempts,outcomes={outcome:p['calls'] for outcome,p in zip(metrics['outcomes'],populations)},
            reasons={reason:n for reason,n in zip(metrics['reasons'],op['reasons'])},
            attempt_reasons={reason:n for reason,n in zip(metrics['reasons'],op['attempt_reasons'])} if is_native else None,
            terminal_rpc_codes=op['rpc_codes'] if is_native else None,
            connection_attempts=None if is_native else op['connection_attempts'],
            connection_failures=None if is_native else op['connection_failures'],
            populations=[dict(outcome=outcome,calls=p['calls'],input_items=p['input_items'],
                completed_before_cutoff=p['completed_before_cutoff'],whole_call_latency=histogram([p]))
                for outcome,p in zip(metrics['outcomes'],populations)],
            whole_call_latency=histogram(populations),successful_whole_call_latency=histogram([populations[0]])))
    return rows

def validate_storage(observation, directory, phase, devices=None):
    require(type(observation['started_unix_ns']) is int and type(observation['completed_unix_ns']) is int and
            0<observation['started_unix_ns']<=observation['completed_unix_ns'], 'invalid storage observation interval')
    require(set(observation['filesystems'])=={'tmpfs','retention'}, 'storage filesystem inventory differs')
    result={}
    for name,path in (('tmpfs','/dev/shm'),('retention',str(directory))):
        row=observation['filesystems'][name]
        require(set(row)=={'path','device','fragment_bytes','blocks','free_blocks','available_blocks','available_bytes'} and
                row['path']==path and all(type(row[k]) is int for k in row if k!='path'), 'storage record schema differs')
        require(row['device']>0 and row['fragment_bytes']>0 and
                0<=row['available_blocks']<=row['free_blocks']<=row['blocks'] and
                row['available_bytes']==row['available_blocks']*row['fragment_bytes'], 'invalid statvfs arithmetic')
        require(row['available_bytes']>=STORAGE_GUARDS[phase][name], 'storage guard intervened')
        if devices is not None:
            require(row['device']==devices[name], 'storage device identity changed')
        result[name]=row['device']
    return result

ZERO = ('public_rpc_in_flight','public_rpc_queued','public_rpc_running','public_rpc_encoded_bytes',
        'raft_async_apply_queued','raft_async_apply_in_flight','raft_async_read_queued','raft_async_read_active',
        'raft_async_read_in_flight','raft_async_read_active_groups')
def qualify(s):
    return s['bootstrap_state'] == 'Serving' and s['fatal'] == '' and all(s[k] == '0' for k in ZERO) and s['raft_async_apply_stopped'] == s['raft_async_read_stopped'] == 'false' and s['applied_term'] == s['driver_applied_term'] and s['applied_index'] == s['driver_applied_index']

def retention_file(job):
    path, entry = job
    require(path.is_file() and not path.is_symlink() and path.stat().st_size == entry['bytes'], 'retained file identity/size differs: ' + str(path))
    require(sha(path) == entry['sha256'], 'retained data hash differs: ' + str(path))
    return str(path), entry


result = dict(complete=False, matched_diagnostic_accepted=False, cases=[],
              scope='Two-repeat 10-second v3 point PUT/BatchPut(64) c1/c64 shared-host tmpfs write-only selection screen; not full-workload acceptance; no sustained-capacity, equal-durability, exact issued nonce ledger or full-history claim',
              parent_confirmed_session=args.root_session, parent_confirmed_exit_code=args.root_exit_code,
              expected_successful_arms=args.successful_arms, protocol_id=PROTOCOL_ID,
              concurrency_points=list(CONCURRENCIES), started_unix_ns=time.time_ns())
try:
    matrix=read(ROOT/'matrix.json')
    validate_matrix_protocol(matrix)
    require(matrix['smoke'] is False and matrix['storage']=='tmpfs' and matrix['client_revision']==CLIENT_REV,
            'wrong fixture or source scope')
    require(matrix['complete'] is (args.root_exit_code==0), 'original fixture verdict differs from parent terminal confirmation')
    require(args.successful_arms==24 and args.root_exit_code==0, 'expected complete diagnostic scope differs')
    require(sha(ROOT/'driver.py')==args.expected_driver_sha256, 'executed driver snapshot differs')
    require(matrix['placement']==dict(client_cpus=[0,1],server_cpus=[2,3,4,5],
                                     separated=True,exclusive_host=False), 'diagnostic CPU scope differs')
    outer=ROOT.parent
    final=read(outer/'summary.json');before=read(outer/'before.json');after=read(outer/'after.json')
    isolation=read(outer/'isolation.json');invocation=read(outer/'invocation.json')
    require(final['complete'] is True and final['restoration_complete'] is True and final['child_exit_code']==0 and
            not final['child_cleanup_errors'] and not final.get('forced_group_cleanup') and
            not final['deferred_cleanup_signals'] and not after['errors'],'outer terminal/cleanup/restoration differs')
    require(isolation['complete'] is True and isolation['exclusive_host'] is False and
            isolation['measured_client_cpus']==[0,1] and isolation['measured_server_cpus']==[2,3,4,5] and
            isolation['driver_sha256']==args.expected_driver_sha256,'outer isolation scope differs')
    require(matrix['isolation_snapshot']==isolation and sha(outer/'isolation.json')==matrix['isolation_snapshot_sha256'] and
            sha(ROOT/'isolation-snapshot.json')==sha(outer/'isolation.json'),'inner/outer snapshot binding differs')
    expected_argv=['/usr/bin/python3','/tmp/kv9-owned-proposal-screen-preparation/matched-driver.py',
                   '--output',str(ROOT),'--isolation-snapshot',str(outer/'isolation.json'),
                   '--client-source','/tmp/kv9-point-write-measurement-v3',
                   '--client-build','/tmp/kv9-point-write-v3-release-first/native','--expected-client-revision',CLIENT_REV,
                   '--old-server-source','/tmp/kv9-wal-crc32-table',
                   '--old-server-build','/tmp/kv9-wal-crc32-release-first',
                   '--new-server-source','/tmp/kv9-owned-proposal-buffers',
                   '--new-server-build','/tmp/kv9-owned-proposal-release-first',
                   '--redis-client-source','/tmp/kv9-point-write-measurement-v3',
                   '--redis-client-build','/tmp/kv9-point-write-v3-release-first/redis','--storage','tmpfs']
    require(invocation['argv']==expected_argv and invocation['wrapper_sha256']==
            'e922939e675d722fe36c3b0890d31d6329be810dd7567b2e369643d10e2137fb' and
            invocation['driver_arguments_sha256']=='19eb6e0377ded7004dfc9ed8ba305a9a395a75443a93f3e2fc35ebaacc2bbfb2',
            'reviewed wrapper or explicit client arguments differ')
    expected_containers={
      'kv9-chaos-ci-p0-20260908-control-plane':'ae9b27a77d14f295e35e112b31f5dce62ed6fd2d86a90e7b2a69b52798e17582',
      'kv9-chaos-control-plane':'d31317fe14d1d3344caa91ed45b4246e5e949d99b25cec5e28d9c6695fa43e8c',
      'kv9-minio-dev':'4b05c35453b87a409e7dd8a4dc935c782e282eb42cf14dd8c1ea6771efc0e042'}
    require(set(before['containers'])==set(after['containers'])==set(isolation['containers'])==set(expected_containers),
            'owned container scope differs')
    for name,cid in expected_containers.items():
        original=before['containers'][name];restored=after['containers'][name];isolated=read(outer/(name+'-isolated.json'))
        require(original['id']==cid and restored==original and original['configured_cpus']==original['effective_cpus']=='0-31',
                'container exact configured/effective restoration differs')
        for observed in (isolated,isolation['containers'][name]):
            require(all(observed[k]==original[k]for k in ('name','id','pid','start_ticks','started','cgroup')) and
                    observed['configured_cpus']==observed['effective_cpus']=='6-15,22-31','isolated container identity/cpuset differs')
        require(isolated['descendants']['tasks'] and all(set(task['cpus'])<=set(range(6,16))|set(range(22,32))
                for task in isolated['descendants']['tasks']),'owned background descendant affinity escaped')
    require(set(before['clusters'])==set(after['clusters']),'cluster inventory set differs')
    for name,initial in before['clusters'].items():
        final_cluster=after['clusters'][name]
        require(initial['namespaces']==final_cluster['namespaces'],'historical namespace UID changed')
        require([(f['kind'],f['metadata']['uid'])for f in initial['faults']]==
                [(f['kind'],f['metadata']['uid'])for f in final_cluster['faults']],'historical fault identity changed')
        for inventory in (initial,final_cluster):
            require(all(f['kind']=='PodChaos' and f['spec']['action']=='pod-kill' and
                        f['metadata']['uid']=='c3ededa7-0746-4864-bd95-4425ceb3d663'for f in inventory['faults']),
                    'unexpected fault in retained timing inventory')
            require(all(all(c.get('command')==['sleep','3600'] and not c.get('args')for c in p['spec']['containers'])
                        for p in inventory['project_running_pods']),'project workload was active')
    result['outer_restoration']=dict(accepted=True,configured_and_effective_before_after='0-31',
                                     isolated='6-15,22-31',owned_containers=3,namespace_maps_preserved=True,
                                     original_summary_sha256=sha(outer/'summary.json'))
    source_files=0
    require(set(matrix['role_bindings'])==set(PINS)==set(ROLE_PATHS), 'role inventory differs')
    for role,binding in matrix['role_bindings'].items():
        revision,binary=PINS[role]
        expected_source,expected_build,expected_binary=ROLE_PATHS[role]
        require(binding['source']['path']==expected_source and binding['build_directory']==expected_build and
                binding['binary']==str(Path(expected_build)/expected_binary), 'role source/build path differs')
        if role in SERVER_MANIFESTS:
            require(sha(Path(expected_build)/'build.json')==SERVER_MANIFESTS[role], 'server release manifest differs')
        require(binding['source']['revision']==revision and binding['binary_sha256']==binary and sha(binding['binary'])==binary,
                'role executable/revision differs')
        source=Path(binding['source']['path'])
        require(subprocess.check_output(['git','rev-parse','HEAD'],cwd=source,text=True).strip()==revision and
                not subprocess.check_output(['git','status','--porcelain','--untracked-files=all'],cwd=source),
                'role source is not clean and frozen')
        if role in ('old','new'):
            build=Path(binding['build_directory']);cargo=build/'kv9-cargo.jsonl'
            rows=[json.loads(line)for line in cargo.read_text().splitlines()]
            require(rows[-1].get('reason')=='build-finished' and rows[-1]['success'] is True and
                    sum(row.get('reason')=='build-finished'for row in rows)==1,'server Cargo completion differs')
            for name in ('kv9','kv9_engine','kv9_raft','kv9_server'):
                selected=[row for row in rows if row.get('reason')=='compiler-artifact' and row.get('target',{}).get('name')==name]
                require(len(selected)==1 and selected[0]==binding['cargo_artifacts'][name] and
                        selected[0]['features']==[] and selected[0]['profile']['opt_level']=='3' and
                        selected[0]['profile']['test'] is False,'server Cargo identity/features differ')
        sources=binding['source']['sources']
        require(hashlib.sha256(json.dumps(sources,sort_keys=True,separators=(',',':')).encode()).hexdigest()==binding['source']['source_tree_sha256'],
                'role source inventory hash differs')
        for p,h in sources.items():
            require(sha(Path(binding['source']['path'])/p)==h,'role source changed: '+p)
            source_files+=1
        for number,(path,h) in enumerate(binding['evidence'].items()):
            require(sha(path)==h and sha(ROOT/'retained-inputs'/role/f'{number:02d}-{Path(path).name}')==h,
                    'original/retained build evidence differs')
    for number,(path,h) in enumerate(matrix['helper_hashes'].items()):
        require(sha(ROOT/'retained-inputs/helpers'/f'{number:02d}-{Path(path).name}')==h,'executed helper snapshot differs')
        if Path(path).is_relative_to(SOURCE) or Path(path).is_relative_to(REDIS_SOURCE):
            require(sha(path)==h,'validator/helper source changed')
    require(matrix['helper_hashes'][str(Path('/tmp/kv9-owned-proposal-screen-preparation/matched-driver.py'))]==args.expected_driver_sha256,
            'driver is not the retained helper identity')
    sys.path.insert(0,str(SOURCE/'scripts'))
    import batch_benchmark_report as native
    import importlib.util
    redis_spec=importlib.util.spec_from_file_location('independent_redis_get_report', REDIS_SOURCE/'scripts/redis_batch_report.py')
    redis=importlib.util.module_from_spec(redis_spec)
    redis_spec.loader.exec_module(redis)
    for role,reference in (('client',False),('redis',True)):
        build=Path(matrix['role_bindings'][role]['build_directory'])
        _,b,features=native.build_check(build,build,PINS[role][0],reference=reference)
        require(features==[] and b['profile']=='release','workload Cargo identity differs')
    expected=expected_cohorts()
    lifetimes=set();retained_files=retained_bytes=resource_count=drains=writer_bindings=0
    configs={}
    for ordinal,(attempt,(repeat,concurrency,workload,server)) in enumerate(zip(matrix['attempts'],expected)):
        shape=expected_descriptor(repeat,concurrency,workload,server)
        shared=shape['shared'];arm=shape['arm'];api=shape['read_api'];write_api=shape['write_api']
        d=Path(attempt['directory']);desc=attempt['descriptor'];is_native=server is not None
        require(d==ROOT/f'{ordinal:03d}-{arm}-p{repeat}{concurrency:04d}' and attempt['index']==ordinal and desc==matrix['cohort_inventory'][ordinal],
                'cohort path/order differs')
        require(same_json(desc,shape),
                'API shape or common cohort config differs')
        summary=read(d/'summary.json');cleanup=read(d/'cleanup.json')
        require(not cleanup.get('errors') and not summary.get('cleanup_errors') and not summary.get('cleanup_failure'),
                'owned cleanup failed')
        identities={}
        for child in cleanup['children']:
            identity=child['identity'];pid=identity['pid']
            require(child['exit_code'] is not None and child.get('absent',child.get('pid_absent_after_reap')) and
                    not Path('/proc',str(pid)).exists(),'owned lifetime remains')
            life=(pid,identity['start_ticks'],identity['boot_id']);require(life not in lifetimes,'duplicate owned lifetime');lifetimes.add(life)
            identities[pid]=identity
        require(attempt['complete'] and summary['complete'], 'expected successful arm incomplete')
        c=read(d/'requested-config.json');r=read(d/'run/report.json');configs[(repeat,concurrency,arm)]=c
        validate_requested_config(c,repeat,concurrency,workload,server)
        require(all(c[k]==v for k,v in shared.items()),'requested shared config differs')
        require(sha(d/'run/report.json')==attempt['report_sha256']==summary['report_sha256'],'original report changed')
        role='client' if is_native else 'redis';build=Path(matrix['role_bindings'][role]['build_directory'])
        validated=(native if is_native else redis).validate(d/'run',build,d/'requested-config.json',PINS[role][0],require_timing=True)
        require(validated['accepted'] and r['stop_reason']=='duration' and r['dropped_slots']==0,'report incomplete/capped')
        validate_phase_apis(r,c,is_native)
        require(read(d/'client-exit.json')['exit_code']==0,'actual client terminal status differs')
        for child in cleanup['children']:
            is_client=child['identity']['pid']==r['process_id']
            require(child['exit_code']==(0 if is_client or not is_native else -9),'owned exit status differs')
        client_pid=r['process_id'];require(client_pid in identities and len(identities)==(4 if is_native else 2),'owned client/backend set differs')
        for pid,identity in identities.items():
            mask=[0,1] if pid==client_pid else [2,3,4,5]
            require(identity.get('cpu_affinity',identity.get('affinity'))==mask,'owned startup mask differs')
            if is_native:
                expected_sha=PINS['client' if pid==client_pid else server][1]
                require(identity['executable_sha256']==expected_sha,'executing native role differs')
            else:
                expected_path=matrix['role_bindings']['redis']['binary'] if pid==client_pid else '/usr/bin/redis-server'
                require(identity['command_line'][0]==expected_path,'Redis original argv-zero identity differs')
                require(sha(identity['executable'])==(PINS['redis'][1] if pid==client_pid else matrix['redis_server_sha256']),
                        'Redis executable bytes differ')
        samples=read(d/'resource-samples.json');coverage=read(d/'resource-coverage.json')
        begin=r['measurement_start_unix_ns'];end=begin+r['cohort_elapsed_ns']
        inside=[sample for sample in samples if begin<=sample['unix_ns']<=end]
        require(len(inside)==coverage['samples']>=10 and inside[0]['unix_ns']-begin==coverage['first_gap_ns']<=150_000_000 and
                end-inside[-1]['unix_ns']==coverage['last_gap_ns']<=150_000_000,'diagnostic resource coverage differs')
        for sample in inside:
            require({x['pid']for x in sample['processes'].values()}==set(identities),'sample owned process set differs')
            for name,item in sample['processes'].items():
                require(item['start_ticks']==identities[item['pid']]['start_ticks'],'sample lifetime differs')
                mask=[0,1] if name=='client' else [2,3,4,5];placement=item['thread_placement']
                require(item['cpu_affinity']==placement['process']==placement['expected']==mask and placement['threads'] and
                        all(m==mask for m in placement['threads'].values()),'sample thread placement differs')
        resource_count+=len(inside)
        memory = {}
        for name in inside[0]['processes']:
            if name=='client': continue
            observations=[sample['processes'][name] for sample in inside]
            require(all(type(x[k])is int and x[k]>0 for x in observations for k in ('rss_bytes','peak_rss_bytes')),
                    'missing/invalid sampled RSS/HWM')
            memory[name]=dict(pid=observations[0]['pid'], start_ticks=observations[0]['start_ticks'],
                samples=len(observations), rss_first_bytes=observations[0]['rss_bytes'],
                rss_last_bytes=observations[-1]['rss_bytes'], rss_min_bytes=min(x['rss_bytes']for x in observations),
                rss_max_bytes=max(x['rss_bytes']for x in observations),
                rss_mean_bytes=sum(x['rss_bytes']for x in observations)/len(observations),
                hwm_first_bytes=observations[0]['peak_rss_bytes'], hwm_last_bytes=observations[-1]['peak_rss_bytes'],
                hwm_max_bytes=max(x['peak_rss_bytes']for x in observations),
                scope='Sampled RSS during measurement; HWM is process-lifetime high water including setup, not interval-only peak')
        host=read(d/'host-resource-samples.json');require(host and all('proc_stat'in x and 'background'in x for x in host),'host observations absent')
        preflight=read(d/'storage-preflight.json')
        devices=validate_storage(preflight,d,'preflight')
        require(preflight['completed_unix_ns']<begin and not (d/'storage-guard-intervention.json').exists(),
                'storage preflight overlaps measurement or a guard intervened')
        storage=read(d/'storage-resource-samples.json')
        callbacks=[row for row in read(d/'sample-calls.json') if row['pid']==client_pid]
        require(storage and len(storage)==len(callbacks), 'storage observation missing from client callback')
        storage_by_callback={}
        for i,(observation,callback) in enumerate(zip(storage,callbacks)):
            validate_storage(observation,d,'runtime',devices)
            require(callback['observed_unix_ns']<=observation['started_unix_ns']<=observation['completed_unix_ns']<=
                    read(d/'client-exit.json')['observed_unix_ns'], 'storage callback interval differs')
            if i+1<len(callbacks):
                require(observation['completed_unix_ns']<=callbacks[i+1]['observed_unix_ns'], 'storage callbacks reordered')
            key=(callback['pid'],callback['observed_monotonic_ns'])
            require(key not in storage_by_callback,'duplicate client storage observation')
            storage_by_callback[key]=observation
        require(all((sample['processes']['client']['pid'],sample['processes']['client']['observed_monotonic_ns'])
                    in storage_by_callback for sample in inside),'measured client callback lacks storage observation')
        storage_summary=dict(observations=len(storage),measured_callbacks=len(inside),devices=devices,
            minimum_available_bytes={name:min(row['filesystems'][name]['available_bytes'] for row in storage)
                                     for name in ('tmpfs','retention')},
            scope='Observed free-space operational guards; call caps and smoke growth do not prove a worst-case storage bound')
        if is_native:
            # Bind the already captured endpoint envelopes for a separate
            # downstream stage reader. This is byte inventory, not acceptance
            # of endpoint metric semantics or an additive latency partition.
            for phase in ('before','after'):
                for name in ('metrics','status','resources'):
                    read(d/f'{phase}-{name}.json')
            allocator_records = {}
            for label in ('before', 'post-client'):
                provenance = read(d/(label+'-allocator-provenance.json'))
                require(provenance['complete'] is True and provenance['label']==label and
                        set(provenance['voters'])=={'1','2','3'} and
                        provenance['started_unix_ns'] <= provenance['ended_unix_ns'], 'incomplete allocator observation')
                if label=='before':
                    require(provenance['ended_unix_ns'] < begin, 'allocator preflight overlaps timing')
                else:
                    require(provenance['started_unix_ns'] >= read(d/'client-exit.json')['observed_unix_ns'] >= end,
                            'allocator after observation precedes client exit')
                for node, sample in provenance['voters'].items():
                    identity=identities[sample['pid']]
                    require(sample['pid']!=client_pid and sample['start_ticks']==identity['start_ticks'] and
                            sample['boot_id']==sample['boot_after']==identity['boot_id'], 'allocator process identity differs')
                    for raw in (sample['stat_before'],sample['stat_after']):
                        require(int(raw.split(' ',1)[0])==sample['pid'] and
                                int(raw.rsplit(')',1)[1].split()[19])==identity['start_ticks'], 'allocator raw stat identity differs')
                    require(sample['checked_environment_names']==['_RJEM_MALLOC_CONF','MALLOC_CONF','LD_PRELOAD','LD_AUDIT','TOKIO_WORKER_THREADS'] and
                            sample['present_environment_names']==[], 'allocator environment override present/unchecked')
                    config=sample['configuration_file']
                    require(config['path']==f'/proc/{sample["pid"]}/root/etc/_rjem_malloc.conf' and
                            type(config['exists'])is bool and config['symlink']is False, 'allocator file observation differs')
                    if config['exists']:
                        require(type(config['mode'])is int and not stat.S_ISLNK(config['mode']), 'allocator configuration symlink present')
                allocator_records[label]=provenance
            require(allocator_records['before']['voters'].keys()==allocator_records['post-client']['voters'].keys(),
                    'allocator endpoint voter set differs')
            before=read(d/'listener-before.json');after=read(d/'listener-after.json')
            voters=read(d/'fixture/tmpfs-voters.json')['voters'];require(len(voters)==3,'three voter mount records missing')
            for node,listener in before.items():
                pid=listener['pid'];identity=identities[pid];later=after[node]
                require(all(provenance['voters'][node]['pid']==pid for provenance in allocator_records.values()),
                        'allocator observer voter/listener mapping differs')
                require(pid==later['pid'] and listener['advertised_endpoint']==later['advertised_endpoint'] and
                        listener['listener_inodes']==later['listener_inodes'] and listener['experimental_environment_absent'] and
                        later['experimental_environment_absent'],'ordinary listener binding differs')
                matches=[v for v in voters if v['pid']==pid];require(len(matches)==1,'voter mount owner differs');v=matches[0]
                require(v['mount']['filesystems'][0]['fstype']=='tmpfs' and v['executable_sha256']==identity['executable_sha256'] and
                        v['logical_data']==str(d/'fixture/data'/('n'+node)),'voter mount/source differs')
                require(v['command_line'][v['command_line'].index('--data-dir')+1]==v['logical_data'],'voter data path differs')
                writer_bindings+=1
            for label in ('before','post-client','post-readback'):
                drain=read(d/(label+'-fresh-drain.json'))
                require(0<=drain['completed_unix_ns']-drain['started_unix_ns']<=20_000_000_000,'fresh drain exceeded bound')
                require(set(drain['baseline'])==set(drain['first_advances'])==set(drain['final'])=={'1','2','3'},'drain voter set differs')
                for node,baseline in drain['baseline'].items():
                    first=drain['first_advances'][node]['status'];last=drain['final'][node];identity=identities[int(baseline['pid'])]
                    require(qualify(first) and qualify(last),'first/final drain was not empty and serving')
                    for state in (baseline,first,last):
                        require(int(state['pid'])==identity['pid'] and int(state['process_start_ticks'])==identity['start_ticks'] and
                                state['process_boot_id']==identity['boot_id'],'drain writer identity differs')
                    require(int(baseline['metrics_export_successes'])<int(first['metrics_export_successes'])<int(last['metrics_export_successes']),
                            'drain exports not strictly newer')
                require(len({(x['applied_term'],x['applied_index'])for x in drain['final'].values()})==1,'drain final positions differ')
                drains+=1
            dataset={}
            for page in sorted(d.glob('readback-*.txt')):
                raw=page.read_bytes();records[str(page)]=dict(bytes=len(raw),sha256=hashlib.sha256(raw).hexdigest())
                lines=raw.decode().splitlines();require(lines[-1]==f'count={len(lines)-1}','scan count differs')
                for line in lines[:-1]:
                    fields=dict(part.split('=',1)for part in line.split());key=bytes.fromhex(fields['key_hex'])
                    require(key not in dataset,'duplicate final scan key');dataset[key]=bytes.fromhex(fields['value_hex'])
            retention=read(d/'fixture/tmpfs-retention.json')
            require(retention['complete'] and retention['children_exited'] and retention['scratch_removed'] and
                    not Path(retention['original_scratch']).exists(),'owned tmpfs cleanup incomplete')
            retained=Path(retention['retained_data']);require(retained==d/'fixture/data' and retained.is_dir() and not retained.is_symlink(),'retained directory differs')
            require({str(p.relative_to(retained))for p in retained.rglob('*')if p.is_file()}==set(retention['files']),'retained member set differs')
            with ThreadPoolExecutor(max_workers=2) as pool:
                for path,entry in pool.map(retention_file,[(retained/name,entry)for name,entry in retention['files'].items()]):
                    records[path]=entry;retained_files+=1;retained_bytes+=entry['bytes']
        else:
            for label in ('before','after'):
                settings=read(d/(label+'-redis-config.json'))
                require(settings['configuration']=={'save':'','appendonly':'no','io-threads':'1'} and
                        'connected_slaves:0\r\n'in settings['replication'],'Redis memory mode differs')
            listener=read(d/'redis-listener.json');require(listener['pid']in identities and listener['pid']!=client_pid,'Redis listener owner differs')
            raw=read(d/'final-dataset.json');dataset={base64.b64decode(k):base64.b64decode(v)if v is not None else None for k,v in raw.items()}
        dataset_result=dataset_check(c,dataset)
        metrics=r['metrics']['measurement'];outcomes={name:sum(op['populations'][i]['calls']for op in metrics['statistics'])for i,name in enumerate(metrics['outcomes'])}
        operations=operation_statistics(metrics,r['cohort_elapsed_ns'],is_native)
        all_attempts=sum(op['attempts'] for op in operations)
        result['cases'].append(dict(ordinal=ordinal,repeat=repeat,concurrency=concurrency,workers=concurrency,
             arm=arm,workload=workload,complete=True,api=api,read_api=api,write_api=write_api,
             batch_size=c['batch_size'],read_percent=c['read_percent'],operations=operations,
             directory=str(d),calls=r['measured_completed'],issued=r['measured_issued'],dropped_slots=r['dropped_slots'],outcomes=outcomes,
             cohort_elapsed_ns=r['cohort_elapsed_ns'],successful_calls_per_second=outcomes['success']/(r['cohort_elapsed_ns']/1e9),
             completed_calls_per_second=r['measured_completed']*1e9/r['cohort_elapsed_ns'],
             completed_input_items_per_second=sum(op['input_items'] for op in operations)*1e9/r['cohort_elapsed_ns'],
             input_items=sum(op['input_items'] for op in operations),
             successful_items_per_second=r['successful_input_items_per_second'],
             completed_before_cutoff=sum(p['completed_before_cutoff']for op in metrics['statistics']for p in op['populations']),
             attempts=all_attempts,
             connection_attempts=None if is_native else sum(op['connection_attempts']for op in metrics['statistics']),
             connection_failures=None if is_native else sum(op['connection_failures']for op in metrics['statistics']),
             attempt_reasons=({name:sum(op['attempt_reasons'][i]for op in metrics['statistics'])for i,name in enumerate(metrics['reasons'])} if is_native else None),
             terminal_rpc_codes=([sum(op['rpc_codes'][i]for op in metrics['statistics'])for i in range(17)] if is_native else None),
             reasons={name:sum(op['reasons'][i]for op in metrics['statistics'])for i,name in enumerate(metrics['reasons'])},
             report_sha256=sha(d/'run/report.json'),final_keys=len(dataset),dataset=dataset_result,
             all_nonce_zero=dataset_result['all_nonce_zero'],
             all_success_single_attempt=outcomes['success']==r['measured_completed']==all_attempts,
             whole_call_latency=histogram([p for op in metrics['statistics']for p in op['populations']]),
             successful_whole_call_latency=histogram([op['populations'][0]for op in metrics['statistics']]),
             server_memory=memory,
             storage=storage_summary,
             endpoint_snapshot_scope='Six existing before/after metric/status/resource files are hash-bound only; endpoint stage semantics remain separately validated' if is_native else None,
             short_diagnostic_only=True))
        print('PASS: retained diagnostic cohort '+str(ordinal)+' '+arm+' c'+str(concurrency),flush=True)
    for repeat in (0,1):
        for concurrency in CONCURRENCIES:
            for workload,_,_ in WORKLOADS:
                for server in ('old','new'):
                    redis.paired_configuration(configs[(repeat,concurrency,'redis-'+workload)],
                                               configs[(repeat,concurrency,server+'-'+workload)])
    memory_pairs = []
    performance_pairs = []
    for repeat in (0,1):
        for concurrency in CONCURRENCIES:
            for workload,_,_ in WORKLOADS:
                old,new = paired_cases(result['cases'],repeat,concurrency,workload)
                require(set(old['server_memory'])==set(new['server_memory'])=={'voter-1','voter-2','voter-3'},
                        'paired voter memory inventory differs')
                aggregate={role:dict(
                    sampled_mean_rss_bytes=sum(v['rss_mean_bytes'] for v in case['server_memory'].values()),
                    summed_hwm_bytes=sum(v['hwm_max_bytes'] for v in case['server_memory'].values()))
                    for role,case in (('old',old),('new',new))}
                memory_pairs.append(dict(repeat=repeat,concurrency=concurrency,workload=workload,
                    read_api=old['read_api'],write_api=old['write_api'],aggregate_voters=aggregate,
                    aggregate_mean_rss_delta_bytes=aggregate['new']['sampled_mean_rss_bytes']-aggregate['old']['sampled_mean_rss_bytes'],
                    aggregate_hwm_delta_bytes=aggregate['new']['summed_hwm_bytes']-aggregate['old']['summed_hwm_bytes'],
                    per_voter={name:dict(old=old['server_memory'][name],new=new['server_memory'][name],
                        sampled_mean_rss_delta_bytes=new['server_memory'][name]['rss_mean_bytes']-old['server_memory'][name]['rss_mean_bytes'],
                        sampled_hwm_delta_bytes=new['server_memory'][name]['hwm_max_bytes']-old['server_memory'][name]['hwm_max_bytes'])
                        for name in old['server_memory']},
                    scope='Within one concurrency/repetition; voter numbers need not match leader roles; HWM includes setup and summed high waters need not coincide'))
                performance_pairs.append(dict(repeat=repeat,concurrency=concurrency,workload=workload,
                    read_api=old['read_api'],write_api=old['write_api'],batch_size=old['batch_size'],read_percent=old['read_percent'],
                    old_ordinal=old['ordinal'],new_ordinal=new['ordinal'],
                    successful_qps_change_percent=(new['successful_calls_per_second']/old['successful_calls_per_second']-1)*100
                        if old['successful_calls_per_second'] else None,
                    completed_qps_change_percent=(new['completed_calls_per_second']/old['completed_calls_per_second']-1)*100,
                    mean_change_percent=(new['whole_call_latency']['mean_ns']/old['whole_call_latency']['mean_ns']-1)*100,
                    old_whole_call_latency=old['whole_call_latency'],new_whole_call_latency=new['whole_call_latency'],
                    old_operations=old['operations'],new_operations=new['operations'],
                    all_success_single_attempt=old['all_success_single_attempt'] and new['all_success_single_attempt']))
    result['performance_comparison'] = performance_pairs
    require(len(result['cases'])==24 and len(lifetimes)==80 and drains==48 and writer_bindings==48,
            'full cohort/lifetime/drain/listener counts differ')
    result['memory_comparison'] = memory_pairs
    result.update(complete=True,matched_diagnostic_accepted=args.successful_arms==24,
                  all_success_single_attempt=all(c['all_success_single_attempt'] for c in result['cases']),
                  performance_promotion=False,
                  successful_arms=args.successful_arms,owned_lifetimes_exited=len(lifetimes),
                  role_source_file_checks=source_files,resource_samples=resource_count,qualifying_drains=drains,
                  voter_writer_listener_bindings=writer_bindings,retained_files=retained_files,retained_bytes=retained_bytes)
except BaseException as error:
    result['failure']=repr(error)
    raise
finally:
    result['ended_unix_ns']=time.time_ns();save('audit.json',result);save('input-inventory.json',records)
print('PASS: twenty-four write-only v3 c1/c64 matched cohorts and exact outer restoration accepted',flush=True)
