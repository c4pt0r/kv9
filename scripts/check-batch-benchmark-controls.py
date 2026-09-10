#!/usr/bin/env python3
"""Reject copied-report corruptions without changing the retained run or build."""
import argparse
import copy
import json
from pathlib import Path
import time

from batch_benchmark_report import bucket_bounds, report_check, validate
from workload_report import bounded, require, sha, strict_json


def controls(report):
    result = []

    def change(name, path, value):
        def mutate(r):
            target = r
            for key in path[:-1]:
                target = target[key]
            target[path[-1]] = value
        result.append((name, mutate))

    def field(name, value):
        change(name, [name], value)

    field('complete', False)
    field('failure', 'unconfirmed drain')
    field('full_history_recorded', True)
    field('independently_checked', True)
    field('workload_model', 'bounded_native_point_performance')
    field('version', True)
    field('process_id', report['process_id'] + 1)
    field('runtime_threads', 4)
    field('measurement_start_unix_ns', 0)
    field('task_failures', 1)
    field('measured_issued', report['measured_issued'] + 1)
    field('measured_completed', report['measured_completed'] - 1)
    field('cohort_elapsed_ns', report['cohort_elapsed_ns'] + 1)
    field('timing_eligible', not report['timing_eligible'])
    field('stop_reason', 'worker_failure')
    for name in ('completed_batches_per_second', 'successful_batches_per_second', 'successful_input_items_per_second'):
        field(name, report[name] + 1)
    change('configured-batch-size', ['configuration', 'batch_size'], 0)
    change('encoded-request-size', ['wire_sizes', 'put_request_bytes'], 1)
    change('in-flight-item-bound', ['wire_sizes', 'maximum_input_items_in_flight'], 0)
    change('build-revision', ['build', 'revision'], '0' * 40)
    change('missing-worker', ['workers'], report['workers'][:-1])
    change('worker-failure', ['workers', 0, 'stopped_for_failure'], True)
    change('worker-issued', ['workers', 0, 'issued'], report['workers'][0]['issued'] + 1)
    change('worker-identity-type', ['workers', 0, 'worker'], False)
    change('worker-terminal-after-stop', ['workers', 0, 'last_terminal_ns'], report['workers'][0]['stopped_ns'] + 1)
    change('worker-stop-after-report', ['workers', 0, 'stopped_ns'], (1 << 64) - 1)
    change('cutoff', ['stages', 'measurement', 'end_ns'], report['stages']['measurement']['end_ns'] + 1)
    change('overlapping-verification', ['stages', 'verification', 'start_ns'], 0)
    change('missing-drain', ['stages', 'drain', 'end_ns'], report['stages']['measurement']['start_ns'])
    phase = ['metrics', 'measurement']
    change('metric-validity', phase + ['valid'], False)
    change('operation-vocabulary', phase + ['operations'], ['batch_put', 'batch_get'])
    change('outcome-vocabulary', phase + ['outcomes'], ['success'])
    change('histogram-resolution', phase + ['histogram_subdivisions'], 32)
    kind = next(i for i, op in enumerate(report['metrics']['measurement']['statistics']) if op['populations'][0]['calls'])
    op = report['metrics']['measurement']['statistics'][kind]
    base = phase + ['statistics', kind]
    pop = op['populations'][0]
    hist = pop['whole_call']
    p = base + ['populations', 0]
    change('data-integrity-failure', base + ['data_failures'], 1)
    change('missing-terminal-reason', base + ['reasons', 0], op['reasons'][0] - 1)
    change('invented-terminal-status', base + ['rpc_codes', 14], op['rpc_codes'][14] + 1)
    change('invented-attempt-status', base + ['attempt_rpc_codes', 14], op['attempt_rpc_codes'][14] + 1)
    change('omitted-attempt', base + ['attempt_reasons', 0], op['attempt_reasons'][0] - 1)
    change('input-item-understatement', p + ['input_items'], pop['input_items'] - 1)
    change('omitted-terminal-call', p + ['calls'], pop['calls'] + 1)
    change('too-many-before-cutoff', p + ['completed_before_cutoff'], pop['calls'] + 1)
    change('wrong-before-cutoff-count-within-range', p + ['completed_before_cutoff'],
           pop['completed_before_cutoff'] - 1 if pop['completed_before_cutoff'] else 1)
    change('boolean-count', p + ['calls'], True)
    h = p + ['whole_call']
    change('histogram-overflow', h + ['raw', 'valid'], False)
    change('histogram-bucket-count', h + ['raw', 'buckets'], [0] * 65)
    change('histogram-count-omission', h + ['raw', 'count'], hist['raw']['count'] + 1)
    change('impossible-histogram-sum', h + ['raw', 'sum_ns'], 0)
    change('impossible-histogram-extrema', h + ['raw', 'min_ns'], hist['raw']['max_ns'] + 1)
    change('invented-histogram-mean', h + ['mean_ns'], hist['mean_ns'] + 1)
    change('non-finite-histogram-mean', h + ['mean_ns'], float('inf'))
    change('invented-p99', h + ['p99'], {'lower_ns': 0, 'upper_ns': 0})
    change('boolean-p50', h + ['p50', 'lower_ns'], True)
    change('missing-failure-population', base + ['populations'], op['populations'][:-1])
    change('omitted-unknown-latency', base + ['populations', 2, 'calls'], 1)
    change('verification-sentinel-omission', ['metrics', 'verification', 'statistics', 0, 'populations', 0, 'input_items'], report['configuration']['keys'])
    if report['offered_slots'] is not None:
        field('offered_slots', report['offered_slots'] + 1)
        field('dropped_slots', report['dropped_slots'] + 1)
        change('omitted-dispatch-delay', base + ['dispatch_lateness'], report['metrics']['initialization']['statistics'][0]['dispatch_lateness'])
        change('omitted-scheduled-latency', p + ['scheduled_to_completion'], report['metrics']['initialization']['statistics'][0]['populations'][0]['scheduled_to_completion'])
    else:
        field('offered_slots', report['measured_issued'])
        field('dropped_slots', 1)
        def exceed_span(r):
            def constant(count, sample):
                if not count:
                    return dict(raw=dict(count=0, sum_ns=0, min_ns=None, max_ns=None, valid=True, buckets=[]),
                                mean_ns=None, p50=None, p95=None, p99=None)
                slot = next(i for i in range(3776) if bucket_bounds(i)[0] <= sample <= bucket_bounds(i)[1])
                buckets = [0] * 3776
                buckets[slot] = count
                low, high = bucket_bounds(slot)
                interval = dict(lower_ns=low, upper_ns=high)
                return dict(raw=dict(count=count, sum_ns=count*sample, min_ns=sample, max_ns=sample,
                                     valid=True, buckets=buckets), mean_ns=float(sample),
                            p50=interval.copy(), p95=interval.copy(), p99=interval.copy())
            for op in r['metrics']['measurement']['statistics']:
                for pop in op['populations']:
                    pop['whole_call'] = constant(pop['calls'], 1_000_000_000_000)
                    pop['sdk_call'] = constant(pop['calls'], 500_000_000_000)
        result.append(('whole-and-sdk-samples-exceed-cohort', exceed_span))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--directory', type=Path, action='append', required=True)
    parser.add_argument('--build-directory', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    require(not args.output.exists(), 'control output must be new')
    args.output.mkdir()
    summary = {'complete': False, 'started_unix_ns': time.time_ns(), 'runs': [],
               'scope': 'In-memory corruptions of genuine reports; not new runtime, fault or timing evidence'}
    summary_path = args.output / 'summary.json'
    try:
        for directory in args.directory:
            accepted = validate(directory, args.build_directory)
            original = bounded(directory / 'report.json', 16 * 1024 * 1024)
            report = strict_json(original)
            c, b = copy.deepcopy(report['configuration']), copy.deepcopy(report['build'])
            row = {'directory': str(directory), 'accepted_original': accepted, 'controls': []}
            summary['runs'].append(row)
            for name, mutate in controls(report):
                report_check(copy.deepcopy(report), c, b)
                altered = copy.deepcopy(report)
                mutate(altered)
                try:
                    report_check(altered, c, b)
                except ValueError as error:
                    row['controls'].append({'name': name, 'rejected': True, 'reason': str(error)})
                else:
                    raise ValueError('corrupted report was accepted: ' + name)
                report_check(copy.deepcopy(report), c, b)
            require(bounded(directory / 'report.json', 16 * 1024 * 1024) == original, 'original report changed')
            row['original_unchanged_sha256'] = sha(original)
            row['accepted_restored'] = validate(directory, args.build_directory)
        summary['complete'] = True
    finally:
        summary['ended_unix_ns'] = time.time_ns()
        summary_path.write_text(json.dumps(summary, indent=2, sort_keys=True) + '\n')
    print('PASS: copied-report corruptions rejected; original reports revalidated unchanged')


if __name__ == '__main__':
    main()
