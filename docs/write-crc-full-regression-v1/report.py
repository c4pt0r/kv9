#!/usr/bin/env python3
"""Operation-aware statistics for one complete new native48 campaign; no runtime or WAL audit."""
import argparse
import hashlib
import json
import os
from pathlib import Path
from core import aggregate, add_maps, merge_histograms, phase_accounting, qtext
from cpu import cpu_from_samples, percent, bucket_direction
from gates import (HERE, PREP, RUN, SMOKE, AUDIT, PROTOCOL, ROLES, WORKLOADS,
                   accepted_cases, matrix_gate, terminal_gate, report_binding)

CAP = 16 * 1024**2
CPU_SCOPE = ('Source-bound original per-process first/last observations inside measurement; '
    'CPU rates weighted by each process observed sample seconds. Voter rates are summed and client '
    'separate. Sampling endpoints differ; this is not exclusive request service time or per-operation CPU. '
    'Voter numbers do not assert leader identity. HWM includes setup; no pooled HWM is invented.')


def measurement_gate(case, report, descriptor):
    metrics = report['metrics']['measurement']
    assert metrics['operations'] == (['get', 'put'] if descriptor['shared']['batch_size'] == 1
                                     else ['batch_get', 'batch_put'])
    assert metrics['outcomes'][0] == 'success' and len(metrics['statistics']) == 2
    accounted = phase_accounting(metrics, True)
    for k in ('calls', 'input_items', 'attempts', 'outcomes', 'reasons', 'attempt_reasons'):
        assert accounted[k] == case[k], ('measurement accounting differs', k)
    for index, (op, original) in enumerate(zip(accounted['operations'], case['operations'], strict=True)):
        for k in ('operation', 'calls', 'input_items', 'attempts', 'outcomes', 'reasons',
                  'attempt_reasons', 'terminal_rpc_codes'):
            assert op[k] == original[k], ('operation accounting differs', index, k)
        populations = metrics['statistics'][index]['populations']
        assert len(populations) == len(metrics['outcomes'])
        assert merge_histograms(p['whole_call']['raw'] for p in populations) == original['whole_call_latency']
    rp = descriptor['shared']['read_percent']
    if rp == 0:
        assert accounted['operations'][0]['calls'] == 0, 'unused read population'
    if rp == 100:
        assert accounted['operations'][1]['calls'] == 0, 'unused write population'
    assert merge_histograms(p['whole_call']['raw'] for op in metrics['statistics']
                            for p in op['populations']) == case['whole_call_latency']
    assert merge_histograms(op['populations'][0]['whole_call']['raw'] for op in metrics['statistics']) == case['successful_whole_call_latency']
    assert report['cohort_elapsed_ns'] == case['cohort_elapsed_ns'] > 0
    assert report['measured_completed'] == case['calls']
    assert report['measured_issued'] == case['issued']
    assert report['dropped_slots'] == case['dropped_slots']
    assert len(case['operations']) == 2


