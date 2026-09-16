#!/usr/bin/env python3
"""Independently check corpus, allocator windows, raw timing and declared gates."""
import hashlib
import importlib.util
import json
import math
from pathlib import Path
import sys

S = Path(sys.argv[1]).resolve()
mode = sys.argv[2]
assert mode in ('prepare', 'counting', 'final')
load = lambda p: json.loads(p.read_text())
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
plan = load(S / 'plan.json')
build = load(S / 'build-summary.json')
assert build['complete'] and build['plan_sha256'] == sha(S / 'plan.json')
for path, digest in build['sources'].items():
    assert sha(Path(path)) == digest, path
spec = importlib.util.spec_from_file_location('reference', S / 'reference-inputs.py')
reference = importlib.util.module_from_spec(spec)
spec.loader.exec_module(reference)
models, metadata = reference.reconstruct()
metadata = [m for m in metadata if m['dataset'] == 'original24' and m['workload'] != 'initial_fill']
names = ['overwrite', 'unique_insert']
assert [m['workload'] for m in metadata] == names
probes = {(name, operation): reference.probes(models['original24', name], operation)
          for name in names for operation in ('get_hit', 'get_miss')}
query_hashes = {key: reference.digest([(k, b'') for k in keys]) for key, keys in probes.items()}
terminals = []

def payload(name, profile):
    path = S / 'runs' / (name + '.json')
    terminal = load(S / 'runs' / (name + '-terminal.json'))
    assert terminal['complete'] and terminal['exit_code'] == 0 and terminal['profile'] == profile
    assert terminal['result_sha256'] == sha(path)
    assert terminal['result_bytes'] == path.stat().st_size <= plan['process_output_max_bytes']
    binary = build['arms'][terminal['arm']][profile]
    assert terminal['binary_sha256'] == sha(Path(binary['path'])) == binary['sha256']
    assert terminal['ended_ns'] >= terminal['started_ns'] and terminal['pid'] > 0
    stat = Path(f'/proc/{terminal["pid"]}/stat')
    if stat.exists():
        assert int(stat.read_text().rsplit(')', 1)[1].split()[19]) != terminal['start_ticks']
    terminals.append(terminal)
    x = load(path)
    assert x['complete'] and x['workloads'] == metadata
    assert x.get('counting_build', False) == (profile == 'counting')
    assert x['input_plan_sha256'] == sha(S / 'plan.json')
    assert x['corpus_sha256'] == sha(S / 'groups.bin') == plan['corpus_sha256']
    return x

prepared = {}
canonical = []
for profile in ('timing', 'counting'):
    for arm in ('baseline', 'candidate'):
        name = f'prepare-{profile}-{arm}'
        x = payload(name, profile)
        assert x['mode'] == 'prepare'
        assert x['checks'] == [dict(workload=n, pinned=pinned, live_prefixes=106,
                                   old_views=106 if pinned else 0, position_checks=106,
                                   other_cfs_checked=True)
                               for n in names for pinned in (False, True)]
        assert len(x['read_inputs']) == 2
        for entry, n in zip(x['read_inputs'], names):
            assert entry['dataset'] == 'original24' and entry['workload'] == n
            assert entry['keys_hex'] == [key.hex() for key, value in models['original24', n]]
            assert [q['operation'] for q in entry['queries']] == ['get_hit', 'get_miss']
            for q in entry['queries']:
                assert q['queries_hex'] == [k.hex() for k in probes[n, q['operation']]]
                assert q['query_sha256'] == query_hashes[n, q['operation']]
        prepared[name] = sha(S / 'runs' / (name + '.json'))
        x.pop('counting_build', None)
        canonical.append(x)
assert all(x == canonical[0] for x in canonical)
accepted = dict(complete=True, models=metadata, prepared_sha256=prepared,
                live_prefixes=1696, old_views=848, independent_reconstruction=True,
                query_identities=[dict(workload=k[0], operation=k[1], sha256=v) for k, v in query_hashes.items()])

def check_run_summary(stage, expected):
    summary = load(S / (stage + '-summary.json'))
    assert summary['complete'] and len(summary['processes']) == expected
    assert summary['plan_sha256'] == sha(S / 'plan.json')
    for path, digest in summary['tools'].items():
        assert sha(Path(path)) == digest
    for terminal in summary['processes']:
        assert terminal == load(S / 'runs' / (terminal['name'] + '-terminal.json'))
    return summary

