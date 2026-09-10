#!/usr/bin/env python3
"""Reject copied Redis batch reports while retaining and restoring genuine evidence."""
import argparse
import copy
import json
from pathlib import Path
import time
from redis_batch_report import report_check, validate
from workload_report import bounded, require, sha, strict_json


def controls(r):
    result = []
    def change(name, path, value):
        def mutate(r):
            target = r
            for p in path[:-1]: target = target[p]
            target[path[-1]] = value
        result.append((name, mutate))
    for field, value in [('complete', False), ('failure', 'unconfirmed'), ('version', True),
                         ('workload_model', 'bounded_native_batch_performance'), ('protocol', 'resp3'),
                         ('full_history_recorded', True), ('independently_checked', True),
                         ('runtime_threads', 4), ('task_failures', 1), ('stop_reason', 'worker_failure'),
                         ('preconnected_workers', r['preconnected_workers']+1),
                         ('measured_issued', r['measured_issued']+1), ('measured_completed', r['measured_completed']-1),
                         ('cohort_elapsed_ns', r['cohort_elapsed_ns']+1), ('timing_eligible', not r['timing_eligible']),
                         ('completed_batches_per_second', r['completed_batches_per_second']+1),
                         ('successful_input_items_per_second', r['successful_input_items_per_second']+1)]:
        change(field, [field], value)
    change('source-revision', ['build', 'revision'], '0'*40)
    change('wire-size', ['wire_sizes', 'mset_request_bytes'], 0)
    change('configuration', ['configuration', 'workers'], True)
    change('missing-worker', ['workers'], r['workers'][:-1])
    change('worker-issued', ['workers', 0, 'issued'], r['workers'][0]['issued']+1)
    change('worker-stop-after-report', ['workers', 0, 'stopped_ns'], (1 << 64)-1)
    change('cutoff', ['stages', 'measurement', 'end_ns'], r['stages']['measurement']['end_ns']+1)
    change('verification-overlap', ['stages', 'verification', 'start_ns'], 0)
    phase = ['metrics', 'measurement']
    change('missing-outcome', phase+['outcomes'], ['success'])
    change('false-validity', phase+['valid'], False)
    kind = next(i for i, op in enumerate(r['metrics']['measurement']['statistics']) if op['populations'][0]['calls'])
    base = phase+['statistics', kind]
    op = r['metrics']['measurement']['statistics'][kind]
    pop = op['populations'][0]
    p = base+['populations', 0]
    change('wrong-cutoff-population', p+['completed_before_cutoff'], pop['completed_before_cutoff']-1 if pop['completed_before_cutoff'] else 1)
    change('omitted-input-item', p+['input_items'], pop['input_items']-1)
    change('missing-call', p+['calls'], pop['calls']+1)
    change('missing-reason', base+['reasons', 0], op['reasons'][0]-1)
    change('command-replay', base+['command_attempts'], op['command_attempts']+1)
    change('connection-failure-hidden', base+['connection_failures'], op['connection_failures']+1)
    change('protocol-failure', base+['data_failures'], 1)
    change('omitted-latency', p+['whole_call', 'raw', 'count'], pop['calls']-1)
    change('impossible-time-sum', p+['whole_call', 'raw', 'sum_ns'], 0)
    change('invented-p99', p+['whole_call', 'p99'], dict(lower_ns=0, upper_ns=0))
    change('boolean-mean', p+['client_call', 'mean_ns'], True)
    change('sentinel-omitted', ['metrics', 'verification', 'statistics', 0, 'populations', 0, 'input_items'], r['configuration']['keys'])
    if r['offered_slots'] is not None:
        change('omitted-offered-slot', ['offered_slots'], r['offered_slots']-1)
        change('omitted-dropped-slot', ['dropped_slots'], r['dropped_slots']+1)
        change('omitted-dispatch-lateness', base+['dispatch_lateness'], r['metrics']['initialization']['statistics'][0]['dispatch_lateness'])
    else:
        change('invented-offered-load', ['offered_slots'], r['measured_issued'])
    return result


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--directory', type=Path, action='append', required=True)
    p.add_argument('--build-directory', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    a = p.parse_args(); a.output.mkdir(exist_ok=False)
    result = dict(complete=False, started_unix_ns=time.time_ns(), runs=[])
    try:
        for directory in a.directory:
            accepted = validate(directory, a.build_directory)
            raw = bounded(directory/'report.json', 16*1024*1024); r = strict_json(raw)
            c, b = copy.deepcopy(r['configuration']), copy.deepcopy(r['build'])
            row = dict(directory=str(directory), accepted_original=accepted, controls=[]); result['runs'].append(row)
            for name, mutate in controls(r):
                report_check(copy.deepcopy(r), c, b)
                changed = copy.deepcopy(r); mutate(changed)
                try: report_check(changed, c, b)
                except ValueError as error: row['controls'].append(dict(name=name, rejected=True, reason=str(error)))
                else: raise ValueError('corrupted report accepted: '+name)
                report_check(copy.deepcopy(r), c, b)
            require(bounded(directory/'report.json', 16*1024*1024) == raw, 'original report changed')
            row['original_unchanged_sha256'] = sha(raw)
            row['accepted_restored'] = validate(directory, a.build_directory)
        result['complete'] = True
    finally:
        result['ended_unix_ns'] = time.time_ns()
        (a.output/'summary.json').write_text(json.dumps(result, indent=2, sort_keys=True)+'\n')
    print('PASS: Redis report corruptions rejected; originals revalidated unchanged')


if __name__ == '__main__': main()
