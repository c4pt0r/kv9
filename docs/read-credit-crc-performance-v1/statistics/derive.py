#!/usr/bin/env python3
"""Derive the complete read-credit-on-CRC workload matrix only after accepted auditing."""
import hashlib
import json
from pathlib import Path

from core import aggregate, add_maps, compare, merge_histograms, phase_accounting, qtext

assert __debug__
HERE = Path(__file__).resolve().parent
AUDIT_DIR = Path('/tmp/kv9-read-credit-crc-performance-audit-repair-first/results-first')
GATE = Path('/tmp/kv9-read-credit-crc-performance-root-preparation/audit-terminal-second.json')


def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def read(path):
    return json.loads(path.read_text())


def save(name, value):
    with (HERE / name).open('x') as stream:
        json.dump(value, stream, indent=2, sort_keys=True)
        stream.write('\n')


gate = read(GATE)
assert gate['exit_code'] == 0 and gate['terminal'] is True
audit = read(AUDIT_DIR / 'audit.json')
inventory = read(AUDIT_DIR / 'input-inventory.json')
assert audit['complete'] and audit['matched_diagnostic_accepted']
assert audit['parent_confirmed_session'] == gate['timing_session']
assert audit['protocol_id'] == 'kv9-read-credit-crc-broad-workloads-c1-c64-v1' and len(audit['cases']) == 72
reports = {}
bound = {}
for case in audit['cases']:
    path = Path(case['directory']) / 'run/report.json'
    assert sha(path) == case['report_sha256'] == inventory[str(path)]['sha256']
    report = read(path)
    reports[case['ordinal']] = report
    bound[str(path)] = dict(bytes=path.stat().st_size, sha256=sha(path))
    coverage_path = Path(case['directory']) / 'resource-coverage.json'
    assert sha(coverage_path) == inventory[str(coverage_path)]['sha256']
    report['_accepted_cpu'] = read(coverage_path)['cpu']
    bound[str(coverage_path)] = dict(bytes=coverage_path.stat().st_size, sha256=sha(coverage_path))
    metrics = report['metrics']['measurement']['statistics']
    for index, operation in enumerate(metrics):
        assert merge_histograms(p['whole_call']['raw'] for p in operation['populations']) == case['operations'][index]['whole_call_latency']
    assert merge_histograms(p['whole_call']['raw'] for operation in metrics for p in operation['populations']) == case['whole_call_latency']

cases = []
for case in audit['cases']:
    report = reports[case['ordinal']]
    row = {key: case[key] for key in ['ordinal', 'repeat', 'concurrency', 'arm', 'workload', 'read_api', 'write_api', 'batch_size', 'read_percent', 'directory', 'report_sha256', 'all_success_single_attempt', 'dataset', 'storage']}
    row['role'] = case['arm'].split('-', 1)[0]
    row.update(aggregate([case], [report]))
    row['phases'] = {phase: phase_accounting(metrics, row['role'] != 'redis') for phase, metrics in report['metrics'].items()}
    cases.append(row)

pooled = []
pairs = []
pooled_pairs = []
workloads = ['point-r000', 'point-r050', 'point-r100', 'batch64-r000', 'batch64-r050', 'batch64-r100']
for concurrency in [1, 64]:
    for workload in workloads:
        for role in ['old', 'new', 'redis']:
            selected = [c for c in cases if c['concurrency'] == concurrency and c['workload'] == workload and c['role'] == role]
            assert len(selected) == 2 and {c['repeat'] for c in selected} == {0, 1}
            row = {key: selected[0][key] for key in ['concurrency', 'workload', 'role', 'read_api', 'write_api', 'batch_size', 'read_percent']}
            row.update(aggregate([audit['cases'][c['ordinal']] for c in selected], [reports[c['ordinal']] for c in selected]))
            row['source_ordinals'] = [c['ordinal'] for c in selected]
            pooled.append(row)
        for repeat in [0, 1]:
            selected = {c['role']: c for c in cases if c['concurrency'] == concurrency and c['workload'] == workload and c['repeat'] == repeat}
            assert set(selected) == {'old', 'new', 'redis'}
            pairs.append(dict(concurrency=concurrency, workload=workload, repeat=repeat, comparison=compare(selected['old'], selected['new'])))
        selected = {c['role']: c for c in pooled if c['concurrency'] == concurrency and c['workload'] == workload}
        pooled_pairs.append(dict(concurrency=concurrency, workload=workload, comparison=compare(selected['old'], selected['new']),
            redis_throughput_ratio=selected['redis']['successful_calls_per_second'] / selected['new']['successful_calls_per_second'],
            candidate_to_redis_mean_ratio=selected['new']['whole_call_latency']['mean_ns'] / selected['redis']['whole_call_latency']['mean_ns']))

