#!/usr/bin/env python3
"""Finite production-API syscall acceptance; root builds the probe separately.

One owned probe at a time. No Cargo, service, checkpoint premise, power-cut
simulation, old payload access, cleanup or implicit retry is performed here.
"""
import argparse
import errno
import hashlib
import json
import os
from pathlib import Path
import re
import resource
import signal
import stat
import subprocess
import time

CPUS=list(range(6,16))+list(range(22,32))
CUTS=('successor_before','successor_after','parent_before','parent_after')
OUTPUT_CAP=64*1024**2
FILE_CAP=1024**2
STREAM='47'*16

def need(ok,message):
    if not ok:raise ValueError(message)
def sha(path):
    with Path(path).open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()
def save(path,value):
    path.write_text(json.dumps(value,indent=2,sort_keys=True)+'\n')
def ticks(pid):return int(Path(f'/proc/{pid}/stat').read_text().rsplit(')',1)[1].split()[19])
def allocated(root):return sum(p.lstat().st_blocks*512 for p in root.rglob('*'))
def run(argv,env,folder,timeout=30):
    folder.mkdir(exist_ok=False)
    started=time.time_ns();child=None;result=dict(complete=False,argv=list(map(str,argv)),started_ns=started)
    def limits():
        os.sched_setaffinity(0,CPUS)
        resource.setrlimit(resource.RLIMIT_FSIZE,(FILE_CAP,FILE_CAP))
        resource.setrlimit(resource.RLIMIT_CORE,(0,0))
        resource.setrlimit(resource.RLIMIT_CPU,(30,30))
    try:
        with (folder/'stdout').open('xb') as out,(folder/'stderr').open('xb') as err:
            child=subprocess.Popen(list(map(str,argv)),stdout=out,stderr=err,env=env,
                start_new_session=True,preexec_fn=limits)
            result.update(pid=child.pid,start_ticks=ticks(child.pid),boot_id=Path('/proc/sys/kernel/random/boot_id').read_text().strip())
            save(folder/'invocation.json',result)
            child.wait(timeout=timeout)
            result.update(complete=True,exit_code=child.returncode)
    except BaseException as error:
        result['failure']=repr(error)
        raise
    finally:
        if child is not None:
            if child.poll() is None:
                need(ticks(child.pid)==result['start_ticks'],'owned child identity changed before stop')
                os.killpg(child.pid,signal.SIGKILL)
            child.wait(timeout=10)
            result.update(exit_code=child.returncode,reaped=True,pid_absent=not Path(f'/proc/{child.pid}').exists())
        result['ended_ns']=time.time_ns();save(folder/'result.json',result)
    return result

def successful(row):
    return row['event']=='fsync' and row['real_called'] is True and row['real_result']==0 and row['returned']==0 and not row['omitted'] and not row['injected']
def segment(data,sequence):return data/'catalog.segments'/STREAM/f'{sequence:020}.wal'
def chain(directory):return [str(directory),*map(str,directory.parents)]
def trace(path,pid):
    need(path.is_file() and not path.is_symlink() and path.stat().st_size<=FILE_CAP,'missing/bounded shim trace')
    rows=[json.loads(line) for line in path.read_text().splitlines()]
    need(0<len(rows)<=4096 and [r['seq'] for r in rows]==list(range(1,len(rows)+1)),'trace sequence incomplete')
    need(all(r['pid']==pid for r in rows),'trace owner differs')
    need(all(a['monotonic_ns']<=b['monotonic_ns'] for a,b in zip(rows,rows[1:])),'trace time reversed')
    return rows
