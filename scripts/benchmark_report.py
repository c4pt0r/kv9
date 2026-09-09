"""Independently recompute a complete benchmark matrix from bounded raw artifacts."""
import hashlib
import importlib.util
import json
from pathlib import Path
import statistics

from workload_report import bounded, keys, require, sha, strict_json, uint, validate

SCRIPT = Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location('server_metrics',SCRIPT/'check-latency-metrics.py')
server_metrics=importlib.util.module_from_spec(spec);spec.loader.exec_module(server_metrics)
TARGETS=['wal','minio','loopback']
MIXES={'read':dict(get=100,put=0,delete=0),'write':dict(get=0,put=100,delete=0),'mixed':dict(get=50,put=40,delete=10)}
MINIO='quay.io/minio/minio@sha256:14cea493d9a34af32f524e538b8346cf79f3321eff8e708c1e2960462bd8936e'


def read(path, limit=2*1024*1024): return strict_json(bounded(path,limit))


def file_hash(path, bound=512*1024*1024):
    require(0<path.stat().st_size<=bound,'invalid benchmark executable size')
    with path.open('rb') as stream: return hashlib.file_digest(stream,'sha256').hexdigest()


def check_build(build, expected_revision=None, release=False):
    build=Path(build);b=read(build/'build.json');w=read(build/'workload/build.json');inventory=read(build/'workload/sources.json')
    require(b['version']==1 and b['profile'] in ('debug','release'),'invalid benchmark build')
    require(not release or b['profile']=='release','measurements require an actual release build')
    require(b['profile']==w['profile'] and b['rustc']==w['rustc'] and b['revision']==w['revision'] and b['dirty']==w['dirty'],
            'benchmark binaries have different build provenance')
    require(b['sources']==inventory['sources'] and sha(json.dumps(b['sources'],sort_keys=True,separators=(',',':')).encode())==
            b['source_tree_sha256']==w['source_tree_sha256'],'benchmark source inventory differs')
    require(b['workload_build_sha256']==sha(bounded(build/'workload/build.json',65536)),'benchmark workload manifest differs')
    if expected_revision: require(b['revision']==expected_revision and b['dirty'] is False,'benchmark is not from the required clean revision')
    require(set(b['binaries'])=={'kv9','workload-loopback'},'benchmark binary inventory differs')
    for name,selector in [('kv9',['--bin','kv9']),('workload-loopback',['-p','kv9-server','--example','workload-loopback'])]:
        item=b['binaries'][name]
        require(file_hash(build/name)==item['sha256'],'benchmark executable hash mismatch')
        require(item['command']==['cargo','build','--locked',*selector,*(['--release'] if b['profile']=='release' else []),
                                  '--message-format=json-render-diagnostics'],'benchmark build enabled different features or targets')
    return b


def quantiles(buckets):
    total=sum(buckets);result={}
    for p in (50,95,99):
        value=None;rank=(total*p+99)//100;seen=0
        if total:
            for i,n in enumerate(buckets):
                seen+=n
                if seen>=rank:
                    value=server_metrics.bounds(i);break
        result[f'p{p}']=value
    return result


