#!/usr/bin/env python3
"""Independent four-repeat c1/c64 forty-eight-cohort diagnostic readback; no workload execution."""
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
parser.add_argument('--successful-arms', type=int, choices=(48,), required=True)
args = parser.parse_args()
ROOT = args.run.resolve()
OUT = args.output.resolve()
if not OUT.is_relative_to(Path(__file__).parent) or OUT.exists():
    raise ValueError('audit output must be a new directory within the owned preparation')
OUT.mkdir()
SOURCE = Path('/tmp/kv9-point-batch1-measurement')
CLIENT_REV = '03c1c776a5dd7d1cc67491ab253e02ce51665bf8'
OLD_REV = '57ff6851e40ed63c837189d6eb0a11190704725a'
NEW_REV = '1b70dba62e492cd8e10dd20f643fee74d39abc57'
REDIS_REV = 'b8ec38f660786412705f350d96ab086f0e7f6c60'
REDIS_SOURCE = Path('/tmp/kv9-redis-point-get-reference')
PINS = {
 'old': (OLD_REV, '6d76779a3e421388692b01ce08bd289477283fc05fd28b734eaed9138e0f7e30'),
 'new': (NEW_REV, 'ab05a16fd520d4c612b01cedea1a514411e607eb0bc645ce28fce24b3299d1ef'),
 'client': (CLIENT_REV, '22ca0883ca2900fab0b457d87ebc6bc50662d7145af0846304f02b5841a6d30b'),
 'redis': (REDIS_REV, '61349111171ded5cb7fff9085c7886381788e5f4c7a407e91a5dda618d124892'),
}
PROTOCOL_ID = 'kv9-work-signal-c1-c64-v1'
CONCURRENCIES = (1, 64)
ARMS = (('old-point','old','point_get'), ('old-batch1','old','batch_get'),
        ('new-point','new','point_get'), ('new-batch1','new','batch_get'),
        ('redis-mget1',None,'mget'), ('redis-get1',None,'get'))
ROLE_PATHS = {
 'old': ('/tmp/kv9-read-group-credit', '/tmp/kv9-read-group-credit-release-first', 'kv9'),
 'new': ('/tmp/kv9-work-signal-coalescing', '/tmp/kv9-work-signal-coalescing-release-first', 'kv9'),
 'client': ('/tmp/kv9-point-batch1-measurement', '/tmp/kv9-point-batch1-client-release-first', 'kv9-batch-benchmark'),
 'redis': ('/tmp/kv9-redis-point-get-reference', '/tmp/kv9-redis-point-get-reference-release-first', 'kv9-redis-batch-reference'),
}
SERVER_MANIFESTS = {
 'old': '7e5179f856b8828c26a6bf68ce0056755e16e12bbdc35c9a11845178712caabd',
 'new': '800e74e36f7de539bb38d3cfe097e356ed2ebd1e33d16d7b48435eb6f2bc93f3',
}
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
    forward = [(concurrency,*arm) for concurrency in CONCURRENCIES for arm in ARMS]
    return [(repeat,*row) for repeat in (0,1,2,3)
            for row in (forward if repeat in (0,3) else reversed(forward))]

def shared_config(repeat, concurrency):
    require(type(repeat) is int and repeat in (0,1,2,3) and
            type(concurrency) is int and concurrency in CONCURRENCIES, 'invalid repeat/concurrency')
    return dict(run_id=f'p{repeat}{concurrency:04d}',seed=71,workers=concurrency,keys=4096,
                batch_size=1,value_bytes=128,read_percent=100,warmup_calls=128,
                measure_ms=10000,max_calls=10_000_000,load={'kind':'closed_loop'})

def expected_descriptor(repeat, concurrency, arm, server, api):
    return dict(repeat=repeat,arm=arm,server_role=server,read_api=api,mix='read',
                target='kv9' if server else 'redis-memory',shared=shared_config(repeat,concurrency))

def validate_matrix_protocol(matrix):
    require(type(matrix['version']) is int and matrix['version']==2 and
            matrix['protocol_id']==PROTOCOL_ID and
            same_json(matrix['concurrency_points'],list(CONCURRENCIES)), 'c1/c64 protocol scope differs')
    require(matrix['smoke'] is False and matrix['fixed_rates'] is None, 'smoke/fixed-rate is not timed protocol')
    expected=[expected_descriptor(*row) for row in expected_cohorts()]
    require(len(matrix['attempts'])==48 and same_json(matrix['cohort_inventory'],expected),
            'forty-eight-cohort inventory/order differs')

