#!/usr/bin/env python3
"""Run only the prospectively declared engine-interface diagnostic."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

S = Path(sys.argv[1]).resolve()
mode = sys.argv[2]
assert mode in ('prepare', 'measure')
H = Path(__file__).resolve().parent
load = lambda p: json.loads(p.read_text())
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
plan = load(S / 'plan.json')
build = load(S / 'build-summary-v1.json')
assert build['complete'] and sha(S / 'plan.json') == build['plan_sha256']
assert sorted(os.sched_getaffinity(0)) == list(range(6, 16)) + list(range(22, 32))
for path, digest in build['sources'].items():
    assert sha(Path(path)) == digest, path
outroot = S / 'runs'
if mode == 'prepare':
    outroot.mkdir(exist_ok=False)
else:
    assert load(S / 'inputs-accepted.json')['complete']
    assert load(S / 'codegen-review.json')['measurement_authorized']
summary = dict(complete=False, started_ns=time.time_ns(), mode=mode, processes=[],
               plan_sha256=sha(S / 'plan.json'),
               tools={str(H / p): sha(H / p) for p in ('run.py', 'validate.py')})
for path, digest in summary['tools'].items():
    dest = S / 'tools' / Path(path).name
    if not dest.exists():
        dest.parent.mkdir(exist_ok=True)
        shutil.copyfile(path, dest)
    assert sha(dest) == digest

def execute(name, arm, case, order):
    binary = build['arms'][arm]['timing']
    assert sha(Path(binary['path'])) == binary['sha256']
    free = {p: shutil.disk_usage(p).free for p in ('/home/dongxu/kv9', '/mnt/data')}
    assert min(free.values()) >= plan['minimum_free_bytes']
    argv = ['taskset', '-c', str(plan['cpu']), binary['path'], str(S),
            str(outroot / (name + '.json')), mode, str(case), str(order)]
    row = dict(name=name, arm=arm, mode=mode, case=case, order=order, argv=argv,
               complete=False, binary_sha256=binary['sha256'], started_ns=time.time_ns(),
               free_bytes=free, shared_host=True, loadavg=Path('/proc/loadavg').read_text().strip())
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
        (S / (mode + '-progress.json')).write_text(json.dumps(summary, indent=2) + '\n')

try:
    if mode == 'prepare':
        for order, arm in enumerate(('baseline', 'candidate')):
            execute('prepare-' + arm, arm, 0, order)
        with (S / 'input-validation.stdout').open('x') as out, (S / 'input-validation.stderr').open('x') as err:
            subprocess.run(['python3', '-B', str(H / 'validate.py'), str(S), 'prepare'], check=True, stdout=out, stderr=err, timeout=120)
        assert load(S / 'inputs-accepted.json')['complete']
    else:
        for case in range(len(plan['cases'])):
            for order, arm in enumerate(plan['orders']):
                execute(f'case-{case:02d}-{order}-{arm}', arm, case, order)
            print(json.dumps({'case': case, 'complete': True, 'processes': len(summary['processes'])}), flush=True)
    assert len(summary['processes']) == (2 if mode == 'prepare' else 88)
    assert all(p['complete'] for p in summary['processes'])
    for path, digest in {**build['sources'], **summary['tools']}.items():
        assert sha(Path(path)) == digest, path
    summary['complete'] = True
except BaseException as error:
    summary['failure'] = repr(error)
    raise
finally:
    summary['ended_ns'] = time.time_ns()
    (S / (mode + '-summary.json')).write_text(json.dumps(summary, indent=2) + '\n')