def aggregate_group(cases, reports):
    assert cases and len(cases) == len(reports)
    result = aggregate(cases, reports)  # Exact qualified arithmetic, unchanged core.py.
    outcomes = reports[0]['metrics']['measurement']['outcomes']
    assert all(r['metrics']['measurement']['outcomes'] == outcomes for r in reports)
    for index, op in enumerate(result['operations']):
        originals = [r['metrics']['measurement']['statistics'][index] for r in reports]
        op['latency_available'] = op['whole_call_latency']['count'] > 0
        op['outcome_items'] = {name: sum(x['populations'][i]['input_items'] for x in originals)
                               for i, name in enumerate(outcomes)}
        op['outcome_latency'] = {name: merge_histograms(x['populations'][i]['whole_call']['raw']
                                                       for x in originals)
                                 for i, name in enumerate(outcomes)}
        op['terminal_rpc_codes'] = [sum(c['operations'][index]['terminal_rpc_codes'][i] for c in cases)
                                    for i in range(17)]
        op['completed_before_cutoff'] = sum(x['populations'][i]['completed_before_cutoff']
                                            for x in originals for i in range(len(outcomes)))
        op['completed_after_cutoff'] = op['calls'] - op['completed_before_cutoff']
    result['successful_whole_call_latency'] = merge_histograms(
        op['populations'][0]['whole_call']['raw'] for r in reports
        for op in r['metrics']['measurement']['statistics'])
    result['outcome_latency'] = {name: merge_histograms(op['populations'][i]['whole_call']['raw']
        for r in reports for op in r['metrics']['measurement']['statistics'])
        for i, name in enumerate(outcomes)}
    result['terminal_rpc_codes'] = [sum(op['terminal_rpc_codes'][i] for op in result['operations'])
                                   for i in range(17)]
    result['completed_before_cutoff'] = sum(op['completed_before_cutoff'] for op in result['operations'])
    result['completed_after_cutoff'] = result['calls'] - result['completed_before_cutoff']
    result['source_ordinals'] = [c['ordinal'] for c in cases]
    result['healthy_comparison_eligible'] = all(c['all_success_single_attempt'] and c['dropped_slots'] == 0
        and c['calls'] == c['issued'] == c['attempts'] == c['outcomes']['success'] for c in cases)
    return result


def measurement_comparison(old, new):
    return dict(successful_qps_change_percent=percent(old['successful_calls_per_second'], new['successful_calls_per_second']),
        completed_qps_change_percent=percent(old['completed_calls_per_second'], new['completed_calls_per_second']),
        successful_items_per_second_change_percent=percent(old['successful_input_items_per_second'], new['successful_input_items_per_second']),
        completed_items_per_second_change_percent=percent(old['completed_input_items_per_second'], new['completed_input_items_per_second']),
        mean_change_percent=percent(old['whole_call_latency']['mean_ns'], new['whole_call_latency']['mean_ns']),
        quantile_directions={q: bucket_direction(old['whole_call_latency'][q], new['whole_call_latency'][q])
                             for q in ('p50', 'p95', 'p99')},
        old_whole_call_latency=old['whole_call_latency'], new_whole_call_latency=new['whole_call_latency'])


def compare_pair(old, new):
    assert [(x['operation']) for x in old['operations']] == [x['operation'] for x in new['operations']]
    return dict(old_ordinals=old['source_ordinals'], new_ordinals=new['source_ordinals'],
        combined=measurement_comparison(old, new),
        operations=[dict(api_class='read' if i == 0 else 'write', operation=a['operation'],
                         comparison=measurement_comparison(a, b))
                    for i, (a, b) in enumerate(zip(old['operations'], new['operations'], strict=True))],
        server_cpu_change_percent=percent(old['server_cpu_cores'], new['server_cpu_cores']),
        client_cpu_change_percent=percent(old['client_cpu_cores'], new['client_cpu_cores']),
        healthy_comparison_eligible=old['healthy_comparison_eligible'] and new['healthy_comparison_eligible'])