def validate_trace(rows,data,cut=None):
    need(rows[0]['event']=='phase' and rows[0]['phase']=='start' and rows[-1]['event']=='end' and rows[-1]['phase']=='done','probe phase/end coverage')
    parent=segment(data,1).parent;ancestors=chain(parent);topology=data/'catalog.wal'
    def phase(name):return [r for r in rows if r['phase']==name and r['event'] in ('fsync','rename')]
    for name,number in [('create',1),('recover-initial',1),('recover-final',1 if cut else 3),*([('recover-after-fault',1)] if cut else [])]:
        found=phase(name)
        dirs=[r['path'] for r in found if successful(r) and stat.S_ISDIR(r['mode'])]
        expected=ancestors+[str(data)] if name=='create' else [str(data)]+ancestors
        need(dirs==expected,f'{name}: missing full ancestor sync sequence')
        files=[r['path'] for r in found if successful(r) and stat.S_ISREG(r['mode'])]
        expected_files=[str(segment(data,number)),str(topology)] if name=='create' else [str(topology),str(segment(data,number))]
        need(files==expected_files,f'{name}: file sync coverage differs')
        need(all(successful(r) for r in found),f'{name}: unsuccessful or omitted fsync')
    if cut:
        selected=phase('rotate-1');injected=[r for r in rows if r['injected']]
        need(len(injected)==1 and injected[0]['phase']=='rotate-1','one exact rotation injection required')
        fault=injected[0];want=segment(data,2) if cut.startswith('successor_') else parent
        need(fault['path']==str(want) and fault['errno']==errno.EIO and fault['returned']==-1,'wrong EIO target/result')
        need(fault['real_called']==cut.endswith('_after') and fault['real_result']==0,'before/after syscall cut differs')
        want_paths=[str(segment(data,1)),str(segment(data,2))]+([str(parent)] if cut.startswith('parent_') else [])
        need([r['path'] for r in selected]==want_paths and all(r['event']=='fsync' for r in selected),'failed successor unexpectedly published topology')
        need(all(successful(r) for r in selected[:-1]) and selected[-1] is fault,'fault did not end selected rotation boundary')
        need(not phase('fence-check'),'fenced handle performed new publication I/O')
    else:
        need(not any(r['injected'] for r in rows),'positive case injected a fault')
        for number in (2,3):
            found=phase(f'rotate-{number-1}')
            expected=[('fsync',str(segment(data,number-1))),('fsync',str(segment(data,number))),
                      ('fsync',str(parent)),('fsync',str(topology.with_suffix('.topology.tmp'))),
                      ('rename',str(topology)),('fsync',str(data))]
            need([(r['event'],r['path']) for r in found]==expected,'successor file/parent/topology order or directory scope differs')
            need(sum(successful(r) and r['path']==str(parent) for r in found)==1,'rotation missing required real immediate-parent sync')
            need(all(successful(r) if r['event']=='fsync' else r['real_called'] and r['returned']==0 for r in found),'successful rotation contains unsuccessful syscall')
            need([r['path'] for r in found if stat.S_ISDIR(r['mode'])]==[str(parent),str(data)],'rotation resynchronized ancestors or omitted topology parent')
    need(not any(r['omitted'] for r in rows),'required real fsync omitted')

