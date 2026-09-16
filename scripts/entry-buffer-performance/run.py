#!/usr/bin/env python3
"""Execute the fixed single-buffer write screen; no read stage runs after a write failure."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

S = Path(sys.argv[1]).resolve()
stage = sys.argv[2]
assert stage in ('prepare', 'counting', 'timing')
H = Path(__file__).resolve().parent
load = lambda p: json.loads(p.read_text())
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
plan = load(S / 'plan.json')
build = load(S / 'build-summary.json')
assert build['complete'] and sha(S / 'plan.json') == build['plan_sha256']
assert sorted(os.sched_getaffinity(0)) == list(range(6, 16)) + list(range(22, 32))
for path, digest in build['sources'].items():
    assert sha(Path(path)) == digest, path
outroot = S / 'runs'
if stage == 'prepare':
    outroot.mkdir(exist_ok=False)
else:
    assert load(S / 'inputs-accepted.json')['complete']
if stage == 'timing':
    assert load(S / 'allocation-analysis.json')['accepted']
summary = dict(complete=False, started_ns=time.time_ns(), stage=stage, processes=[],
               plan_sha256=sha(S / 'plan.json'),
               tools={str(H / p): sha(H / p) for p in ('run.py', 'validate.py')})
for path, digest in summary['tools'].items():
    dest = S / 'tools' / Path(path).name
    if not dest.exists():
        dest.parent.mkdir(exist_ok=True)
        shutil.copyfile(path, dest)
    assert sha(dest) == digest

def execute(name, profile, arm, mode, case, order):
    binary = build['arms'][arm][profile]
    assert sha(Path(binary['path'])) == binary['sha256']
    free = {p: shutil.disk_usage(p).free for p in ('/home/dongxu/kv9', '/mnt/data')}
    assert min(free.values()) >= plan['minimum_free_bytes']
    argv = ['taskset', '-c', str(plan['cpu']), binary['path'], str(S),
            str(outroot / (name + '.json')), mode, str(case), str(order)]
    row = dict(name=name, profile=profile, arm=arm, mode=mode, case=case, order=order,
               argv=argv, complete=False, binary_sha256=binary['sha256'],
               started_ns=time.time_ns(), free_bytes=free, shared_host=True,
               loadavg=Path('/proc/loadavg').read_text().strip())
    process = None
    try:
        with (outroot / (name + '.stdout')).open('x') as out, (outroot / (name + '.stderr')).open('x') as err:
            process = subprocess.Popen(argv, stdout=out, stderr=err)
            row['pid'] = process.pid
            row['start_ticks'] = int(Path(f'/proc/{process.pid}/stat').read_text().rsplit(')', 1)[1].split()[19])
            (outroot / (name + '-launch.json')).write_text(json.dumps(row, indent=2) + '\n')
            row['exit_code'] = process.wait(timeout=plan['process_timeout_seconds'])
        assert row['exit_code'] == 0, name
        path = outroot / (name + '.json')
        assert path.stat().st_size <= plan['process_output_max_bytes']
        x = load(path)
        assert x['complete'] and x['mode'] == mode
        assert x['input_plan_sha256'] == build['plan_sha256'] and x['corpus_sha256'] == plan['corpus_sha256']
        assert x.get('counting_build', False) == (profile == 'counting')
        if mode == 'measure':
            assert (x['case'], x['order'], x['backend'], x['spec']) == (case, order, arm, plan['cases'][case])
        assert sha(Path(binary['path'])) == binary['sha256']
        row.update(complete=True, result_sha256=sha(path), result_bytes=path.stat().st_size)
    finally:
        if process is not None and process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=10)
        row['ended_ns'] = time.time_ns()
        (outroot / (name + '-terminal.json')).write_text(json.dumps(row, indent=2) + '\n')
        summary['processes'].append(row)
        (S / (stage + '-progress.json')).write_text(json.dumps(summary, indent=2) + '\n')

try:
    if stage == 'prepare':
        for profile in ('timing', 'counting'):
            for order, arm in enumerate(('baseline', 'candidate')):
                execute(f'prepare-{profile}-{arm}', profile, arm, 'prepare', 0, order)
    else:
        sequence = plan['orders'] if stage == 'timing' else ['baseline', 'candidate']
        for case in range(len(plan['cases'])):
            for order, arm in enumerate(sequence):
                execute(f'case-{case:02d}-{stage}-{order}-{arm}', stage, arm, 'measure', case, order)
            print(json.dumps({'case': case, 'profile': stage, 'complete': True, 'processes': len(summary['processes'])}), flush=True)
    assert len(summary['processes']) == {'prepare': 4, 'counting': 16, 'timing': 32}[stage]
    assert all(p['complete'] for p in summary['processes'])
    for path, digest in {**build['sources'], **summary['tools']}.items():
        assert sha(Path(path)) == digest, path
    summary['complete'] = True
except BaseException as error:
    summary['failure'] = repr(error)
    raise
finally:
    summary['ended_ns'] = time.time_ns()
    (S / (stage + '-summary.json')).write_text(json.dumps(summary, indent=2) + '\n')
