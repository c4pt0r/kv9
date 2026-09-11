#!/usr/bin/env python3
"""Derive source-bound endpoint metric deltas after the independent v3 audit."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import sys

if not __debug__:
    raise RuntimeError('assertions must remain enabled')
sys.dont_write_bytecode = True

SELECTED = (
    'public_raw_write_prepare_queue', 'public_raw_write_backend',
    'raft_proposal_submission', 'runtime_logical_proposal_wait',
    'raft_application_wait', 'raft_command_apply',
    'raft_wal_record_write', 'raft_wal_record_sync',
    'engine_wal_record_write', 'engine_wal_record_sync',
)
SCOPE = ('Independent before/after metric deltas over the complete client envelope, '
         'including setup, warmup, measurement, drain and final readback. '
         'Metrics have independent populations and overlapping intervals. '
         'No additive phase partition or measured-only latency attribution.')


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def delta(before, after, bounds):
    require(before['outcome'] == after['outcome'], 'outcome order differs')
    buckets = [b - a for a, b in zip(before['buckets'], after['buckets'], strict=True)]
    count, total = after['count'] - before['count'], after['sum_ns'] - before['sum_ns']
    require(count >= 0 and total >= 0 and all(n >= 0 for n in buckets), 'counter regression')
    require(sum(buckets) == count and (count > 0 or total == 0), 'delta conservation fails')
    lower = sum(n * bounds(i)['lower_ns'] for i, n in enumerate(buckets))
    upper = sum(n * bounds(i)['upper_ns'] for i, n in enumerate(buckets))
    require(lower <= total <= upper, 'sum is inconsistent with delta bucket bounds')
    result = dict(outcome=after['outcome'], count=count, sum_ns=total,
                  mean_ns=total / count if count else None)
    for percent in (50, 95, 99):
        interval = None
        if count:
            rank, cumulative = (count * percent + 99) // 100, 0
            for i, n in enumerate(buckets):
                cumulative += n
                if cumulative >= rank:
                    interval = bounds(i)
                    break
        result['p' + str(percent)] = interval
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument('--run', type=Path, required=True)
    parser.add_argument('--audit', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    require(not args.output.exists(), 'output already exists; retain first derivation')
    run = args.run.resolve()
    inputs = {}
    accepted_inputs = None

    def read(path):
        path = Path(path).resolve()
        raw = path.read_bytes()
        inputs[str(path)] = dict(bytes=len(raw), sha256=hashlib.sha256(raw).hexdigest())
        if accepted_inputs is not None and path.is_relative_to(run):
            require(str(path) in accepted_inputs and accepted_inputs[str(path)] == inputs[str(path)],
                    'cohort input differs from accepted inventory: ' + str(path))
        return json.loads(raw)

    audit = read(args.audit)
    inventory = read(args.audit.parent / 'input-inventory.json')
    accepted_inputs = inventory
    matrix_path = run / 'matrix.json'
    require(audit['complete'] and audit['matched_diagnostic_accepted'], 'independent audit not accepted')
    require(inventory[str(matrix_path)]['sha256'] == digest(matrix_path), 'audit belongs to another matrix')
    matrix = read(matrix_path)
    require(matrix['complete'] and not matrix['smoke'] and
            matrix['protocol_id'] == 'kv9-v3-workloads-c1-c64-v1', 'wrong matrix scope')
    checkers = {}
    for role in ('old', 'new'):
        binding = matrix['role_bindings'][role]['source']
        relative = 'scripts/check-latency-metrics.py'
        path = Path(binding['path']) / relative
        require(digest(path) == binding['sources'][relative], 'metric checker differs from server source')
        inputs[str(path)] = dict(bytes=path.stat().st_size, sha256=digest(path))
        spec = importlib.util.spec_from_file_location('server_metrics_' + role, path)
        checker = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(checker)
        checkers[role] = checker
    cases = []
    for attempt in matrix['attempts']:
        desc = attempt['descriptor']
        role = desc['server_role']
        if role is None:
            continue
        directory = Path(attempt['directory'])
        require(directory.is_relative_to(run), 'cohort escapes matrix root')
        report = read(directory / 'run/report.json')
        endpoints = {side: read(directory / (side + '-metrics.json')) for side in ('before', 'after')}
        statuses = {side: read(directory / (side + '-status.json')) for side in ('before', 'after')}
        resources = {side: read(directory / (side + '-resources.json')) for side in ('before', 'after')}
        drains = {side: read(directory / (label + '-fresh-drain.json'))
                  for side, label in (('before', 'before'), ('after', 'post-readback'))}
        cleanup = read(directory / 'cleanup.json')
        exited = read(directory / 'client-exit.json')
        identities = {child['identity']['pid']: child['identity'] for child in cleanup['children']}
        require(report['process_id'] in identities and exited['exit_code'] == 0,
                'client lacks accepted lifetime and exit')
        require(set(endpoints['before']) == set(endpoints['after']) == {'1', '2', '3'}, 'missing voter')
        expected_writes = sum(phase['statistics'][1]['populations'][0]['calls']
                              for phase in report['metrics'].values())
        nodes, observed_writes = {}, 0
        checker = checkers[role]
        for node in sorted(endpoints['before']):
            before, after = (endpoints[side][node] for side in ('before', 'after'))
            checker.validate(before)
            checker.validate(after)
            require(checker.identity(before) == checker.identity(after), 'exporter changed')
            require(int(before['captured_unix_ns']) < int(after['captured_unix_ns']), 'export time regressed')
            for side, document in (('before', before), ('after', after)):
                state = statuses[side][node]
                resource = resources[side]['processes'][node]
                require(document['node_id'] == int(node) and document['process_id'] == int(state['pid']) == resource['pid'],
                        'snapshot identity mismatch')
                require(resource['start_ticks'] == int(state['process_start_ticks']), 'resource lifetime differs')
                identity = identities.get(resource['pid'])
                require(identity is not None and identity['start_ticks'] == resource['start_ticks'] and
                        identity['boot_id'] == state['process_boot_id'], 'endpoint differs from audited lifetime')
                bound = drains[side]['final'][node]
                require(all(state[key] == bound[key] for key in ('pid', 'process_start_ticks', 'process_boot_id')),
                        'endpoint differs from audited drain writer')
                require(int(document['captured_unix_ns']) >= resources[side]['requested_unix_ns'], 'stale export')
                require(int(document['captured_unix_ns']) > drains[side]['completed_unix_ns'],
                        'endpoint capture precedes its completed drain')
                require(document['export_failures_before_capture'] == 0 and not document['export_failures_saturated'],
                        'exporter failure')
            require(all(statuses['before'][node][key] == statuses['after'][node][key]
                        for key in ('pid', 'process_start_ticks', 'process_boot_id')), 'server restarted')
            require(int(before['captured_unix_ns']) < identities[report['process_id']]['observed_unix_ns'] <
                    exited['observed_unix_ns'] < drains['after']['started_unix_ns'] <
                    int(after['captured_unix_ns']), 'snapshots do not bracket the drained client envelope')
            old = {metric['name']: metric['latency']['outcomes'] for metric in before['metrics']}
            new = {metric['name']: metric['latency']['outcomes'] for metric in after['metrics']}
            metrics = {name: [delta(a, b, checker.bounds) for a, b in zip(old[name], new[name], strict=True)]
                       for name in SELECTED}
            observed_writes += metrics['public_raw_write_backend'][0]['count']
            nodes[node] = dict(pid=before['process_id'], metrics=metrics)
        require(observed_writes >= expected_writes, 'endpoint counters omit successful client writes')
        cases.append(dict(descriptor=desc, directory=str(directory), nodes=nodes,
                          client_successful_writes_all_phases=expected_writes,
                          observed_public_write_successes=observed_writes))
    require(len(cases) == 48, 'expected all 48 native cohorts')
    require(all(Path(p).stat().st_size == record['bytes'] and digest(p) == record['sha256']
                for p, record in inputs.items()), 'input changed during derivation')
    result = dict(complete=True, scope=SCOPE, audit_sha256=digest(args.audit),
                  matrix_sha256=digest(matrix_path), cases=cases, inputs=inputs)
    args.output.write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(dict(complete=True, native_cohorts=len(cases), inputs=len(inputs), scope=SCOPE)))


if __name__ == '__main__':
    main()