def summarize(audit, reports):
    assert len(audit['cases']) == len(reports) == 48
    cases = []
    for case, report in zip(audit['cases'], reports, strict=True):
        row = {k: case[k] for k in ('ordinal', 'repeat', 'concurrency', 'arm', 'workload',
               'read_api', 'write_api', 'batch_size', 'read_percent', 'directory', 'report_sha256')}
        row.update(role=case['arm'].split('-', 1)[0], order='forward' if case['repeat'] == 0 else 'reverse',
                   server_memory=case['server_memory'], source_cpu_readback=report['_accepted_cpu'])
        row.update(aggregate_group([case], [report]))
        row['phases'] = {name: phase_accounting(metric, True) for name, metric in report['metrics'].items()}
        cases.append(row)
    pooled = []; pairs = []; pooled_pairs = []
    for concurrency in (1, 64):
        for workload in WORKLOADS:
            for role in ROLES:
                indices = [i for i, row in enumerate(cases) if
                    (row['concurrency'], row['workload'], row['role']) == (concurrency, workload, role)]
                assert len(indices) == 2 and {cases[i]['repeat'] for i in indices} == {0, 1}
                row = {k: cases[indices[0]][k] for k in
                       ('concurrency', 'workload', 'role', 'read_api', 'write_api', 'batch_size', 'read_percent')}
                row.update(aggregate_group([audit['cases'][i] for i in indices], [reports[i] for i in indices]))
                pooled.append(row)
            for repeat in (0, 1):
                selected = {r['role']: r for r in cases if
                    (r['concurrency'], r['workload'], r['repeat']) == (concurrency, workload, repeat)}
                assert set(selected) == set(ROLES)
                pairs.append(dict(concurrency=concurrency, workload=workload, repeat=repeat,
                                  **compare_pair(selected['old'], selected['new'])))
            selected = {r['role']: r for r in pooled if (r['concurrency'], r['workload']) == (concurrency, workload)}
            assert set(selected) == set(ROLES)
            pooled_pairs.append(dict(concurrency=concurrency, workload=workload,
                                     **compare_pair(selected['old'], selected['new'])))
    assert (len(pooled), len(pairs), len(pooled_pairs)) == (24, 24, 12)
    totals = {k: sum(r[k] for r in cases) for k in ('calls', 'input_items', 'issued', 'attempts', 'dropped_slots')}
    for key in ('outcomes', 'reasons', 'attempt_reasons'):
        totals[key] = add_maps(r[key] for r in cases)
    phase_totals = {}
    for phase in ('initialization', 'warmup', 'measurement', 'verification'):
        rows = [r['phases'][phase] for r in cases]
        phase_totals[phase] = {key: sum(x[key] for x in rows) for key in ('calls', 'input_items', 'attempts', 'extra_attempts')}
        for key in ('outcomes', 'reasons', 'attempt_reasons'):
            phase_totals[phase][key] = add_maps(x[key] for x in rows)
    return dict(cases=cases, pooled=pooled, pairs=pairs, pooled_pairs=pooled_pairs, totals=totals,
                phase_totals=phase_totals)


def markdown_table(rows, operations=False, repeat=False):
    labels = ['c', 'Cell', 'Role'] + (['Order'] if repeat else []) + (['API'] if operations else [])
    labels += ['Calls', 'Success calls/s', 'Completed calls/s', 'Success items/s', 'Mean us', 'p50 us', 'p95 us', 'p99 us', 'Outcomes']
    lines = ['| ' + ' | '.join(labels) + ' |', '|' + '|'.join('---' for _ in labels) + '|']
    for row in rows:
        for item in row['operations'] if operations else [row]:
            h = item['whole_call_latency']
            values = [str(row['concurrency']), row['workload'], row['role']]
            if repeat: values += [row['order']]
            if operations: values += [item['operation']]
            values += [str(item['calls']), f"{item['successful_calls_per_second']:.3f}",
                f"{item['completed_calls_per_second']:.3f}", f"{item['successful_input_items_per_second']:.3f}",
                'unavailable' if h['mean_ns'] is None else f"{h['mean_ns']/1000:.3f}",
                *(qtext(h, q) for q in (50, 95, 99)), json.dumps(item['outcomes'], sort_keys=True)]
            lines.append('| ' + ' | '.join(values) + ' |')
    return '\n'.join(lines) + '\n'