check_run_summary('prepare', 4)
if mode == 'prepare':
    with (S / 'inputs-accepted.json').open('x') as out:
        json.dump(accepted, out, indent=2)
    print(json.dumps({'complete': True, 'preparations': 4, 'live_prefixes': 1696, 'old_views': 848}))
    sys.exit(0)
assert load(S / 'inputs-accepted.json') == accepted

def metric_summary(samples):
    assert samples and all(type(v) is int and v > 0 for v in samples)
    ordered = sorted(samples)
    n = len(samples)
    return dict(windows=n, sum_ns=sum(samples), mean_ns=sum(samples) / n,
                **{f'p{q}_ns': ordered[math.ceil(n * q / 100) - 1] for q in (50, 95, 99)})

def check_row(row, spec, counting):
    operation, name = spec['operation'], spec['workload']
    raw = row['metrics']['window_records' if counting else 'samples_ns']
    assert row['metrics']['windows'] == len(raw) and row['other_cfs_checked']
    unit = row['operation_window'] if counting else row['unit']
    if counting:
        assert row['unit'] == 'allocator_counts_per_window'
        m = row['metrics']
        assert set(m) == {'windows', 'allocation_counts', 'maximum_window_extra_live_bytes', 'sum_window_live_delta_bytes', 'window_records', 'elapsed_time_recorded'}
        assert m['elapsed_time_recorded'] is False
        for counts, delta, peak in raw:
            assert len(counts) == 6 and all(type(n) is int and n >= 0 for n in counts)
            assert type(delta) is int and type(peak) is int and peak >= 0
        assert m['allocation_counts'] == [sum(r[0][i] for r in raw) for i in range(6)]
        assert m['maximum_window_extra_live_bytes'] == max(r[2] for r in raw)
        assert m['sum_window_live_delta_bytes'] == sum(r[1] for r in raw)
    else:
        assert {k: v for k, v in row['metrics'].items() if k != 'samples_ns'} == metric_summary(raw)
    if operation == 'read':
        passes = 1 if row['phase'] == 'first_probe' else 12
        per_call = spec['timer'] == 'per_call'
        assert unit == ('ns_per_read' if per_call else 'ns_per_512_read_pass')
        assert row['epochs'] == row['stable_old_view_checks'] == 12
        assert row['passes_per_epoch'] == passes and row['passes'] == passes * 12
        assert row['queries_per_pass'] == 512 and row['warmup_passes'] == 8
        assert row['queries_per_window'] == (1 if per_call else 512)
        assert len(raw) == passes * 12 * (512 if per_call else 1)
        assert row['query_sha256'] == query_hashes[name, spec['queries']]
        assert row['all_outputs_checked'] and row['initial_applied_index'] == 106 and row['later_applied_index'] == 107
        if counting:
            model = dict(models['original24', name])
            lengths = [len(model[k]) for k in probes[name, spec['queries']]] if spec['queries'] == 'get_hit' else []
            for i, record in enumerate(raw):
                if spec['api'] == 'owned' and spec['queries'] == 'get_hit':
                    n, size = (1, lengths[i % 512]) if per_call else (512, sum(lengths))
                    expected = [[n, size, 0, 0, 0, 0], size, size]
                else:
                    expected = [[0, 0, 0, 0, 0, 0], 0, 0]
                assert record == expected, (spec, row['phase'], i, record, expected)
    else:
        assert row['applied_index'] == row['data_revision'] == 106 and row['passes'] == 12
        assert row['final_state_sha256'] == reference.digest(models['original24', name])
        if operation == 'write_applied':
            assert unit == 'ns_per_group' and len(raw) == 1272
            assert row['mutations'] == 12 * 100096 and row['final_keys'] == len(models['original24', name])
        else:
            assert unit == 'ns_per_snapshot' and len(raw) == 6144
            assert row['queries_per_pass'] == 512 and row['warmup_passes'] == 8
            if counting:
                size = row['snapshot_bytes']
                assert type(size) is int and size > 0
                assert all(record == [[1, size, 0, 0, 0, 0], size, size] for record in raw)
    return raw, unit

allocation_rows = 0
allocation_cells = []
check_run_summary('counting', 44)
for case, case_spec in enumerate(plan['cases']):
    rows = []
    for order, arm in enumerate(('baseline', 'candidate')):
        x = payload(f'case-{case:02d}-counting-{order}-{arm}', 'counting')
        assert (x['mode'], x['case'], x['order'], x['backend'], x['spec']) == ('measure', case, order, arm, case_spec)
        phases = ['first_probe', 'warm'] if case_spec['operation'] == 'read' else ['apply_group' if case_spec['operation'] == 'write_applied' else 'steady_snapshot']
        assert [r['phase'] for r in x['rows']] == phases
        for row in x['rows']:
            check_row(row, case_spec, True)
            allocation_rows += 1
        rows.append(x['rows'])
    assert rows[0] == rows[1], ('allocation mismatch', case)
    for row in rows[0]:
        allocation_cells.append(dict(case=case, spec=case_spec, phase=row['phase'],
                                     operation_window=row['operation_window'], identical_per_window=True,
                                     **{k: v for k, v in row['metrics'].items() if k != 'window_records'}))