totals = {key: sum(c[key] for c in cases) for key in ['calls', 'input_items', 'issued', 'attempts', 'dropped_slots']}
for key in ['outcomes', 'reasons', 'attempt_reasons']:
    totals[key] = add_maps(c[key] for c in cases)
phase_totals = {}
for phase in ['initialization', 'warmup', 'measurement', 'verification']:
    selected = [c['phases'][phase] for c in cases]
    phase_totals[phase] = {key: sum(c[key] for c in selected) for key in ['calls', 'input_items', 'attempts', 'extra_attempts']}
    for key in ['outcomes', 'reasons', 'attempt_reasons']:
        phase_totals[phase][key] = add_maps(c[key] for c in selected)
checks = {key: audit[key] for key in ['successful_arms', 'owned_lifetimes_exited', 'role_source_file_checks', 'resource_samples', 'qualifying_drains', 'voter_writer_listener_bindings', 'retained_files', 'retained_bytes', 'outer_restoration']}
result = dict(complete=True, accepted_audit_sha256=sha(AUDIT_DIR / 'audit.json'), accepted_inventory_sha256=sha(AUDIT_DIR / 'input-inventory.json'),
    protocol_id=audit['protocol_id'], scope=audit['scope'], cases=cases, pooled=pooled, pairs=pairs, pooled_pairs=pooled_pairs,
    totals=totals, phase_totals=phase_totals, checks=checks, performance_promotion=False,
    method='Rates divide summed successful calls or keys by complete cohort elapsed time. Means use summed whole-call nanoseconds/count. Quantiles merge validated raw buckets; no percentile averaging or latency division by batch size. All outcomes and original repeats remain retained.',
    limitation='Two 10-second forward/reverse c1/c64 point/batch read/mixed/write repeats on a shared host; three-voter KV9 quorum/sync with tmpfs WAL versus standalone Redis with persistence and pipelining disabled. No equal-durability, sustained-capacity, significance or new Chaos claim.')
save('statistics-first.json', result)
save('statistics-inputs-first.json', dict(files=bound, audit_sha256=result['accepted_audit_sha256'], gate_sha256=sha(GATE), core_sha256=sha(HERE / 'core.py'), reader_sha256=sha(Path(__file__))))
lines = ['# Read-credit-on-CRC full workload matrix: first accepted recording', '', result['limitation'], '',
    '| Concurrency | Workload | Role | Calls/s | Keys/s | Mean us | p95 us | p99 us | Client cores | Server cores |',
    '| ---: | --- | --- | ---: | ---: | ---: | --- | --- | ---: | ---: |']
for row in pooled:
    hist = row['whole_call_latency']
    lines.append(f"| {row['concurrency']} | {row['workload']} | {row['role']} | {row['successful_calls_per_second']:.3f} | {row['successful_input_items_per_second']:.3f} | {hist['mean_ns']/1000:.3f} | {qtext(hist,95)} | {qtext(hist,99)} | {row['client_cpu_cores']:.3f} | {row['server_cpu_cores']:.3f} |")
lines += ['', 'Quantiles are bucket intervals; batch latency covers the whole call.', '', '| Concurrency | Workload | Repeat | QPS change | Mean change | Old p99 ns | New p99 ns |', '| ---: | --- | ---: | ---: | ---: | --- | --- |']
for row in pairs:
    c = row['comparison']
    lines.append(f"| {row['concurrency']} | {row['workload']} | {row['repeat']} | {c['successful_qps_change_percent']:+.3f}% | {c['mean_change_percent']:+.3f}% | {c['old_p99']} | {c['new_p99']} |")
lines += ['', f"All measured calls: {totals['calls']:,}; input keys: {totals['input_items']:,}; outcomes: {json.dumps(totals['outcomes'],sort_keys=True)}.", '', 'All phases, original per-operation populations and CPU sample scopes remain in statistics-first.json. Promotion requires further workload and recovery acceptance.']
with (HERE / 'READOUT.md').open('x') as stream:
    stream.write('\n'.join(lines) + '\n')
print(json.dumps(dict(complete=True, cases=len(cases), pooled_rows=len(pooled), totals=totals)))