def write_outputs(output, result, inputs):
    note = ('Selected11113 versus CRCe748 full native six-cell regression. No automatic promotion. '
        'Inactive API counts/rates remain zero; their means/quantiles and zero-baseline deltas are unavailable. '
        'Mixed API rates each use the full cohort duration. Means are count-weighted sums; percentile buckets '
        'are merged, never averaged or divided by64. Every outcome and both orders remain.\n\n')
    content = {
        'summary.json': json.dumps(result, indent=2, sort_keys=True) + '\n',
        'input-hashes.json': json.dumps(inputs, indent=2, sort_keys=True) + '\n',
        'README.md': note + markdown_table(result['pooled']) + '\nSeparate operations:\n\n' + markdown_table(result['pooled'], True),
        'PER-REPEAT.md': note + markdown_table(result['cases'], repeat=True) + '\nSeparate operations:\n\n' + markdown_table(result['cases'], True, True),
    }
    cpu = ['Sampled CPU cores; ' + CPU_SCOPE, '', '| c | Cell | Role | Client | Voter1 | Voter2 | Voter3 | Server sum |', '|---|---|---|---|---|---|---|---|']
    for row in result['pooled']:
        cpu.append('| ' + ' | '.join([str(row['concurrency']), row['workload'], row['role'],
            *(f"{row['cpu'][k]['cpu_cores']:.6f}" for k in ('client', 'voter-1', 'voter-2', 'voter-3')),
            f"{row['server_cpu_cores']:.6f}"]) + ' |')
    content['CPU.md'] = '\n'.join(cpu) + '\n'
    comparisons = ['Positive rates mean higher throughput; negative means lower latency. Percentile differences retain bucket directions, not significance.', '', '| Scope | c | Cell | Repeat | API | Success calls/s % | Mean % | p99 direction |', '|---|---|---|---|---|---|---|---|']
    for scope, rows in [('repeat', result['pairs']), ('pooled', result['pooled_pairs'])]:
        for row in rows:
            for name, item in [('merged', row['combined'])] + [(x['operation'], x['comparison']) for x in row['operations']]:
                comparisons.append('| ' + ' | '.join(map(str, [scope, row['concurrency'], row['workload'], row.get('repeat', 'both'), name,
                    item['successful_qps_change_percent'], item['mean_change_percent'], item['quantile_directions']['p99']])) + ' |')
    content['COMPARISONS.md'] = '\n'.join(comparisons) + '\n'
    encoded = {name: value.encode() for name, value in content.items()}
    existing = sum(p.stat().st_size for p in HERE.rglob('*') if p.is_file())
    assert existing + sum(map(len, encoded.values())) <= CAP, '16 MiB complete preparation/output artifact cap'
    for name, data in encoded.items():
        with (output / name).open('xb') as stream: stream.write(data)