def snapshots(directory, fixture, report, ticks):
    resources={phase:read(directory/(phase+'-resources.json')) for phase in ('before','after')}
    ids=fixture['process_ids'];result=dict(population='whole trial envelope including initialization, warmup, measurement, drain, verification and metric export waits',processes={})
    require(set(ids)==({'1'} if fixture['target']=='loopback' else {'1','2','3'}),'unexpected fixture process population')
    for phase,r in resources.items(): require(set(r['processes'])==set(ids),'resource snapshot omitted a process')
    for node,pid in ids.items():
        before,after=(resources[p]['processes'][node] for p in ('before','after'))
        require(before['pid']==after['pid']==pid and before['start_ticks']==after['start_ticks']==fixture['process_identities'][node]['start_ticks'],'resource snapshots span a process restart')
        require(before['cpu_affinity']==after['cpu_affinity']==fixture['placement']['server_cpus'],'server CPU placement changed during the trial')
        elapsed=uint(after['observed_monotonic_ns']-before['observed_monotonic_ns'],1)
        cpu=sum(uint(after[k]-before[k]) for k in ('user_ticks','system_ticks'))/ticks
        require(before['peak_rss_bytes']>0 and after['peak_rss_bytes']>0 and before['rss_bytes']>0 and after['rss_bytes']>0,'invalid process memory samples')
        require(before['io'].keys()==after['io'].keys(),'process I/O inventory differs')
        io={k:uint(after['io'][k]-v) for k,v in before['io'].items()}
        result['processes'][node]=dict(cpu_seconds=cpu,elapsed_ns=elapsed,cpu_cores=cpu*1e9/elapsed,
            before_rss_bytes=before['rss_bytes'],after_rss_bytes=after['rss_bytes'],reported_peak_rss_before_bytes=before['peak_rss_bytes'],reported_peak_rss_after_bytes=after['peak_rss_bytes'],
            reported_peak_rss_decreased=after['peak_rss_bytes']<before['peak_rss_bytes'],io_delta=io)
    if fixture['target']=='loopback': return result
    metrics={p:read(directory/(p+'-metrics.json')) for p in ('before','after')}
    states={p:read(directory/(p+'-status.json')) for p in ('before','after')}
    result['server_metrics']={}
    acknowledged_reads=sum(row[0][0] for row in report['metrics']['logical_counts'])
    acknowledged_writes=sum(row[1][0]+row[2][0] for row in report['metrics']['logical_counts'])
    observed_reads=observed_writes=0
    client_start=report['wall_anchor_unix_ns']
    client_end=client_start+report['elapsed_ns']-report['wall_anchor_monotonic_ns']
    for phase in ('before','after'):
        require(set(metrics[phase])==set(states[phase])==set(ids),'server snapshot omitted a voter')
        for node,pid in ids.items():
            doc=metrics[phase][node];server_metrics.validate(doc)
            require(doc['node_id']==int(node) and doc['process_id']==pid==int(states[phase][node]['pid']),'server snapshot identity differs')
            require(int(doc['captured_unix_ns'])>=resources[phase]['requested_unix_ns'],'server metric snapshot is stale')
            require(int(states[phase][node]['public_rpc_limit_requests'])==64 and
                    int(states[phase][node]['public_rpc_limit_encoded_bytes'])==16777216,'server admission settings changed')
            require(not states[phase][node]['fatal'],'server reported a fatal error during a trial')
            require((int(doc['captured_unix_ns'])<=client_start) if phase=='before' else
                    (int(doc['captured_unix_ns'])>=client_end),'server snapshots do not bracket the full client trial')
    for node in ids:
        before,after=metrics['before'][node],metrics['after'][node]
        require(server_metrics.identity(before)==server_metrics.identity(after),'server metrics span a restart')
        delta={}
        for a,b in zip(before['metrics'],after['metrics']):
            outcomes={}
            for x,y in zip(a['latency']['outcomes'],b['latency']['outcomes']):
                buckets=[uint(j-i) for i,j in zip(x['buckets'],y['buckets'])]
                count=uint(y['count']-x['count']);duration=uint(y['sum_ns']-x['sum_ns'])
                require(count==sum(buckets),'server delta histogram does not conserve its count')
                outcomes[x['outcome']]=dict(count=count,sum_ns=duration,**quantiles(buckets))
            delta[a['name']]=outcomes
        observed_reads+=delta['public_raw_read_backend']['success']['count']
        observed_writes+=delta['public_raw_write_backend']['success']['count']
        result['server_metrics'][node]=dict(latency_delta=delta,apply_lag_before=before['apply_lag'],apply_lag_after=after['apply_lag'])
    require(observed_reads>=acknowledged_reads and observed_writes>=acknowledged_writes,'server snapshots omit acknowledged operation samples')
    result['kernel_database_write_bytes']=sum(p['io_delta']['write_bytes'] for p in result['processes'].values())
    result['object_store_process_io_included']=False
    return result


