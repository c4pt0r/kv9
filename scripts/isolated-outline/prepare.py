#!/usr/bin/env python3
"""Run only semantic preparation and reconstruct every final key and probe."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import time

S = Path(sys.argv[1]).resolve()
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
load = lambda p: json.loads(p.read_text())
build = load(S / 'build-summary.json')
assert build['complete'] and load(S / 'proof-isolated/result.json')['accepted']
assert load(S / 'tests-accepted.json')['complete']
assert load(S / 'codegen-review.json')['mutation_callback_removed']
assert sorted(os.sched_getaffinity(0)) == list(range(6, 16)) + list(range(22, 32))
for path, digest in build['sources'].items():
    assert sha(Path(path)) == digest
spec = importlib.util.spec_from_file_location('reference', S / 'reference-inputs.py')
reference = importlib.util.module_from_spec(spec)
spec.loader.exec_module(reference)
models, metadata = reference.reconstruct()
metadata = [m for m in metadata if m['dataset'] == 'original24' and m['workload'] != 'initial_fill']
out = S / 'preparation'
out.mkdir(exist_ok=False)
summary = dict(complete=False, started_ns=time.time_ns(), processes=[], performance_executed=False)
try:
    payloads = []
    for arm in ('baseline', 'candidate'):
        binary = build['arms'][arm]['binary']
        assert sha(Path(binary['path'])) == binary['sha256']
        output = out / (arm + '.json')
        argv = ['taskset', '-c', '4', binary['path'], str(S), str(output), 'prepare', '0', '0']
        row = dict(arm=arm, argv=argv, complete=False, started_ns=time.time_ns(), binary_sha256=binary['sha256'])
        process = None
        try:
            with (out / (arm + '.stdout')).open('x') as stdout, (out / (arm + '.stderr')).open('x') as stderr:
                process = subprocess.Popen(argv, stdout=stdout, stderr=stderr)
                row['pid'] = process.pid
                row['start_ticks'] = int(Path(f'/proc/{process.pid}/stat').read_text().rsplit(')', 1)[1].split()[19])
                (out / (arm + '-launch.json')).write_text(json.dumps(row, indent=2) + '\n')
                row['exit_code'] = process.wait(timeout=180)
            assert row['exit_code'] == 0
            x = load(output)
            assert x['complete'] and x['mode'] == 'prepare' and x['workloads'] == metadata
            assert x['input_plan_sha256'] == sha(S / 'plan.json') and x['corpus_sha256'] == sha(S / 'groups.bin')
            assert x['checks'] == [dict(workload=name, pinned=pinned, live_prefixes=106, old_views=106 if pinned else 0, position_checks=106, other_cfs_checked=True)
                                   for name in ('overwrite', 'unique_insert') for pinned in (False, True)]
            assert len(x['read_inputs']) == 2
            for entry, name in zip(x['read_inputs'], ('overwrite', 'unique_insert')):
                rows = models['original24', name]
                assert entry['dataset'] == 'original24' and entry['workload'] == name
                assert entry['keys_hex'] == [key.hex() for key, value in rows]
                assert [q['operation'] for q in entry['queries']] == ['get_hit', 'get_miss']
                for query in entry['queries']:
                    keys = reference.probes(rows, query['operation'])
                    assert query['queries_hex'] == [key.hex() for key in keys]
                    assert query['query_sha256'] == reference.digest([(key, b'') for key in keys])
            payloads.append(x)
            row.update(complete=True, result_sha256=sha(output), bytes=output.stat().st_size)
        finally:
            if process is not None and process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=10)
            row['ended_ns'] = time.time_ns()
            (out / (arm + '-terminal.json')).write_text(json.dumps(row, indent=2) + '\n')
            summary['processes'].append(row)
    assert payloads[0] == payloads[1]
    assert all(sha(Path(p)) == h for p, h in build['sources'].items())
    summary.update(complete=True, live_prefixes=848, old_views=424, probe_sets=4,
                   independent_full_keys_probes_and_digest_reconstruction=True)
except BaseException as error:
    summary['failure'] = repr(error)
    raise
finally:
    summary['ended_ns'] = time.time_ns()
    (S / 'preparation-summary.json').write_text(json.dumps(summary, indent=2) + '\n')
print(json.dumps({'complete': True, 'live_prefixes': 848, 'old_views': 424, 'performance_executed': False}))