def main():
    ap = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    ap.add_argument('--output', type=Path, required=True)
    ap.add_argument('--protocol-sha256', required=True)
    ap.add_argument('--audit-sha256', required=True)
    ap.add_argument('--input-inventory-sha256', required=True)
    ap.add_argument('--timing-session', type=int, required=True)
    ap.add_argument('--audit-terminal', type=Path, required=True)
    ap.add_argument('--audit-terminal-sha256', required=True)
    args = ap.parse_args()
    assert __debug__ and os.sysconf('SC_CLK_TCK') == 100
    assert args.output.is_absolute() and args.output.parent == HERE and not args.output.exists()
    args.output.mkdir(exist_ok=False); used = {}; result = dict(complete=False, performance_promotion=False)
    def raw(path):
        path = Path(path); assert path.is_file() and not path.is_symlink() and path.stat().st_size <= CAP
        data = path.read_bytes(); pin = dict(bytes=len(data), sha256=hashlib.sha256(data).hexdigest())
        assert str(path) not in used or used[str(path)] == pin, 'input changed during reporting'
        used[str(path)] = pin; return data
    def read(path): return json.loads(raw(path))
    try:
        own = read(HERE / 'reader-pins.json')
        for name, pin in own['files'].items():
            raw(HERE / name); assert used[str(HERE / name)] == pin
        assert own['files']['core.py']['sha256'] == '827110ec99b2ae82f042e40fc81844263eefe4c484680ef6d8ab54653515b17f'
        protocol = read(PREP / 'protocol.json'); audit = read(AUDIT)
        inventory_path = AUDIT.with_name('input-inventory.json'); inventory = read(inventory_path)
        for path, expected in [(PREP/'protocol.json', args.protocol_sha256), (AUDIT, args.audit_sha256),
                               (inventory_path, args.input_inventory_sha256)]:
            assert used[str(path)]['sha256'] == expected
        runtime = read(HERE/'runtime-binding.json')
        assert args.protocol_sha256 == runtime['files']['protocol.json']['sha256']
        for name, pin in runtime['files'].items():
            raw(PREP/name); assert used[str(PREP/name)] == pin
        assert protocol['driver_sha256'] == runtime['files']['matched-driver.py']['sha256']
        assert protocol['auditor_sha256'] == runtime['files']['audit.py']['sha256']
        term = read(args.audit_terminal); assert used[str(args.audit_terminal)]['sha256'] == args.audit_terminal_sha256
        terminal_gate(term, args.audit_sha256, args.input_inventory_sha256)
        accepted_cases(audit, protocol, args.timing_session)
        def bound(path, case=None):
            data = raw(path)
            if Path(path).is_relative_to(RUN): report_binding(path, data, inventory, case)
            else: assert inventory[str(path)] == used[str(path)]
            return json.loads(data)
        matrix = bound(RUN/'matrix.json'); smoke = bound(SMOKE/'matrix.json')
        matrix_gate(matrix, smoke, protocol)
        assert matrix['helper_hashes'][str(PREP/'matched-driver.py')] == protocol['driver_sha256']
        assert smoke['helper_hashes'][str(PREP/'matched-driver.py')] == protocol['driver_sha256']
        reports = []
        for case, desc in zip(audit['cases'], protocol['timed_inventory'], strict=True):
            directory = Path(case['directory'])
            report = bound(directory/'run/report.json', case)
            config = bound(directory/'requested-config.json')
            assert all(config[k] == value for k, value in desc['shared'].items())
            samples = bound(directory/'resource-samples.json'); coverage = bound(directory/'resource-coverage.json')
            measurement_gate(case, report, desc)
            report['_accepted_cpu'] = cpu_from_samples(samples, coverage, report)
            reports.append(report)
        result.update(summarize(audit, reports))
        result.update(complete=True, protocol_id=PROTOCOL, timed_cohorts=48, smoke_cohorts=24,
            accepted_audit_sha256=args.audit_sha256, accepted_inventory_sha256=args.input_inventory_sha256,
            protocol_sha256=args.protocol_sha256, actual_timing_session=args.timing_session, audit_terminal=term,
            source_roles=matrix['role_bindings'], source_pins=dict(servers=protocol['server_pins'], client=protocol['client_pins']),
            checks={k: audit[k] for k in ('successful_arms', 'owned_lifetimes_exited', 'qualifying_drains',
                'voter_writer_listener_bindings', 'role_source_file_checks', 'resource_samples', 'outer_restoration')},
            smoke_checks=audit['smoke'], compressed_retention=audit['compressed_retention'],
            protocol_required_smoke_drains=72,
            smoke_drains_scope='72 is the unchanged protocol requirement of three fresh drains per smoke cohort; the audit has no separately summed smoke-drain field.',
            memory_comparison=audit['memory_comparison'], all_success_single_attempt=audit['all_success_single_attempt'],
            healthy_comparison_eligible=all(x['healthy_comparison_eligible'] for x in result['cases']),
            scope='Only this new full24-smoke/48-timed selected11113/CRCe748 native six-cell campaign. Smoke excluded from performance pooling; both exact timing orders retained. No historical pooling or promotion.',
            method='Exact unchanged core: sum calls/items over sum cohort time, sum nanoseconds/count, merged raw p50/p95/p99 buckets. Whole batch calls are not divided by64. Read/write rates each use the full interval.',
            cpu_scope=CPU_SCOPE, durability=protocol['durability'], original_audit_scope=audit['scope'],
            decision_scope='Accounting validity is separate from healthy comparison. All attempts/outcomes/drops retained; no significance, sustained-capacity, power-loss or new Chaos inference.')
        assert audit['all_success_single_attempt'] == all(c['all_success_single_attempt'] for c in audit['cases'])
        for path, pin in list(used.items()): raw(path); assert used[path] == pin
        write_outputs(args.output, result, used)
    except BaseException as error:
        failure = dict(complete=False, failure=repr(error), performance_promotion=False)
        (args.output/'failure.json').write_text(json.dumps(failure, indent=2)+'\n'); raise
    print(json.dumps({k: result[k] for k in ('complete', 'timed_cohorts', 'smoke_cohorts', 'healthy_comparison_eligible', 'performance_promotion')}))


if __name__ == '__main__': main()
