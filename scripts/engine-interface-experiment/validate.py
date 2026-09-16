#!/usr/bin/env python3
"""Independently reconstruct engine inputs and recompute every timing statistic."""
import hashlib
import importlib.util
import json
import math
from pathlib import Path
import sys

S = Path(sys.argv[1]).resolve()
mode = sys.argv[2]
load = lambda p: json.loads(p.read_text())
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
plan = load(S / 'plan.json')
build = load(S / 'build-summary-v1.json')
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
query_hashes = {}
for name in names:
    for operation in ('get_hit', 'get_miss'):
        keys = reference.probes(models['original24', name], operation)
        query_hashes[name, operation] = reference.digest([(k, b'') for k in keys])

def payload(name):
    path = S / 'runs' / (name + '.json')
    terminal = load(S / 'runs' / (name + '-terminal.json'))
    assert terminal['complete'] and terminal['exit_code'] == 0
    assert terminal['result_sha256'] == sha(path)
    assert terminal['result_bytes'] == path.stat().st_size <= plan['process_output_max_bytes']
    binary = build['arms'][terminal['arm']]['timing']
    assert terminal['binary_sha256'] == sha(Path(binary['path'])) == binary['sha256']
    assert terminal['ended_ns'] >= terminal['started_ns'] and terminal['pid'] > 0
    x = load(path)
    assert x['complete'] and x['workloads'] == metadata
    assert x['input_plan_sha256'] == sha(S / 'plan.json')
    assert x['corpus_sha256'] == sha(S / 'groups.bin') == plan['corpus_sha256']
    return x

prepared = {}
for arm in ('baseline', 'candidate'):
    x = payload('prepare-' + arm)
    assert x['mode'] == 'prepare'
    assert x['checks'] == [dict(workload=name, pinned=pinned, live_prefixes=106,
                               old_views=106 if pinned else 0, position_checks=106,
                               other_cfs_checked=True)
                           for name in names for pinned in (False, True)]
    assert len(x['read_inputs']) == 2
    for actual, name in zip(x['read_inputs'], names):
        model = models['original24', name]
        assert actual['dataset'] == 'original24' and actual['workload'] == name
        assert actual['keys_hex'] == [k.hex() for k, v in model]
        assert [q['operation'] for q in actual['queries']] == ['get_hit', 'get_miss']
        for q in actual['queries']:
            keys = reference.probes(model, q['operation'])
            assert q['queries_hex'] == [k.hex() for k in keys]
            assert q['query_sha256'] == query_hashes[name, q['operation']]
    prepared[arm] = sha(S / 'runs' / ('prepare-' + arm + '.json'))
assert len(set(prepared.values())) == 1
accepted = dict(complete=True, independent_reconstruction=True, models=metadata,
                prepared_sha256=prepared, live_prefixes=848, old_views=424,
                query_identities=[dict(workload=k[0], operation=k[1], sha256=v)
                                  for k, v in query_hashes.items()])
if mode == 'prepare':
    with (S / 'inputs-accepted.json').open('x') as out:
        json.dump(accepted, out, indent=2)
    print(json.dumps({'complete': True, 'live_prefixes': 848, 'old_views': 424, 'probe_sets': 4}))
    sys.exit(0)
assert mode == 'final' and load(S / 'inputs-accepted.json') == accepted
assert load(S / 'codegen-review.json')['measurement_authorized']
runs = load(S / 'measure-summary.json')
assert runs['complete'] and len(runs['processes']) == 88
for path, digest in runs['tools'].items():
    assert sha(Path(path)) == digest

def metrics(samples):
    assert samples and all(type(v) is int and v > 0 for v in samples)
    ordered = sorted(samples)
    n = len(samples)
    return dict(windows=n, sum_ns=sum(samples), mean_ns=sum(samples) / n,
                **{f'p{q}_ns': ordered[math.ceil(n * q / 100) - 1] for q in (50, 95, 99)})