def validate_requested_config(config, repeat, concurrency, server, api):
    expected=shared_config(repeat,concurrency)
    if server is not None:
        client=config['client']
        expected.update(version=2,rpc_transport='tonic_stream',read_api=api,
                        client=dict(version=1,peers=client['peers'],keyspace_id=client['keyspace_id'],
                                    epoch_conf_ver=1,epoch_version=1,max_in_flight=concurrency,
                                    max_attempts=6,deadline_ms=1500,retry_backoff_ms=5))
    else:
        expected.update(version=2,read_api=api,address=config['address'],deadline_ms=1500)
    require(same_json(config,expected), 'independent exact request config differs')

def paired_cases(cases, repeat, concurrency, suffix):
    result=[]
    for arm in ('old-'+suffix,'new-'+suffix):
        selected=[c for c in cases if c['repeat']==repeat and c['concurrency']==concurrency and c['arm']==arm]
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
    return dict(count=count, sum_ns=total, mean_ns=total / count,
                **{f'p{p}': quantile(p) for p in (50, 95, 99)})

def value(seed, size, index, nonce):
    def mix(n):
        n = (n + 0x9e3779b97f4a7c15) % 2**64
        n = ((n ^ (n >> 30)) * 0xbf58476d1ce4e5b9) % 2**64
        n = ((n ^ (n >> 27)) * 0x94d049bb133111eb) % 2**64
        return n ^ (n >> 31)
    result = index.to_bytes(8, 'big') + nonce.to_bytes(8, 'big')
    for word in range((size - 16 + 7) // 8):
        result += mix(seed ^ mix(index) ^ mix(nonce) ^ word).to_bytes(8, 'big')
    return result[:size]

def dataset_check(config, dataset):
    require(len(dataset) == config['keys'] + 1, 'final dataset cardinality differs')
    for index in range(config['keys'] + 1):
        actual = dataset[f'{config["run_id"]}:{index:016x}'.encode()]
        require(actual is not None and len(actual) == config['value_bytes'], 'final value length/missing differs')
        nonce = int.from_bytes(actual[8:16], 'big')
        require(nonce <= config['warmup_calls'] + config['max_calls'] and actual == value(config['seed'], config['value_bytes'], index, nonce), 'independent value body differs')
        require(index != config['keys'] or nonce == 0, 'sentinel changed')

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
              scope='Four-repeat 10-second c1/c64 shared-host tmpfs diagnostic; no sustained-capacity or equal-durability claim',
              parent_confirmed_session=args.root_session, parent_confirmed_exit_code=args.root_exit_code,
              expected_successful_arms=args.successful_arms, protocol_id=PROTOCOL_ID,
              concurrency_points=list(CONCURRENCIES), started_unix_ns=time.time_ns())