def validate_matrix(root, build, expected_revision=None):
    root,build=Path(root),Path(build);p=read(root/'protocol.json',65536);index=read(root/'index.json',65536)
    keys(p,'version kind repetitions measure_ms targets mixes concurrency keys value_bytes warmup_operations seed_base'.split())
    require(p['version']==1 and p['kind'] in ('smoke','measurement'),'invalid benchmark protocol')
    require(p['targets']==TARGETS and p['mixes']==MIXES and p['concurrency']==[1,4] and
            (p['keys'],p['value_bytes'],p['warmup_operations'],p['seed_base'])==(8,128,32,40),'unsupported measurement matrix')
    require((p['repetitions'],p['measure_ms'])==((2,500) if p['kind']=='smoke' else (3,5000)),'measurement duration or repetition count differs')
    b=check_build(build,expected_revision,release=p['kind']=='measurement')
    require(index['version']==1 and index['complete'] is True and 'failure' not in index,'benchmark run is incomplete or failed')
    require(index['protocol_sha256']==sha(bounded(root/'protocol.json',65536)),'benchmark protocol hash differs')
    expected=[f'r{r}-{target}' for r in range(p['repetitions']) for target in (TARGETS if r%2==0 else reversed(TARGETS))]
    require(index['fixtures']==expected,'benchmark matrix omits or duplicates a fixture')
    require({d.name for d in root.iterdir() if d.is_dir() and (d/'fixture.json').exists()}==set(expected),'unreported fixture evidence')
    host=read(root/'host.json');ticks=uint(host['cpu_clock_ticks_per_second'],1,1_000_000)
    require(host['cpu_affinity'] and host['machine'] and host['release'] and host['network'].startswith('127.0.0.1'),'missing host topology or resource inventory')
    require(all(host[name]['exit_code']==0 and host[name]['stdout'] for name in ('cpu','block_devices','filesystem','docker')),'missing host hardware or tool inventory')
    placement=host['placement']
    require(placement['client_cpus'] and placement['server_cpus'] and
            set(placement['client_cpus']+placement['server_cpus'])<=set(host['cpu_affinity']) and placement['exclusive_host'] is False,
            'invalid benchmark CPU placement')
    require(placement['separated']==set(placement['client_cpus']).isdisjoint(placement['server_cpus']) and
            (p['kind']=='smoke' or placement['separated']),'measurements require separate client and server CPU sets')
    workload_hash=file_hash(build/'workload/kv9-workload')
    trials=[];guards=[]
    for fixture_name in expected:
        folder=root/fixture_name;f=read(folder/'fixture.json');target=f['target'];repeat=int(fixture_name.split('-')[0][1:])
        require(target in TARGETS and fixture_name==f'r{repeat}-{target}' and f['complete'] is True,'fixture is incomplete or mislabeled')
        require(f['server_settings']==dict(public_max_requests=64,public_max_encoded_bytes=16777216,flush_interval_ms=100),'fixture settings differ')
        require(set(f['addresses'])==set(f['process_ids'])==set(f['process_identities']),'fixture peer inventory differs')
        require(f['placement']==placement,'fixture CPU placement differs from the host protocol')
        for node,identity in f['process_identities'].items():
            require(identity['pid']==f['process_ids'][node] and identity['cpu_affinity']==placement['server_cpus'] and
                    identity['executable_sha256']==b['binaries']['workload-loopback' if target=='loopback' else 'kv9']['sha256'],
                    'running fixture executable or CPU placement differs')
        if target=='minio':
            require(f['object_store']['image']==MINIO and f['object_store']['version'] and f['object_store']['image_id'].startswith('sha256:'),'MinIO identity is missing or unpinned')
            require(f['object_store']['cpu_affinity']==','.join(map(str,placement['server_cpus'])),'object store CPU placement differs')
            require(set(f['remote_checkpoints'])=={'1','2','3'},'MinIO checkpoint evidence omitted a replica')
            for node,checkpoint in f['remote_checkpoints'].items():
                data=bounded(folder/f'n{node}-checkpoint.bin',48*1024*1024)
                require(data.startswith(b'KV9CHECKPOINT\x01') and len(data)==checkpoint['bytes'] and sha(data)==checkpoint['sha256'],
                        'retained MinIO checkpoint evidence differs')
        cells=[(mix,n) for mix in MIXES for n in (1,4)]
        if repeat%2: cells.reverse()
        expected_trials=[(fixture_name+'-before','mixed',4,True)]+[(f'{fixture_name}-{mix}-c{n}',mix,n,False) for mix,n in cells]+[(fixture_name+'-after','mixed',4,True)]
        require([r['name'] for r in f['trials']]==[r[0] for r in expected_trials],'trial matrix omits or duplicates a cell')
        require({d.name for d in folder.iterdir() if d.is_dir() and (d/'requested-config.json').exists()}=={r[0] for r in expected_trials},'unreported trial evidence')
        loopback_requests=[0,0,0];loopback_writes=0;client_processes=set();keyspaces=set()
        for entry,(name,mix,workers,guard) in zip(f['trials'],expected_trials):
            require(entry['complete'] is True and entry['exit_code']==0 and entry['mix']==mix and entry['workers']==workers and
                    entry['guard']==guard and entry['repeat']==repeat,'failed or mislabeled trial was accepted')
            directory=folder/name
            checked=validate(directory/'run',build/'workload',expected_revision,seconds=60)
            report=read(directory/'run/report.json');config=report['configuration']
            require(entry['report_sha256']==checked['report_sha256'],'trial report hash differs')
            require(read(directory/'requested-config.json')==config,'trial ran a different requested configuration')
            require(config['mode']==('correctness' if guard else 'performance') and config['run_id']==config['keyspace_name']==name and
                    config['workers']==workers and config['keys']==8 and config['value_bytes']==128 and config['mix']==MIXES[mix] and
                    config['seed']==40+repeat and config['warmup_operations']==32 and config['interval_ms']==(2 if guard else 0) and
                    config['max_operations']==(180 if guard else 1_000_000) and config['measure_ms']==(2000 if guard else p['measure_ms']),
                    'trial workload parameters differ from the fixed protocol')
            if not guard:
                require(all(weight>0 or sum(report['metrics']['logical_counts'][2][i])==0
                            for i,weight in enumerate((MIXES[mix][op] for op in ('get','put','delete')))),
                        'trial issued an operation excluded by its workload mix')
            client=config['client']
            require(client['max_in_flight']==workers and client['max_attempts']==6 and client['deadline_ms']==1500 and client['retry_backoff_ms']==5 and
                    {str(peer['node_id']):peer['address'] for peer in client['peers']}==f['addresses'],'trial routing or retry limits differ')
            require(report['runtime_threads']==2,'workload runtime worker count changed')
            require(report['process_id'] not in client_processes,'trial reused a workload process')
            client_processes.add(report['process_id'])
            identity=entry['client_identity']
            require(identity['pid']==report['process_id'] and identity['cpu_affinity']==placement['client_cpus'] and
                    identity['executable_sha256']==workload_hash,'client process or CPU placement differs')
            if target=='loopback': require(client['keyspace_id']==1,'calibration keyspace differs')
            else:
                require(client['keyspace_id'] not in keyspaces,'trial reused a mutable dataset keyspace')
                keyspaces.add(client['keyspace_id'])
                receipt=dict(line.split('=',1) for line in (directory/'create-keyspace.out').read_text().splitlines())
                require(int(receipt['keyspace_id'])==client['keyspace_id'] and int(receipt['proposed_term'])>0 and int(receipt['proposed_index'])>0,'keyspace setup lacks its acknowledged receipt')
            resource_summary=snapshots(directory,f,report,ticks)
            for op in range(3): loopback_requests[op]+=sum(sum(phase[op]) for phase in report['metrics']['attempt_counts'])
            loopback_writes+=sum(phase[1][0]+phase[2][0] for phase in report['metrics']['logical_counts'])
            if guard:
                require(checked['full_history_independently_checked'],'correctness guard lacks a complete checked history')
                guards.append(dict(name=name,target=target,report_sha256=checked['report_sha256'],operations=checked['measured_operations']))
                continue
            require(report['stop']['reason']=='duration' and report['stages']['measurement']['end_ns']-report['stages']['measurement']['start_ns']>=p['measure_ms']*1_000_000,
                    'performance trial stopped before its required duration')
            require(checked['full_history_independently_checked'] is False,'performance-only trial claims a full history')
            counts=report['metrics']['logical_counts'][2]
            require(sum(map(sum,counts))==report['measured_completed'],'performance trial contains undeclared fault phases')
            trials.append(dict(name=name,target=target,repeat=repeat,mix=mix,workers=workers,report_sha256=checked['report_sha256'],
                successful=report['measured_successful'],terminal=report['measured_completed'],cohort_elapsed_ns=report['cohort_elapsed_ns'],
                terminal_ops_per_second=report['cohort_terminal_ops_per_second'],success_ops_per_second=report['cohort_success_ops_per_second'],
                unsuccessful_fraction=1-report['measured_successful']/report['measured_completed'],
                logical_counts=counts,attempt_counts=report['metrics']['attempt_counts'][2],
                logical_latency=report['metrics']['logical_latency'][2],attempt_latency=report['metrics']['attempt_latency'][2],
                client_resources_before=report['resources_before'],client_resources_after=report['resources_after'],
                client_peak_in_flight=report['history']['peak_in_flight'],client_recorder_ns=report['history']['recorder_ns'],
                server_resources=resource_summary))
        if target=='loopback':
            ready,done=read(folder/'ready.json'),read(folder/'loopback.json')
            require(ready['kind']==done['kind']=='in_memory_loopback_calibration' and ready['durability'] is done['durability'] is False,
                    'loopback calibration is mislabeled as durable storage')
            require(ready['pid']==done['pid']==f['process_ids']['1'] and ready['address']==f['addresses']['1'] and ready['runtime_threads']==2,
                    'calibration process identity differs')
            require(done['requests']==loopback_requests and done['synthetic_write_index']==loopback_writes,'calibration endpoint accounting differs from actual client attempts')
            require(0<done['connections']<sum(loopback_requests) and 0<=done['keys']<=4096,'calibration lacks real connection reuse or exceeded its dataset bound')
    groups=[]
    for target in TARGETS:
        for mix in MIXES:
            for workers in (1,4):
                rows=[r for r in trials if (r['target'],r['mix'],r['workers'])==(target,mix,workers)]
                require(len(rows)==p['repetitions'],'missing trial repetition')
                rates=[r['success_ops_per_second'] for r in rows]
                groups.append(dict(target=target,mix=mix,workers=workers,trial_names=[r['name'] for r in rows],
                                   success_rates=rates,minimum=min(rates),median=statistics.median(rates),maximum=max(rates)))
    calibration=[]
    for database in (g for g in groups if g['target']!='loopback'):
        control=next(g for g in groups if (g['target'],g['mix'],g['workers'])==('loopback',database['mix'],database['workers']))
        calibration.append(dict(target=database['target'],mix=database['mix'],workers=database['workers'],
            loopback_minimum_to_database_maximum_ratio=control['minimum']/database['maximum'] if database['maximum'] else None,
            proves_server_capacity=False))
    result=dict(version=1,accepted=True,kind=p['kind'],measurement_baseline=p['kind']=='measurement',
        topology='single host, loopback network, shared hardware; no multi-host capacity claim',placement=placement,
        memory_samples='raw approximate /proc VmRSS and VmHWM; reported high-water samples may decrease',
        build_revision=b['revision'],build_dirty=b['dirty'],build_sha256=sha(bounded(build/'build.json',2*1024*1024)),
        protocol_sha256=index['protocol_sha256'],host_sha256=sha(bounded(root/'host.json',2*1024*1024)),
        validator_sources={name:sha((SCRIPT/name).read_bytes()) for name in ('benchmark_report.py','workload_report.py','history/checker.py','check-latency-metrics.py')},
        latency_populations='per-operation logical and attempt outcomes; percentiles are histogram intervals, never averaged across trials',
        trials=trials,guards=guards,groups=groups,calibration=calibration,
        correctness_scope='full independent histories in before/after guards only; performance trials retain complete counts, not full histories')
    if (root/'report.json').exists(): require(read(root/'report.json',16*1024*1024)==result,'stored benchmark summary differs from independent recomputation')
    return result