def data_inventory(data):
    files=[]
    for p in sorted(data.rglob('*')):
        st=p.lstat();need(not p.is_symlink(),'unexpected probe symlink')
        if p.is_file():
            need(st.st_size<=FILE_CAP,'probe data member bound')
            files.append(dict(path=str(p),bytes=st.st_size,sha256=sha(p)))
    need(len(files)<=32 and sum(x['bytes'] for x in files)<=FILE_CAP,'probe data scope exceeded')
    return files

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--probe',type=Path,required=True)
    parser.add_argument('--expected-probe-sha256',required=True)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    repo=Path(__file__).resolve().parents[1]
    output=args.output.absolute();binary=args.probe.resolve()
    need(output.is_relative_to('/tmp') and not output.exists() and output.parent==output.parent.resolve(),'fresh ordinary /tmp output required')
    need(re.fullmatch(r'[A-Za-z0-9_./-]+',str(output)) is not None,'output path unsupported by trace encoding')
    need(sha(binary)==args.expected_probe_sha256,'probe binary differs from root build binding')
    # Finite small output headroom only; this does not alter any benchmark gate.
    v=os.statvfs(output.parent);need(v.f_bavail*v.f_frsize>=OUTPUT_CAP,'small probe output reservation unavailable')
    output.mkdir()
    sources=[repo/'crates/engine/src/wal_segment.rs',repo/'crates/engine/src/wal_stream.rs',
             repo/'crates/engine/examples/published_directory_probe.rs',
             repo/'scripts/published_directory_shim.c',Path(__file__).resolve()]
    pins={str(p):sha(p) for p in sources};save(output/'source-before.json',pins)
    report=dict(complete=False,started_ns=time.time_ns(),probe_path=str(binary),probe_sha256=sha(binary),
        cases=[],controls=[],scope='Production public API / real syscall-return checks only; no power-cut durability or benchmark acceptance')
    save(output/'report.json',report)
    try:
        env=dict(os.environ);env.pop('LD_PRELOAD',None);env.pop('LD_AUDIT',None)
        for key in list(env):
            if key.startswith('KV9_PD_'):env.pop(key)
        shim=output/'published_directory_shim.so'
        compiled=run(['/usr/bin/cc','-std=c11','-O2','-Wall','-Wextra','-Werror','-shared','-fPIC',
            repo/'scripts/published_directory_shim.c','-ldl','-o',shim],env,output/'shim-build',60)
        need(compiled['exit_code']==0,'shim build failed; original compiler logs retained')
        report['shim_sha256']=sha(shim)
        for label,mode,expect in [('baseline','none','success'),*[(c,c,'fault') for c in CUTS],
                                 ('missing-shim','none','success'),('wrong-target','successor_before','fault'),
                                 ('missing-directory-sync','skip_ancestor','success'),
                                 ('missing-successor-parent-sync','skip_successor_parent','success')]:
            case=output/label;case.mkdir();data=case/'data';parent=segment(data,1).parent
            child_env=dict(env,LD_PRELOAD=str(shim),KV9_PD_MODE=mode,KV9_PD_SUCCESSOR=str(segment(data,2)),
                KV9_PD_PARENT=str(parent),KV9_PD_TOPOLOGY=str(data/'catalog.wal'),
                KV9_PD_TRACE=str(case/'trace.jsonl'),KV9_PD_SKIP_ANCESTOR=str(case))
            if label=='missing-shim':child_env.pop('LD_PRELOAD')
            if label=='wrong-target':child_env['KV9_PD_SUCCESSOR']=str(parent/'unrelated.wal')
            result=run([binary,data,expect],child_env,case/'process')
            need(result['reaped'] and result['pid_absent'],'owned probe lifetime did not exit')
            stdout=(case/'process/stdout').read_text();stderr=(case/'process/stderr').read_text()
            if label=='missing-shim':
                need(result['exit_code']!=0 and 'PUBLISHED_DIRECTORY_MISSING_SHIM:' in stderr and not data.exists() and not (case/'trace.jsonl').exists(),'missing-shim control failed for an unrelated reason')
                report['controls'].append(dict(name=label,rejected=True,process=result))
            elif label=='wrong-target':
                rows=trace(case/'trace.jsonl',result['pid'])
                need(result['exit_code']!=0 and 'PUBLISHED_DIRECTORY_EXPECTED_EIO:' in stderr and not any(r['injected'] for r in rows),'wrong-target control failed for an unrelated reason')
                report['controls'].append(dict(name=label,rejected=True,process=result))
            else:
                need(result['exit_code']==0,'probe failed; original stdout/stderr and payload retained')
                value=json.loads(stdout);need(value['complete'] is True and value['process_id']==result['pid'] and value['process_start_ticks']==result['start_ticks'] and value['boot_id']==result['boot_id'],'probe report lifetime identity differs')
                rows=trace(case/'trace.jsonl',result['pid'])
                if label in ('missing-directory-sync','missing-successor-parent-sync'):
                    omitted=[r for r in rows if r['omitted']]
                    want_path,want_phase,want_error=(str(case),'create','create: missing full ancestor sync sequence') if label=='missing-directory-sync' else (str(parent),'rotate-1','rotation missing required real immediate-parent sync')
                    need(len(omitted)==1 and omitted[0]['path']==want_path and omitted[0]['phase']==want_phase,'omission control target differs')
                    try:validate_trace(rows,data)
                    except ValueError as error:
                        need(str(error)==want_error,'directory control failed for unrelated predicate')
                        report['controls'].append(dict(name=label,rejected=True,reason=str(error),process=result))
                    else:raise ValueError('reader accepted a missing required ancestor fsync')
                else:
                    validate_trace(rows,data,mode if expect=='fault' else None)
                    need(value['old_acknowledged']==[1,7,11] and value['recovered_final']==([1,7,11,21] if expect=='fault' else [1,7,11,21,31]),'acknowledged recovery coverage differs')
                    if expect=='fault':
                        need(value['failed_rotation_write_acknowledged'] is False and value['fenced_write_error'] and value['fenced_rotation_error'] and value['recovered_before_new_write']==[1,7,11],'fault fencing or old-write recovery missing')
                    report['cases'].append(dict(name=label,process=result,probe=value,trace_sha256=sha(case/'trace.jsonl'),trace_events=len(rows)))
                save(case/'probe-report.json',value)
            if data.exists():save(case/'data-inventory.json',data_inventory(data))
            need(allocated(output)<=OUTPUT_CAP,'bounded probe output allocation exceeded')
            save(output/'report.json',report)
        after={str(p):sha(p) for p in sources};save(output/'source-after.json',after)
        need(after==pins and sha(binary)==args.expected_probe_sha256,'source or retained probe changed during acceptance')
        need(len(report['cases'])==5 and len(report['controls'])==4,'finite case inventory incomplete')
        report['complete']=True
    except BaseException as error:
        report['failure']=repr(error)
        raise
    finally:
        report['ended_ns']=time.time_ns();save(output/'report.json',report)
    print(json.dumps(dict(complete=True,cases=len(report['cases']),controls=len(report['controls']),report_sha256=sha(output/'report.json'))))

if __name__=='__main__':main()
