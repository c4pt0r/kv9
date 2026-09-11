#!/usr/bin/env python3
"""Two exact CRC write CPU profiles with a separately retained active read-only prefix; no QPS acceptance."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time
from types import SimpleNamespace

if not __debug__:
    raise RuntimeError("profiling requires PYTHONOPTIMIZE=0")
sys.dont_write_bytecode=True
OUT=Path(__file__).parent
DRIVER=Path('/tmp/kv9-owned-batch-broad-workloads-preparation/matched-driver.py')
DRIVER_SHA='ec2203614200eabfb3aa4f423b66cd511c36fc664854fb767bf82e8293527e77'
SOURCE=Path('/tmp/kv9-point-write-measurement-v3')
SERVER_SOURCE=Path('/tmp/kv9-wal-crc32-table')
SERVER_BUILD=Path('/tmp/kv9-wal-crc32-release-first')
CLIENT_BUILD=Path('/tmp/kv9-point-write-v3-release-first/native')
CLIENT_REV='0be806d9671e2c50701a64aa7889c8859b7648ba'
CLIENT_SHA='8da9af469f962a938027d1970141bbe4622f7d42b2b795f720e288fb3f8d5957'
RESP=Path('/tmp/kv9-redis-batch-reference-preparation/run_fixture.py')
PROFILER_CPUS=list(range(6,16))+list(range(22,32))
PERF=(Path('/usr/lib/linux-tools')/os.uname().release/'perf').resolve()
CAP=128*1024**2

def require(value,message):
    if not value:raise ValueError(message)
def sha(path):
    with Path(path).open('rb')as stream:return hashlib.file_digest(stream,'sha256').hexdigest()
def read(path):return json.loads(Path(path).read_text())
def save(path,value):Path(path).write_text(json.dumps(value,indent=2)+'\n')
def module(name,path):
    spec=importlib.util.spec_from_file_location(name,path);value=importlib.util.module_from_spec(spec);spec.loader.exec_module(value);return value
def anchor():
    first=time.monotonic_ns();wall=time.time_ns();last=time.monotonic_ns()
    return dict(monotonic_before_ns=first,realtime_ns=wall,monotonic_after_ns=last)
def process(pid):
    root=Path('/proc',str(pid));fields=(root/'stat').read_text().rsplit(')',1)[1].split()
    return dict(pid=pid,start_ticks=int(fields[19]),boot_id=Path('/proc/sys/kernel/random/boot_id').read_text().strip(),
                comm=(root/'comm').read_text().strip(),cpu_affinity=sorted(os.sched_getaffinity(pid)),
                command_line=[os.fsdecode(x)for x in (root/'cmdline').read_bytes().split(b'\0')if x],
                observed_unix_ns=time.time_ns())
def subtree(pid):
    queue=[pid];found=[]
    for current in queue:
        try:
            found.append(process(current));queue.extend(map(int,Path('/proc',str(current),'task',str(current),'children').read_text().split()))
        except FileNotFoundError:pass
    return found

def preflight(driver):
    server=read(SERVER_BUILD/'build.json');client=read(CLIENT_BUILD/'build.json');inventory=read(CLIENT_BUILD/'sources.json')
    driver.source_binding(SERVER_SOURCE,server,driver.OLD_REVISION)
    driver.source_binding(SOURCE,dict(client,sources=inventory['sources']),CLIENT_REV)
    require(sha(SERVER_BUILD/'build.json')==driver.SERVER_PINS['old']['manifest_sha256'] and
            sha(SERVER_BUILD/'kv9')==driver.SERVER_PINS['old']['binary_sha256'],'frozen server differs')
    require(sha(CLIENT_BUILD/'kv9-batch-benchmark')==CLIENT_SHA==client['binary_sha256'],'frozen shared client differs')
    require(server['binaries']['kv9']['command']==driver.SERVER_COMMAND,'server build command differs')
    driver.cargo_server(SERVER_BUILD/'kv9-cargo.jsonl')
    require(sha(RESP)==driver.RESP_HELPER_SHA,'retained readback helper differs')
    return dict(server=server,client=client,client_sources=inventory)

def main():
    require(sorted(os.sched_getaffinity(0))==PROFILER_CPUS,'harness must use CPUs6-15,22-31')
    require(sha(DRIVER)==DRIVER_SHA,'reviewed fixture driver changed')
    require(not (OUT/'summary.json').exists(),'profile attempt already started')
    driver=module('matched_profile_fixture',DRIVER)
    prefix_helper=module('active_prefix',OUT/'active_prefix.py')
    frozen=preflight(driver)
    sys.path.insert(0,str(SOURCE/'scripts'))
    import benchmark
    import batch_benchmark_report as native_validator
    import redis_batch_report as paired_validator
    support=module('native_profile_support',SOURCE/'scripts/native-batch-e2e.py')
    tmpfs=module('tmpfs_profile_support',SOURCE/'scripts/tmpfs-redis-diagnostic.py')
    comparison=module('profile_resource_support',SOURCE/'scripts/redis-comparison.py')
    resp=module('retained_resp_readback',RESP)
    _,manifest,features=native_validator.build_check(CLIENT_BUILD,CLIENT_BUILD,CLIENT_REV)
    require(features==[] and manifest['profile']=='release','shared client is not default release')
    roles={'old':dict(build_directory=str(SERVER_BUILD),binary_sha256=driver.SERVER_PINS['old']['binary_sha256']),
           'client':dict(build_directory=str(CLIENT_BUILD),binary_sha256=CLIENT_SHA)}
    args=SimpleNamespace(smoke=True,storage='tmpfs',client_build=CLIENT_BUILD,expected_client_revision=CLIENT_REV)
    # Instrumented run: use profile CPU sets but never label it timing-eligible
    # acceptance. The strict original workload validator is still invoked.
    driver.placement=lambda _:dict(client_cpus=[0,1],server_cpus=[2,3,4,5],separated=True,exclusive_host=False)
    modules=(benchmark,support,tmpfs,comparison,resp,native_validator,paired_validator)
    sources={str(DRIVER):DRIVER_SHA,str(Path(__file__)):sha(__file__),str(RESP):sha(RESP),str(OUT/'active_prefix.py'):sha(OUT/'active_prefix.py')}
    for loaded in list(sys.modules.values())+[support,tmpfs,comparison]:
        path=getattr(loaded,'__file__',None)
        if path and Path(path).resolve().is_relative_to(SOURCE):
            file=Path(path).resolve();relative=str(file.relative_to(SOURCE));require(sha(file)==frozen['client_sources']['sources'][relative],'loaded helper differs');sources[str(file)]=sha(file)
    protocol=dict(version=1,protocol_id='kv9-crc-write-active-prefix-cpu-v1',background_container_isolation=False,instrumented=True,throughput_acceptance=False,server_revision=driver.OLD_REVISION,
                  client_revision=CLIENT_REV,server_sha256=roles['old']['binary_sha256'],client_sha256=CLIENT_SHA,
                  measure_ms=5000,workers=64,keys=4096,batch_sizes=[1,64],value_bytes=128,read_percent=0,warmup_calls=128,
                  max_calls=10000000,deadline_ms=1500,client_cpus=[0,1],server_cpus=[2,3,4,5],profiler_cpus=PROFILER_CPUS,
                  event='cpu-clock',frequency_hz=199,call_graph='dwarf,16384',recording_seconds=20,max_perf_bytes=CAP,
                  storage=tmpfs.VOLATILE,exclusive_host=False,profiles=['point_put','batch_put64'],helper_sources=sources)
    protocol['active_prefix']=dict(prefix_helper.CONFIG,scope='Unmeasured read-only unary CLI priming before the unchanged measurement client; changes cache/CPU state; real sample coverage remains pending original decoder',helper_sha256=sha(OUT/'active_prefix.py'))
    save(OUT/'protocol.json',protocol);save(OUT/'build-bindings.json',frozen)
    save(OUT/'preflight.json',dict(complete=True,perf_backend=str(PERF),perf_sha256=sha(PERF),
         perf_version=subprocess.check_output([str(PERF),'--version'],text=True).strip(),
         sudo_perf_version=subprocess.check_output(['sudo','-n',str(PERF),'--version'],text=True).strip(),
         perf_event_paranoid=Path('/proc/sys/kernel/perf_event_paranoid').read_text().strip(),
         kptr_restrict=Path('/proc/sys/kernel/kptr_restrict').read_text().strip(),source_bindings_verified=True))
    summary=dict(complete=False,instrumented=True,throughput_acceptance=False,profiles=[])
    save(OUT/'summary.json',summary)
    original_launch=benchmark.Fixture.launch
    original_observe=driver.observe_client
    active={}
    def start_perf(fixture,directory):
        pids=[p.pid for p in fixture.nodes.values()];require(len(pids)==3,'three owned voters required')
        record=dict(complete=False,voters={str(n):dict(fixture.identities[p.pid],command_line=process(p.pid)['command_line'])for n,p in fixture.nodes.items()},
                    recorded_pid_scope=pids,anchor_before=anchor(),perf_binary_sha256=sha(PERF))
        active.update(directory=directory,record=record,process=None,log=None)
        probe=['sudo','-n',str(PERF),'stat','-e','cpu-clock','-p',','.join(map(str,pids)),'--','sleep','0.2']
        with (directory/'permission.stdout').open('x')as stdout,(directory/'permission.stderr').open('x')as stderr:
            code=subprocess.run(probe,stdout=stdout,stderr=stderr,timeout=10).returncode
        record['permission_probe']=dict(command=probe,exit_code=code);save(directory/'profile-record.json',record)
        require(code==0,'owned-PID perf permission probe failed')
        command=['sudo','-n',str(PERF),'record','--no-buildid-cache','--clockid','mono','-e','cpu-clock','-F','199',
                 '--call-graph','dwarf,16384','--no-bpf-event','--max-size','128M','-m','256','-p',','.join(map(str,pids)),
                 '-o',str(directory/'perf.data'),'--','sleep','20']
        save(directory/'perf-command.json',dict(command=command,recorded_pid_scope=pids,started_unix_ns=time.time_ns()))
        log=(directory/'perf-record.log').open('x');active['log']=log
        record['record_started_monotonic_ns']=time.monotonic_ns()
        profiler=subprocess.Popen(command,stdout=log,stderr=log,env=dict(os.environ,DEBUGINFOD_URLS=''),start_new_session=True)
        active['process']=profiler;record['launcher_pid']=profiler.pid
        deadline=time.monotonic()+3
        while time.monotonic()<deadline:
            require(profiler.poll() is None,'profiler exited before client launch')
            observed=subtree(profiler.pid);actual=[p for p in observed if p['comm']=='perf']
            if len(actual)==1:
                require(actual[0]['cpu_affinity']==PROFILER_CPUS,'profiler affinity differs')
                record['profiler_lifetimes']=observed;record['executing_profiler']=actual[0];break
            time.sleep(.02)
        else:raise RuntimeError('actual owned perf process not observed')
        time.sleep(.25)
        require(profiler.poll() is None,'profiler terminated before measurement client')
        prefix_helper.run(fixture,directory/'active-prefix',roles['old']['binary_sha256'],record['voters'],profiler)
        record['active_prefix_summary_sha256']=sha(directory/'active-prefix/summary.json')
        # Retain two fresh empty publications after prefix reads and their CLI exits.
        driver.fresh_drain(fixture,directory,'post-prefix')
        record['active_prefix_drain_sha256']=sha(directory/'post-prefix-fresh-drain.json')
        require(profiler.poll() is None,'profiler terminated during active prefix/drain')
        require(time.monotonic_ns()-record['record_started_monotonic_ns']<12_000_000_000,
                'insufficient unchanged 20-second recorder budget before client launch')
        record['prefix_sample_coverage']='pending unchanged offline containment/edge/bin checks; priming completion alone is not acceptance'
        record['before_client_launch_unix_ns']=time.time_ns();save(directory/'profile-record.json',record)
        print('LIVE: profiler '+str(actual[0]['pid'])+' voters '+','.join(map(str,pids)),flush=True)
    def launched(fixture,command,logfile,client=False):
        if client:start_perf(fixture,active['current_directory'])
        return original_launch(fixture,command,logfile,client)
    def observe(*pos,**kwargs):
        try:return original_observe(*pos,**kwargs)
        finally:finish_perf()
    def finish_perf():
        process_handle=active.get('process')
        if process_handle is None:return
        directory=active['directory'];record=active['record']
        try:
            code=process_handle.wait(timeout=25)
            record['perf_exit_code']=code;record['anchor_after']=anchor()
            require(code==0,'bounded perf recording failed')
            raw=directory/'perf.data'
            subprocess.run(['sudo','-n','chown',f'{os.getuid()}:{os.getgid()}',str(raw)],check=True,timeout=10)
            require(0<raw.stat().st_size<CAP,'perf cap reached or empty recording')
            record['raw_perf_bytes']=raw.stat().st_size;record['raw_perf_sha256']=sha(raw)
            record['profiler_lifetimes_exited']=all(not Path('/proc',str(p['pid'])).exists()for p in record['profiler_lifetimes'])
            require(record['profiler_lifetimes_exited'],'owned profiler lifetime remains')
            record['complete']=True
        finally:
            if process_handle.poll() is None:
                subprocess.run(['sudo','-n','kill','-TERM','--','-'+str(process_handle.pid)],check=False,timeout=5)
                try:process_handle.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    subprocess.run(['sudo','-n','kill','-KILL','--','-'+str(process_handle.pid)],check=False,timeout=5);process_handle.wait(timeout=5)
            if active.get('log'):active['log'].close();active['log']=None
            save(directory/'profile-record.json',record);active['process']=None
    benchmark.Fixture.launch=launched;driver.observe_client=observe
    def interrupted(signum,_):raise RuntimeError('profile interrupted by signal '+str(signum))
    signal.signal(signal.SIGTERM,interrupted);signal.signal(signal.SIGINT,interrupted)
    try:
        for api,read_api,batch,run_id in (('point_put','point_get',1,'prfput'),('batch_put','batch_get',64,'prfbat')):
            directory=OUT/api;directory.mkdir();active['current_directory']=directory
            row=dict(arm=api,server_role='old',read_api=read_api,write_api=api,workload=api,target='kv9',repeat=0,mix='write',
                     shared=dict(run_id=run_id,seed=71,workers=64,keys=4096,batch_size=batch,value_bytes=128,read_percent=0,
                                 warmup_calls=128,measure_ms=5000,max_calls=10000000,load=dict(kind='closed_loop')))
            save(directory/'profile-plan.json',row)
            print('START: instrumented '+api,flush=True)
            outcome=driver.native_trial(row,directory,args,modules,roles)
            save(directory/'fixture-result.json',outcome);summary['profiles'].append(dict(api=api,fixture_complete=outcome['complete'],
               profile_complete=read(directory/'profile-record.json')['complete'],directory=str(directory)))
            save(OUT/'summary.json',summary)
            require(outcome['complete'] and summary['profiles'][-1]['profile_complete'],'instrumented profile fixture failed; original attempt retained')
            print('TERMINAL: instrumented '+api+' complete; owned fixtures exited',flush=True)
        require(preflight(driver)==frozen and all(sha(p)==h for p,h in sources.items()),'frozen inputs changed')
        summary['complete']=True
    except BaseException as error:
        summary['failure']=repr(error);raise
    finally:
        finish_perf();benchmark.Fixture.launch=original_launch;driver.observe_client=original_observe
        summary['ended_unix_ns']=time.time_ns();save(OUT/'summary.json',summary)
    print('PASS: two instrumented native fixtures complete; perf interval analysis pending',flush=True)

if __name__=='__main__':main()