assert allocation_rows == plan['expected_counting_rows'] == 76 and len(allocation_cells) == 38
allocations = dict(complete=True, accepted=True, rows=allocation_rows, cells=allocation_cells,
                   exact_per_window_equality=True, independent_read_and_snapshot_expectations=True,
                   elapsed_time_recorded=False, input_acceptance=accepted)
if mode == 'counting':
    with (S / 'allocation-analysis.json').open('x') as out:
        json.dump(allocations, out, indent=2)
    print(json.dumps({'accepted': True, 'rows': allocation_rows, 'cells': 38, 'elapsed_time_recorded': False}))
    sys.exit(0)
assert load(S / 'allocation-analysis.json') == allocations
check_run_summary('timing', 88)
comparisons = []
timing_rows = 0
for case, case_spec in enumerate(plan['cases']):
    panels = {}
    for order, arm in enumerate(plan['orders']):
        x = payload(f'case-{case:02d}-timing-{order}-{arm}', 'timing')
        assert (x['mode'], x['case'], x['order'], x['backend'], x['spec']) == ('measure', case, order, arm, case_spec)
        phases = ['first_probe', 'warm'] if case_spec['operation'] == 'read' else ['apply_group' if case_spec['operation'] == 'write_applied' else 'steady_snapshot']
        assert [r['phase'] for r in x['rows']] == phases
        for row in x['rows']:
            raw, unit = check_row(row, case_spec, False)
            panels.setdefault((row['phase'], unit), {})[order] = raw
            timing_rows += 1
    for (phase, unit), orders in panels.items():
        baseline = metric_summary(orders[0] + orders[3])
        candidate = metric_summary(orders[1] + orders[2])
        delta = lambda b, c: {k: (c[k] / b[k] - 1) * 100 for k in ('mean_ns', 'p99_ns')}
        row = dict(case=case, spec=case_spec, phase=phase, unit=unit,
                   baseline=baseline, candidate=candidate, percent_change=delta(baseline, candidate),
                   by_order=[delta(metric_summary(orders[b]), metric_summary(orders[c])) for b, c in ((0, 1), (3, 2))])
        if unit == 'ns_per_512_read_pass':
            row['pass_derived_mean_ns_per_read'] = {arm: value['mean_ns'] / 512 for arm, value in (('baseline', baseline), ('candidate', candidate))}
            row['p99_scope'] = '512-query pass; not individual request latency'
        comparisons.append(row)
assert timing_rows == plan['expected_timing_rows'] == 152 and len(comparisons) == 38
assert len(terminals) == plan['expected_total_processes'] == 136
write_cells = [r for r in comparisons if r['spec']['operation'] == 'write_applied']
read_cells = [r for r in comparisons if r['spec']['operation'] != 'write_applied']
write_failures = [r['case'] for r in write_cells if r['percent_change']['mean_ns'] > -10 or not all(o['mean_ns'] < 0 for o in r['by_order']) or r['percent_change']['p99_ns'] > 2]
read_failures = [dict(case=r['case'], phase=r['phase'], failed_metrics=[k for k in ('mean_ns', 'p99_ns') if r['percent_change'][k] > 2])
                 for r in read_cells if any(r['percent_change'][k] > 2 for k in ('mean_ns', 'p99_ns'))]
result = dict(complete=True, processes=136, timing_rows=timing_rows, counting_rows=allocation_rows,
              comparisons=comparisons, allocations_accepted=True,
              component_gate_passed=not write_failures and not read_failures,
              write_gate_failed_cases=write_failures, read_snapshot_gate_failures=read_failures,
              production_promoted=False, combined_candidate_gate='failed; unchanged',
              limits='Shared host, two timing orders, component API only; no WAL, Raft, RPC, database QPS or new Redis comparison.')
with (S / 'analysis.json').open('x') as out:
    json.dump(result, out, indent=2)
print(json.dumps({k: v for k, v in result.items() if k not in ('comparisons', 'read_snapshot_gate_failures')}))
