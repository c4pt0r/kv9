#!/usr/bin/env python3
"""Compile once and retain one finite standalone FNV experiment; root launches explicitly."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import signal
import stat
import subprocess
import time

SOURCE = Path(__file__).resolve().parents[1]
MODULE = SOURCE / 'crates/raft/src/storage/fnv.rs'
BENCH = SOURCE / 'experiments/fnv-interleave/bench.rs'
ALLOWED_CPUS = set(range(6, 16)) | set(range(22, 32))
FLOOR = 96 * 1024**3
OUTPUT_CAP = 512 * 1024**2
LOG_CAP = 4 * 1024**2
CASES = [('equal-0', [0]*4), ('equal-32', [32]*4), ('equal-205', [205]*4),
         ('equal-206', [206]*4), ('equal-10600', [10600]*4),
         ('equal-10601', [10601]*4), ('equal-65536', [65536]*4),
         ('skew-point-batch', [205,205,10600,205]),
         ('skew-empty-point-batch', [0,205,10600,32])]


def require(ok, message):
    if not ok:
        raise ValueError(message)


def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def save(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2, sort_keys=True)
        stream.write('\n')


def process_identity(pid):
    root = Path('/proc', str(pid))
    fields = (root/'stat').read_text().rsplit(')', 1)[1].split()
    exe = (root/'exe').stat()
    return dict(pid=pid, start_ticks=int(fields[19]),
                boot_id=Path('/proc/sys/kernel/random/boot_id').read_text().strip(),
                cpu_affinity=sorted(os.sched_getaffinity(pid)),
                executable=os.readlink(root/'exe'), executable_device=exe.st_dev,
                executable_inode=exe.st_ino, executable_bytes=exe.st_size)


def guard(out):
    fs = os.statvfs(out)
    require(fs.f_bavail * fs.f_frsize >= FLOOR, '96 GiB available floor reached')
    total = 0
    for parent, _, files in os.walk(out, followlinks=False):
        for name in files:
            p = Path(parent)/name
            s = p.lstat()
            require(stat.S_ISREG(s.st_mode), 'Unexpected output type')
            total += s.st_blocks*512
    require(total <= OUTPUT_CAP, '512 MiB output allocation cap reached')
    return fs.f_bavail * fs.f_frsize


def child(stage, argv, out, cpu, timeout):
    record = dict(stage=stage, argv=[str(x) for x in argv], started_ns=time.time_ns(),
                  complete=False, identity=None, identity_capture_error=None)
    process = None
    try:
        with (out/(stage+'.stdout')).open('xb') as stdout, (out/(stage+'.stderr')).open('xb') as stderr:
            process = subprocess.Popen(record['argv'], cwd=SOURCE, stdout=stdout, stderr=stderr,
                                       env=dict(os.environ, GIT_OPTIONAL_LOCKS='0'), start_new_session=True)
            record['pid'] = process.pid
            try:
                record['identity'] = process_identity(process.pid)
            except (FileNotFoundError, ProcessLookupError) as error:
                record['identity_capture_error'] = repr(error)
            save(out/(stage+'-invocation.json'), record)
            deadline = time.monotonic()+timeout
            while process.poll() is None:
                require(time.monotonic() < deadline, stage+' deadline exceeded')
                guard(out)
                require((out/(stage+'.stdout')).stat().st_size <= LOG_CAP and
                        (out/(stage+'.stderr')).stat().st_size <= LOG_CAP, stage+' output cap reached')
                time.sleep(0.1)
            record['exit_code'] = process.wait()
            require(record['exit_code'] == 0, stage+' failed')
            require((out/(stage+'.stdout')).stat().st_size <= LOG_CAP and
                    (out/(stage+'.stderr')).stat().st_size <= LOG_CAP, stage+' terminal output cap reached')
            guard(out)
            if stage in ('compile', 'measure'):
                require(record['identity'] is not None and record['identity']['cpu_affinity'] == [cpu],
                        'Timed/compiler child identity or CPU unavailable')
                actual = Path(argv[0]).stat()
                require((record['identity']['executable_device'], record['identity']['executable_inode']) ==
                        (actual.st_dev, actual.st_ino), 'Executing child is not the bound executable')
            record['complete'] = True
    except BaseException as error:
        record['error'] = repr(error)
        raise
    finally:
        if process is not None:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait(timeout=3)
            record.update(exit_code=process.poll(), reaped=process.poll() is not None,
                          absent=not Path('/proc', str(process.pid)).exists())
        record['ended_ns'] = time.time_ns()
        for suffix in ('stdout','stderr'):
            p = out/(stage+'.'+suffix)
            if p.exists():
                record[suffix] = dict(bytes=p.stat().st_size, sha256=sha(p))
        save(out/(stage+'-result.json'), record)
    require(record['reaped'] and record['absent'], 'Owned child still exists')
    return record


def validate_rows(path):
    rows = [json.loads(line) for line in path.read_text().splitlines()]
    require(len(rows) == 110 and rows[0]['kind'] == 'protocol' and rows[-1]['kind'] == 'complete', 'Missing/extra output rows')
    require(rows[0] == dict(kind='protocol',version=1,scope='fnv-four-frame-kernel-only',cases=9,
                           repeats_per_order=3,target_bytes=64*1024**2,empty_groups=1_000_000,
                           min_groups=256,max_groups=1_000_000,warmup_groups=32), 'Protocol differs')
    expected = []
    for repeat in range(3):
        for order in ('forward','reverse'):
            jobs = [(name,lengths,impl) for name,lengths in CASES for impl in ('serial','interleaved')]
            if order == 'reverse':
                jobs.reverse()
            expected.extend((repeat,order,ordinal,*job) for ordinal,job in enumerate(jobs))
    for row, (repeat,order,ordinal,name,lengths,impl) in zip(rows[1:-1],expected,strict=True):
        require(all(row[k] == v for k,v in dict(kind='measurement',repeat=repeat,order=order,ordinal=ordinal,
                                              case=name,lengths=lengths,implementation=impl).items()), 'Row/order identity differs')
        size = sum(lengths)
        groups = max(256,min(1_000_000,64*1024**2//size)) if size else 1_000_000
        require(row['groups']==groups and row['bytes_per_group']==size and row['input_bytes']==size*groups,
                'Byte/group count identity differs')
        require(type(row['elapsed_ns']) is int and row['elapsed_ns'] > 0, 'Invalid elapsed time')
        require(math.isclose(row['ns_per_group'],row['elapsed_ns']/groups,rel_tol=1e-8,abs_tol=1e-8), 'ns/group arithmetic differs')
        if size:
            require(math.isclose(row['mib_per_second'],row['input_bytes']*1e9/row['elapsed_ns']/1024**2,rel_tol=1e-8,abs_tol=1e-8), 'MiB/s arithmetic differs')
        else:
            require(row['mib_per_second'] is None, 'Zero-byte throughput must be unavailable')
    require(rows[-1]['complete'] and rows[-1]['rows']==108 and
            rows[-1]['input_bytes']==sum(r['input_bytes'] for r in rows[1:-1]), 'Terminal byte accounting differs')
    return rows[1:-1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--rustc', type=Path, required=True, help='Exact installed rustc, not a rustup proxy')
    parser.add_argument('--expected-module-sha256', required=True)
    parser.add_argument('--expected-bench-sha256', required=True)
    parser.add_argument('--cpu', type=int, default=6)
    args = parser.parse_args()
    require(__debug__, 'Use PYTHONOPTIMIZE=0')
    require(args.cpu in ALLOWED_CPUS and args.cpu in os.sched_getaffinity(0), 'Helper CPU unavailable')
    require(args.output.is_absolute() and not args.output.exists(), 'Fresh absolute output required')
    require(args.rustc.is_absolute() and args.rustc.resolve()==args.rustc and args.rustc.is_file(), 'Exact rustc path required')
    args.output.mkdir(mode=0o700)
    out = args.output
    result = dict(complete=False, scope='Standalone FNV kernel only; no database QPS, Ready grouping or promotion',
                  pid=os.getpid(), started_ns=time.time_ns(), children=[], runtime_source_changes=False)
    try:
        os.sched_setaffinity(0,{args.cpu})
        result['harness_identity'] = process_identity(os.getpid())
        result['available_before_bytes'] = guard(out)
        sources = {str(p):sha(p) for p in (MODULE,BENCH,Path(__file__).resolve(),SOURCE/'rust-toolchain.toml',SOURCE/'Cargo.toml')}
        require(sources[str(MODULE)]==args.expected_module_sha256 and sources[str(BENCH)]==args.expected_bench_sha256,
                'Expected module/benchmark source differs')
        save(out/'sources-before.json',sources)
        result['rustc_sha256'] = sha(args.rustc)
        result['uname'] = list(os.uname())
        result['cpu'] = args.cpu
        cpuinfo = Path('/proc/cpuinfo').read_text()
        selected_cpu = [block for block in cpuinfo.split('\n\n') if any(
            line.startswith('processor') and line.split(':',1)[1].strip()==str(args.cpu)
            for line in block.splitlines())]
        require(len(selected_cpu)==1, 'Selected CPU description unavailable')
        with (out/'selected-cpuinfo.txt').open('x') as stream:
            stream.write(selected_cpu[0]+'\n')
        result['selected_cpuinfo_sha256'] = sha(out/'selected-cpuinfo.txt')
        for stage, argv in [('compiler-version',[args.rustc,'-Vv']),
                            ('source-revision',['git','rev-parse','HEAD']),
                            ('source-status',['git','status','--porcelain','--untracked-files=all'])]:
            result['children'].append(child(stage,argv,out,args.cpu,15))
        binary = out/'fnv-interleave-bench'
        command = [args.rustc,'--edition=2021','--crate-name','fnv_interleave_bench',BENCH,
                   '-C','opt-level=3','-C','lto=thin','-C','codegen-units=1','-C','panic=unwind',
                   '-C','debuginfo=0','-C','debug-assertions=no','-C','overflow-checks=no',
                   '-C','strip=debuginfo','-o',binary]
        result['children'].append(child('compile',command,out,args.cpu,120))
        result['binary_sha256'] = sha(binary)
        result['binary_bytes'] = binary.stat().st_size
        result['children'].append(child('measure',[binary],out,args.cpu,65))
        rows = validate_rows(out/'measure.stdout')
        pooled=[]
        for order in ('forward','reverse','both'):
            for name,_ in CASES:
                pair={}
                for impl in ('serial','interleaved'):
                    selected=[r for r in rows if r['case']==name and r['implementation']==impl and (order=='both' or r['order']==order)]
                    elapsed=sum(r['elapsed_ns'] for r in selected);groups=sum(r['groups'] for r in selected);size=sum(r['input_bytes'] for r in selected)
                    pair[impl]=dict(elapsed_ns=elapsed,groups=groups,input_bytes=size,ns_per_group=elapsed/groups,
                                    mib_per_second=size*1e9/elapsed/1024**2 if size else None)
                pooled.append(dict(order=order,case=name,implementations=pair,
                                   serial_over_interleaved_ns_ratio=pair['serial']['ns_per_group']/pair['interleaved']['ns_per_group']))
        require({p:sha(Path(p)) for p in sources}==sources and sha(binary)==result['binary_sha256'] and sha(args.rustc)==result['rustc_sha256'], 'Bound source/binary/compiler changed')
        save(out/'sources-after.json',sources)
        save(out/'summary.json',dict(complete=True,scope=result['scope'],rows=rows,pooled=pooled,
                                    latency_scope='Elapsed time per four-frame kernel group, no individual-call histogram/quantile',
                                    input_scope='Deterministic reused warm buffers; no original WAL payload reads or Ready-group claim'))
        result['available_after_bytes']=guard(out)
        result['complete']=True
    except BaseException as error:
        result['failure']=repr(error)
    finally:
        result['ended_ns']=time.time_ns()
        result['child_result_paths'] = [str(p) for p in sorted(out.glob('*-result.json'))]
        result['all_recorded_children_reaped'] = all(
            json.loads(Path(p).read_text()).get('reaped') is True
            for p in result['child_result_paths'])
        save(out/'result.json',result)
    print(json.dumps(result,sort_keys=True))
    return 0 if result['complete'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