try:
    matrix=read(ROOT/'matrix.json')
    validate_matrix_protocol(matrix)
    require(matrix['smoke'] is False and matrix['storage']=='tmpfs' and matrix['client_revision']==CLIENT_REV,
            'wrong fixture or source scope')
    require(matrix['complete'] is (args.root_exit_code==0), 'original fixture verdict differs from parent terminal confirmation')
    require(args.successful_arms==48 and args.root_exit_code==0, 'expected complete diagnostic scope differs')
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
    expected_argv=['/usr/bin/python3','/tmp/kv9-work-signal-comparison-preparation/matched-driver.py',
                   '--output',str(ROOT),'--isolation-snapshot',str(outer/'isolation.json'),
                   '--client-source','/tmp/kv9-point-batch1-measurement',
                   '--client-build','/tmp/kv9-point-batch1-client-release-first','--expected-client-revision',CLIENT_REV]
    require(invocation['argv']==expected_argv and invocation['wrapper_sha256']==
            'e922939e675d722fe36c3b0890d31d6329be810dd7567b2e369643d10e2137fb' and
            invocation['driver_arguments_sha256']=='47c521c0a15dff9cf44af891a6a891daec0b1f9beb2b994f2990002bc58527df',
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
    require(matrix['helper_hashes'][str(Path('/tmp/kv9-work-signal-comparison-preparation/matched-driver.py'))]==args.expected_driver_sha256,
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
    for ordinal,(attempt,(repeat,concurrency,arm,server,api)) in enumerate(zip(matrix['attempts'],expected)):
        shared=shared_config(repeat,concurrency)
        d=Path(attempt['directory']);desc=attempt['descriptor'];is_native=server is not None
        require(d==ROOT/f'{ordinal:03d}-{arm}-p{repeat}{concurrency:04d}' and attempt['index']==ordinal and desc==matrix['cohort_inventory'][ordinal],
                'cohort path/order differs')
        require((desc['arm'],desc['server_role'],desc['read_api'],desc['repeat'])==(arm,server,api,repeat) and desc['shared']==shared,
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
        validate_requested_config(c,repeat,concurrency,server,api)
        require(all(c[k]==v for k,v in shared.items()),'requested shared config differs')
        require(sha(d/'run/report.json')==attempt['report_sha256']==summary['report_sha256'],'original report changed')
        role='client' if is_native else 'redis';build=Path(matrix['role_bindings'][role]['build_directory'])
        validated=(native if is_native else redis).validate(d/'run',build,d/'requested-config.json',PINS[role][0],require_timing=True)
        require(validated['accepted'] and r['stop_reason']=='duration' and r['dropped_slots']==0,'report incomplete/capped')
        if is_native:
            require(c['version']==r['version']==2 and c['read_api']==api and c['rpc_transport']=='tonic_stream','genuine v2 API config missing')
            require(r['workload_model']==('bounded_native_point_get_diagnostic' if api=='point_get' else 'bounded_native_batch_performance'),
                    'workload identity differs')
            for phase in ('warmup','measurement'):
                require(r['metrics'][phase]['operations']==(['get','batch_put'] if api=='point_get' else ['batch_get','batch_put']),
                        'chosen API vocabulary differs')
        else:
            require(c['version']==r['version']==2 and c['read_api']==api and summary['operation']==api,
                    'genuine Redis v2 API configuration missing')
            require(r['workload_model']==('bounded_redis_point_get_diagnostic' if api=='get' else 'bounded_redis_batch_performance'),
                    'Redis workload identity differs')
            for phase in ('warmup','measurement'):
                require(r['metrics'][phase]['operations']==[api,'mset'],'Redis selected API vocabulary differs')
            for phase in ('initialization','verification'):
                require(r['metrics'][phase]['operations']==['mget','mset'],'Redis setup API vocabulary differs')
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
        if is_native:
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
        dataset_check(c,dataset)
        require(all(v[8:16]==bytes(8)for v in dataset.values()),'read-only dataset contains nonzero nonce')
        metrics=r['metrics']['measurement'];outcomes={name:sum(op['populations'][i]['calls']for op in metrics['statistics'])for i,name in enumerate(metrics['outcomes'])}
        result['cases'].append(dict(ordinal=ordinal,repeat=repeat,concurrency=concurrency,workers=concurrency,arm=arm,complete=True,api=api,
             directory=str(d),calls=r['measured_completed'],issued=r['measured_issued'],dropped_slots=r['dropped_slots'],outcomes=outcomes,
             cohort_elapsed_ns=r['cohort_elapsed_ns'],successful_calls_per_second=outcomes['success']/(r['cohort_elapsed_ns']/1e9),
             successful_items_per_second=r['successful_input_items_per_second'],
             completed_before_cutoff=sum(p['completed_before_cutoff']for op in metrics['statistics']for p in op['populations']),
             attempts=(sum(sum(op['attempt_reasons'])for op in metrics['statistics']) if is_native else
                       sum(op['command_attempts']for op in metrics['statistics'])),
             connection_attempts=None if is_native else sum(op['connection_attempts']for op in metrics['statistics']),
             connection_failures=None if is_native else sum(op['connection_failures']for op in metrics['statistics']),
             attempt_reasons=({name:sum(op['attempt_reasons'][i]for op in metrics['statistics'])for i,name in enumerate(metrics['reasons'])} if is_native else None),
             terminal_rpc_codes=([sum(op['rpc_codes'][i]for op in metrics['statistics'])for i in range(17)] if is_native else None),
             reasons={name:sum(op['reasons'][i]for op in metrics['statistics'])for i,name in enumerate(metrics['reasons'])},
             report_sha256=sha(d/'run/report.json'),final_keys=len(dataset),all_nonce_zero=True,
             whole_call_latency=histogram([p for op in metrics['statistics']for p in op['populations']]),
             successful_whole_call_latency=histogram([op['populations'][0]for op in metrics['statistics']]),
             server_memory=memory,
             short_diagnostic_only=True))
        print('PASS: retained diagnostic cohort '+str(ordinal)+' '+arm+' c'+str(concurrency),flush=True)
    for repeat in (0,1,2,3):
        for concurrency in CONCURRENCIES:
            for arm,server,api in ARMS:
                if server is not None:
                    reference='redis-get1' if api=='point_get' else 'redis-mget1'
                    redis.paired_configuration(configs[(repeat,concurrency,reference)],configs[(repeat,concurrency,arm)])
    memory_pairs = []
    performance_pairs = []
    for repeat in (0,1,2,3):
        for concurrency in CONCURRENCIES:
            for api, suffix in (('point_get','point'),('batch_get','batch1')):
                old,new = paired_cases(result['cases'],repeat,concurrency,suffix)
                require(set(old['server_memory'])==set(new['server_memory'])=={'voter-1','voter-2','voter-3'},
                        'paired voter memory inventory differs')
                aggregate={role:dict(
                    sampled_mean_rss_bytes=sum(v['rss_mean_bytes'] for v in case['server_memory'].values()),
                    summed_hwm_bytes=sum(v['hwm_max_bytes'] for v in case['server_memory'].values()))
                    for role,case in (('old',old),('new',new))}
                memory_pairs.append(dict(repeat=repeat,concurrency=concurrency,api=api,aggregate_voters=aggregate,
                    aggregate_mean_rss_delta_bytes=aggregate['new']['sampled_mean_rss_bytes']-aggregate['old']['sampled_mean_rss_bytes'],
                    aggregate_hwm_delta_bytes=aggregate['new']['summed_hwm_bytes']-aggregate['old']['summed_hwm_bytes'],
                    per_voter={name:dict(old=old['server_memory'][name],new=new['server_memory'][name],
                        sampled_mean_rss_delta_bytes=new['server_memory'][name]['rss_mean_bytes']-old['server_memory'][name]['rss_mean_bytes'],
                        sampled_hwm_delta_bytes=new['server_memory'][name]['hwm_max_bytes']-old['server_memory'][name]['hwm_max_bytes'])
                        for name in old['server_memory']},
                    scope='Within one concurrency/repetition; voter numbers need not match leader roles; HWM includes setup and summed high waters need not coincide'))
                performance_pairs.append(dict(repeat=repeat,concurrency=concurrency,api=api,
                    old_ordinal=old['ordinal'],new_ordinal=new['ordinal'],
                    successful_qps_change_percent=(new['successful_calls_per_second']/old['successful_calls_per_second']-1)*100,
                    mean_change_percent=(new['whole_call_latency']['mean_ns']/old['whole_call_latency']['mean_ns']-1)*100,
                    old_whole_call_latency=old['whole_call_latency'],new_whole_call_latency=new['whole_call_latency']))
    result['performance_comparison'] = performance_pairs
    require(len(result['cases'])==48 and len(lifetimes)==160 and drains==96 and writer_bindings==96,
            'full cohort/lifetime/drain/listener counts differ')
    result['memory_comparison'] = memory_pairs
    result.update(complete=True,matched_diagnostic_accepted=args.successful_arms==48,
                  successful_arms=args.successful_arms,owned_lifetimes_exited=len(lifetimes),
                  role_source_file_checks=source_files,resource_samples=resource_count,qualifying_drains=drains,
                  voter_writer_listener_bindings=writer_bindings,retained_files=retained_files,retained_bytes=retained_bytes)
except BaseException as error:
    result['failure']=repr(error)
    raise
finally:
    result['ended_unix_ns']=time.time_ns();save('audit.json',result);save('input-inventory.json',records)
print('PASS: forty-eight c1/c64 matched cohorts and exact outer restoration accepted',flush=True)
