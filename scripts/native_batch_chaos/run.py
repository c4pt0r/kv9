"""Launch only an explicitly authorized prepared point plus native batch matrix and its owned observer."""

if not __debug__:
    raise SystemExit("FAIL: Python optimization disables required checks; use PYTHONOPTIMIZE=0 without -O or -OO.")
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--plan', type=Path, required=True)
args = parser.parse_args()
PLAN = args.plan.resolve()
ROOT = PLAN.parent
OVERLAY = Path(__file__).resolve().parent
p = json.loads(PLAN.read_text())
assert p['fixture_launch_authorized'] is True and p['plan_ready'] is True and p['build_complete'] is True
assert not p['fixture_started']
subprocess.run(['python3', str(OVERLAY / 'verify-prebuilt.py'), '--plan', str(PLAN)], check=True)
env = dict(os.environ, CARGO_TARGET_DIR=p['private_cargo_target'], KUBECONFIG=p['kubeconfig'],
           KIND=p['kind'], KV9_KIND_CLUSTER=p['kind_cluster'], KV9_CHAOS_IMAGE=p['image'],
           KV9_BIN=p['production_binary']['path'], CARGO_BUILD_JOBS='8', GITHUB_SHA=p['revision'],
           KV9_NATIVE_BATCH_CHAOS_PLAN=str(PLAN), KV9_CHAOS_EVIDENCE_PARENT=p['evidence_parent'], PYTHONDONTWRITEBYTECODE='1', PYTHONOPTIMIZE='0')
command = ['taskset', '-c', '6-31', 'bash', str(OVERLAY.parent / 'chaos-mesh-e2e.sh')]
p.update(fixture_started=True, fixture_started_unix_ns=time.time_ns(), fixture_command=command,
         fixture_environment={key: env[key] for key in ('CARGO_TARGET_DIR', 'KUBECONFIG', 'KIND', 'KV9_KIND_CLUSTER',
             'KV9_CHAOS_IMAGE', 'KV9_BIN', 'GITHUB_SHA', 'KV9_NATIVE_BATCH_CHAOS_PLAN', 'KV9_CHAOS_EVIDENCE_PARENT')})


def save():
    PLAN.write_text(json.dumps(p, indent=2) + '\n')


save()
observer = None
observer_log = None
try:
    with (ROOT / 'matrix.log').open('x') as log:
        process = subprocess.Popen(command, cwd=p['worktree'], env=env, stdout=subprocess.PIPE,
                                   stderr=subprocess.STDOUT, text=True, bufsize=1)
        p['fixture_pid'] = process.pid
        save()
        for line in process.stdout:
            log.write(line)
            log.flush()
            if line.startswith('Artifacts: '):
                assert observer is None
                p['artifact'] = line.strip().split('Artifacts: ', 1)[1]
                save()
                raw = Path(p['artifact'])
                shutil.copytree(OVERLAY, raw / 'native-batch-fixture-source', ignore=shutil.ignore_patterns('__pycache__'))
                shutil.copy2(PLAN, raw / 'owned-prelaunch-plan.json')
                observer_log = (ROOT / 'observer.log').open('x')
                observer = subprocess.Popen(['taskset', '-c', '6-31', 'python3', str(OVERLAY / 'observer.py'), '--plan', str(PLAN)],
                                            env=env, stdout=observer_log, stderr=subprocess.STDOUT)
                p['observer_pid'] = observer.pid
                save()
        code = process.wait()
        p.update(fixture_exit_code=code, fixture_ended_unix_ns=time.time_ns())
        log.write(f'COMMAND_EXIT={code}\n')
finally:
    if observer:
        (Path(p['artifact']) / 'async-write-observer.stop').touch()
        try:
            p['observer_exit_code'] = observer.wait(timeout=35)
        except subprocess.TimeoutExpired:
            observer.terminate()
            p['observer_exit_code'] = observer.wait(timeout=15)
        observer_log.close()
    save()
print(json.dumps({key: p.get(key) for key in ('artifact', 'fixture_exit_code', 'observer_exit_code')}))
raise SystemExit(p.get('fixture_exit_code', 1) or p.get('observer_exit_code', 1))
