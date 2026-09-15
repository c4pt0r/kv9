#!/usr/bin/env python3
"""One finite Cargo invocation, retaining each actual outcome and owned child."""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

HERE = Path(__file__).parent
FIXTURE = HERE.parent / 'rpds'
TARGET = Path('/home/dongxu/kv9/target')
name, *command = sys.argv[1:]
assert name and '/' not in name and command
output = HERE / name
output.mkdir()
env = dict(os.environ, CARGO_TARGET_DIR=str(TARGET), CARGO_BUILD_JOBS='4',
           TMPDIR=str(output), PYTHONDONTWRITEBYTECODE='1')


def pin(path):
    b = path.read_bytes()
    return dict(bytes=len(b), sha256=hashlib.sha256(b).hexdigest())


def save(filename, value):
    with (output/filename).open('x') as f:
        json.dump(value, f, indent=2, sort_keys=True); f.write('\n')


def available(path):
    s = os.statvfs(path)
    return s.f_bavail*s.f_frsize


with (TARGET/'.kv9-retained-build.lock').open('a+b') as lock:
    fcntl.flock(lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
    baselines = {p: available(p) for p in ['/', str(HERE)]}
    assert all(v >= 8*1024**3 for v in baselines.values())
    protected = json.loads((HERE/'protected-source-before.json').read_bytes())
    assert all(pin(Path(p)) == value for p, value in protected.items())
    save('invocation.json', dict(argv=command, cwd=str(FIXTURE), target=str(TARGET),
         inherited_lock=str(TARGET/'.kv9-retained-build.lock'), available_before=baselines,
         floor_bytes=8*1024**3, max_decrease_bytes=16*1024**3, timeout_seconds=1200,
         manifest_before=pin(FIXTURE/'Cargo.toml'), lockfile_before=pin(FIXTURE/'Cargo.lock')))
    child = None
    code = None
    error = None
    started = time.monotonic()
    try:
        with (output/'cargo.log').open('x') as log:
            child = subprocess.Popen(command, cwd=FIXTURE, env=env, stdout=log,
                stderr=subprocess.STDOUT, pass_fds=(lock.fileno(),), start_new_session=True)
            fields = Path('/proc', str(child.pid), 'stat').read_text().rsplit(')',1)[1].split()
            save('child.json', dict(pid=child.pid, start_ticks=int(fields[19]),
                 boot_id=Path('/proc/sys/kernel/random/boot_id').read_text().strip()))
            while child.poll() is None:
                assert time.monotonic()-started < 1200, 'Cargo deadline'
                assert (output/'cargo.log').stat().st_size < 16*1024**2, 'log bound'
                for path, baseline in baselines.items():
                    now = available(path)
                    assert now >= 8*1024**3 and baseline-now <= 16*1024**3, 'disk guard'
                time.sleep(.25)
            code = child.wait()
    except BaseException as exc:
        error = repr(exc)
    finally:
        if child is not None and child.poll() is None:
            os.killpg(child.pid, signal.SIGTERM)
            try: child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(child.pid, signal.SIGKILL); child.wait(timeout=5)
        after = {p: pin(Path(p)) for p in protected}
        save('protected-source-after.json', after)
        terminal = dict(complete=code == 0 and error is None and after == protected,
            exit_code=code, error=error, reaped=child is not None and child.poll() is not None,
            pid=child.pid if child is not None else None, protected_unchanged=after == protected,
            elapsed_seconds=time.monotonic()-started, available_after={p:available(p) for p in baselines},
            manifest_after=pin(FIXTURE/'Cargo.toml'), lockfile_after=pin(FIXTURE/'Cargo.lock'),
            log=pin(output/'cargo.log'))
        save('terminal.json', terminal)
        print(json.dumps(dict(attempt=name, terminal=terminal)), flush=True)
sys.exit(0 if terminal['complete'] else 1)
