#!/usr/bin/env python3
"""Bounded local development checks of fixed Rust inputs; not release retention."""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import time

ROOT = Path('/home/dongxu/kv9')
OUT = Path(__file__).resolve().parent
TARGET = Path('/home/dongxu/kv9/target')
GIB = 1024**3


def save(name, obj):
    (OUT/name).write_text(json.dumps(obj, indent=2)+'\n')


def inputs():
    names = subprocess.check_output(['git','ls-files','--cached','--others','--exclude-standard','-z'],cwd=ROOT).decode().split('\0')
    return {n:hashlib.sha256((ROOT/n).read_bytes()).hexdigest() for n in sorted(set(names))
            if n and (n.startswith(('crates/','src/','proto/','.cargo/')) or n in ('Cargo.toml','Cargo.lock','rust-toolchain.toml'))}


def available():
    s=os.statvfs(OUT);return s.f_bavail*s.f_frsize


assert not (OUT/'result.json').exists(), 'fresh test output required'
before=inputs();save('runtime-inputs.json',before)
baseline=available()
commands=[
 ['cargo','test','--offline','--locked','-j','4','-p','kv9-raft','--lib','--features','write-path-diagnostics','transport::tests::three_nodes_over_real_tcp_elect_replicate_and_discover','--','--exact','--nocapture'],
 ['cargo','fmt','--all','--check'],
]
result=dict(complete=False,source=str(ROOT),scope='Fixed Cargo/crates/src/proto inputs, local development checks only.',
            available_before=baseline,host_floor_bytes=8*GIB,max_decrease_bytes=16*GIB,commands=[])
env=dict(os.environ,CARGO_TARGET_DIR=str(TARGET),CARGO_BUILD_JOBS='4',CARGO_NET_OFFLINE='true',PYTHONDONTWRITEBYTECODE='1')
try:
    with (TARGET/'.kv9-retained-build.lock').open('a') as lock:
        fcntl.flock(lock,fcntl.LOCK_EX)
        lock_stat=os.fstat(lock.fileno())
        assert (lock_stat.st_dev,lock_stat.st_ino)==((TARGET/'.kv9-retained-build.lock').stat().st_dev,(TARGET/'.kv9-retained-build.lock').stat().st_ino)
        result['lock_identity']=dict(device=lock_stat.st_dev,inode=lock_stat.st_ino)
        for ordinal,command in enumerate(commands, start=1):
            assert inputs()==before
            assert (lock_stat.st_dev,lock_stat.st_ino)==((TARGET/'.kv9-retained-build.lock').stat().st_dev,(TARGET/'.kv9-retained-build.lock').stat().st_ino)
            assert available()>=8*GIB and baseline-available()<=16*GIB
            row=dict(ordinal=ordinal,argv=command,started_unix_ns=time.time_ns(),samples=[])
            result['commands'].append(row)
            print(json.dumps(dict(starting=ordinal,command=command)),flush=True)
            with (OUT/f'check-{ordinal}.log').open('x') as log:
                child=subprocess.Popen(['taskset','-c','6-15,22-31',*command],cwd=ROOT,env=env,
                                       stdout=log,stderr=subprocess.STDOUT,start_new_session=True,pass_fds=(lock.fileno(),))
                row['pid']=child.pid
                try:
                    while child.poll() is None:
                        current=available();row['samples'].append(dict(unix_ns=time.time_ns(),available=current))
                        assert current>=8*GIB and baseline-current<=16*GIB,'development space guard'
                        assert time.time_ns()-row['started_unix_ns']<1200*10**9,'development timeout'
                        try:child.wait(timeout=5)
                        except subprocess.TimeoutExpired:pass
                finally:
                    if child.poll() is None:
                        os.killpg(child.pid,signal.SIGKILL)
                    child.wait()
                    row.update(exit_code=child.returncode,ended_unix_ns=time.time_ns(),runtime_inputs_unchanged=inputs()==before)
                    save('result.json',result)
                assert child.returncode==0 and row['runtime_inputs_unchanged'],f'check {ordinal} failed; original log retained'
            print(json.dumps(dict(passed=ordinal,available=available())),flush=True)
        result['complete']=True
except BaseException as error:
    result['failure']=repr(error)
    raise
finally:
    result['available_after']=available();save('result.json',result)