comparisons = []
row_count = 0
for case, case_spec in enumerate(plan['cases']):
    panels = {}
    operation = case_spec['operation']
    name = case_spec['workload']
    for order, arm in enumerate(plan['orders']):
        x = payload(f'case-{case:02d}-{order}-{arm}')
        assert (x['mode'], x['case'], x['order'], x['backend'], x['spec']) == ('measure', case, order, arm, case_spec)
        phases = ['first_probe', 'warm'] if operation == 'read' else ['apply_group' if operation == 'write_applied' else 'steady_snapshot']
        assert [r['phase'] for r in x['rows']] == phases
        for row in x['rows']:
            raw = row['metrics']['samples_ns']
            actual = {k: v for k, v in row['metrics'].items() if k != 'samples_ns'}
            assert actual == metrics(raw)
            assert row['other_cfs_checked']
            if operation == 'read':
                passes = 1 if row['phase'] == 'first_probe' else 12
                per_call = case_spec['timer'] == 'per_call'
                assert row['unit'] == ('ns_per_read' if per_call else 'ns_per_512_read_pass')
                assert row['epochs'] == row['stable_old_view_checks'] == 12
                assert row['passes_per_epoch'] == passes and row['passes'] == passes * 12
                assert row['queries_per_pass'] == 512 and row['warmup_passes'] == 8
                assert row['queries_per_window'] == (1 if per_call else 512)
                assert len(raw) == passes * 12 * (512 if per_call else 1)
                assert row['query_sha256'] == query_hashes[name, case_spec['queries']]
                assert row['all_outputs_checked']
                assert row['initial_applied_index'] == 106 and row['later_applied_index'] == 107
            else:
                assert row['applied_index'] == row['data_revision'] == 106
                assert row['final_state_sha256'] == reference.digest(models['original24', name])
                assert row['passes'] == 12
                if operation == 'write_applied':
                    assert row['unit'] == 'ns_per_group' and len(raw) == 1272
                    assert row['mutations'] == 12 * 100096 and row['final_keys'] == len(models['original24', name])
                else:
                    assert row['unit'] == 'ns_per_snapshot' and len(raw) == 6144
                    assert row['queries_per_pass'] == 512 and row['warmup_passes'] == 8
            panels.setdefault((row['phase'], row['unit']), {})[order] = raw
            row_count += 1
    for (phase, unit), orders in panels.items():
        baseline = metrics(orders[0] + orders[3])
        candidate = metrics(orders[1] + orders[2])
        delta = lambda b, c: {k: (c[k] / b[k] - 1) * 100 for k in ('mean_ns', 'p99_ns')}
        row = dict(case=case, spec=case_spec, phase=phase, unit=unit, baseline=baseline,
                   candidate=candidate, percent_change=delta(baseline, candidate),
                   by_order=[delta(metrics(orders[b]), metrics(orders[c])) for b, c in ((0, 1), (3, 2))])
        if unit == 'ns_per_512_read_pass':
            row['pass_derived_mean_ns_per_read'] = {arm: value['mean_ns'] / 512 for arm, value in (('baseline', baseline), ('candidate', candidate))}
            row['p99_scope'] = '512-query pass; not individual request latency'
        comparisons.append(row)
assert row_count == plan['expected_rows'] == 152
assert len(comparisons) == plan['expected_comparison_cells'] == 38
result = dict(complete=True, diagnostic_only=True, production_promoted=False, rows=row_count,
              processes=90, comparisons=comparisons, input_acceptance=accepted,
              original_component_gate='failed; unchanged by this diagnostic',
              limits='Shared host, two orders, engine component only; no WAL, Raft, RPC, database QPS or new Redis comparison.')
with (S / 'analysis.json').open('x') as out:
    json.dump(result, out, indent=2)
print(json.dumps({'complete': True, 'rows': row_count, 'comparisons': len(comparisons), 'production_promoted': False}))
