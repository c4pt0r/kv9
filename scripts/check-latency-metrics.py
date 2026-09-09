#!/usr/bin/env python3
"""Validate bounded latency snapshots against real pressure and I/O fault effects."""
import argparse
import copy
import json
from pathlib import Path

OUTCOMES = ['success', 'error', 'aborted', 'released', 'replaced', 'rejected', 'unconfirmed']
NAMES = [f'public_{kind}_{phase}' for kind in ['raw_read', 'raw_write', 'metadata_read', 'metadata_write', 'transaction']
         for phase in ['prepare_queue', 'backend']]
NAMES += ['raft_proposal_submission', 'runtime_logical_proposal_wait', 'raft_application_wait',
          'raft_read_establishment', 'raft_command_apply', 'raft_pump_service',
          'raft_pump_idle_wait', 'raft_pump_iteration_spacing']
NAMES += [f'{kind}_wal_{operation}' for kind in ['raft', 'engine']
          for operation in ['record_write', 'record_sync', 'recovery_sync', 'namespace_publish']]
MAX = 2**64 - 1


def fields(path):
    return dict(line.split('=', 1) for line in path.read_text().splitlines())


def load(path):
    assert path.stat().st_size <= 512 * 1024, 'export exceeds fixed cap'
    value = json.loads(path.read_text())
    validate(value)
    return value


def bounds(index):
    return {'lower_ns': 0 if index == 0 else 2**(index - 1), 'upper_ns': 2**index - 1}


def validate(doc):
    assert doc['schema_version'] == 2 and doc['bucket_count'] == 65
    assert doc['clock'] == 'monotonic_instant' and doc['duration_unit'] == 'nanoseconds'
    assert doc['reset'] == 'node_component_construction'
    assert doc['snapshot_consistency'] == 'coherent_per_metric_independent_between_metrics'
    assert [metric['name'] for metric in doc['metrics']] == NAMES, 'metric inventory changed'
    assert int(doc['exporter_created_unix_ns']) <= int(doc['captured_unix_ns'])
    for metric in doc['metrics']:
        latency = metric['latency']
        assert latency['valid'], 'invalid observer cannot justify evidence'
        assert [h['outcome'] for h in latency['outcomes']] == OUTCOMES
        for h in latency['outcomes']:
            assert len(h['buckets']) == 65
            assert all(type(n) is int and 0 <= n <= MAX for n in h['buckets'] + [h['count'], h['sum_ns']])
            assert not h['count_saturated'] and not h['sum_saturated'] and not h['duration_clamped'], 'run exceeded representable measurement'
            assert sum(h['buckets']) == h['count'], 'histogram conservation failed'
            if h['count']:
                assert 0 <= h['min_ns'] <= h['max_ns'] <= MAX
                assert h['min_ns'] * h['count'] <= h['sum_ns'] <= h['max_ns'] * h['count']
            else:
                assert h['min_ns'] is None and h['max_ns'] is None and h['sum_ns'] == 0
            for p in [50, 95, 99]:
                expected = None
                if h['count']:
                    rank, cumulative = (h['count'] * p + 99) // 100, 0
                    for i, n in enumerate(h['buckets']):
                        cumulative += n
                        if cumulative >= rank:
                            expected = bounds(i)
                            break
                assert h[f'p{p}'] == expected, 'percentile is not the bucket containing its rank'
    lag = doc['apply_lag']
    pair = lag['driver_applied_term'], lag['driver_applied_index']
    assert (pair[0] is None) == (pair[1] is None), 'torn applied pair'
    compatible = (pair[0] is not None and lag['term_before'] == lag['term_after']
                  and lag['committed_before'] == lag['committed_after']
                  and pair[0] <= lag['term_after'] and pair[1] <= lag['committed_after'])
    assert lag['lag_entries'] == (lag['committed_after'] - pair[1] if compatible else None)


def histogram(doc, name, outcome='success'):
    return next(m for m in doc['metrics'] if m['name'] == name)['latency']['outcomes'][OUTCOMES.index(outcome)]


def count(doc, name, outcome='success'):
    return histogram(doc, name, outcome)['count']


def identity(doc):
    return doc['node_id'], doc['process_id'], doc['exporter_created_unix_ns']


def pressure(root, after_required=True):
    before, during = [load(root / f'admission-{p}.metrics.json') for p in ['before', 'during']]
    assert identity(before) == identity(during), 'pressure snapshots span a restart'
    assert int(before['captured_unix_ns']) < int(during['captured_unix_ns'])
    for name in ['public_raw_read_prepare_queue', 'public_raw_read_backend', 'raft_read_establishment']:
        assert count(during, name) > count(before, name), f'pressure produced no observed {name}'
        assert histogram(during, name)['sum_ns'] > histogram(before, name)['sum_ns']
    if not after_required:
        return
    after = load(root / 'admission-after.metrics.json')
    assert identity(after) == identity(before), 'post-pressure snapshot spans a restart'
    done = json.loads((root / 'admission-pressure.jsonl').read_text().splitlines()[-1])
    assert int(after['captured_unix_ns']) > done['time_ns'], 'post-pressure export is stale'
    for phase in ['before', 'during', 'after']:
        doc = {'before': before, 'during': during, 'after': after}[phase]
        state = fields(root / f'admission-{phase}.status')
        assert int(state['pid']) == doc['process_id'] and int(state['node_id']) == doc['node_id']
    assert count(after, 'public_raw_read_backend') - count(before, 'public_raw_read_backend') >= done['missing'], 'completed pressure reads missing from backend samples'
    assert count(after, 'public_raw_read_prepare_queue') - count(before, 'public_raw_read_prepare_queue') >= done['missing']
    return {'node_id': after['node_id'], 'pressure_reads': done['missing'],
            'backend_samples_delta': count(after, 'public_raw_read_backend') - count(before, 'public_raw_read_backend')}


def io_cell(root, label):
    failed = load(root / f'{label}-metrics.json')
    state = fields(root / f'{label}-status.txt')
    exit_state = fields(root / f'{label}-exit.txt')
    victim, errno = int(label.split('-')[2]), int(label.split('-')[4])
    assert failed['node_id'] == victim and failed['process_id'] == int(state['pid']), 'metrics are not from the failed process'
    assert int(exit_state['exit_code']) == 1 and 'fatal Raft persistence failure during ' in state['fatal']
    assert f'os error {errno}' in state['fatal'], 'intended errno did not reach the real Raft path'
    assert count(failed, 'raft_wal_record_write', 'error') == 1, 'actual failed record write must be observed once'
    assert count(failed, 'raft_wal_record_sync', 'error') == 0, 'failed write fabricated an fsync failure'
    assert count(failed, 'raft_wal_record_write') == count(failed, 'raft_wal_record_sync'), 'failed write did not short-circuit fsync'
    recovered = load(root / f'{label}-recovered-metrics.json')
    recovered_state = fields(root / f'{label}-recovered-status.txt')
    assert recovered['node_id'] == victim and recovered['process_id'] == int(recovered_state['pid'])
    assert identity(failed) != identity(recovered), 'recovery must report the fresh process'
    assert int(recovered['captured_unix_ns']) > int(failed['captured_unix_ns'])
    assert count(recovered, 'raft_wal_record_write', 'error') == 0, 'restart did not reset observers'
    assert count(recovered, 'raft_wal_record_write') > 0 and count(recovered, 'engine_wal_record_sync') > 0
    return {'node_id': victim, 'errno': errno, 'failed_write_samples': 1,
            'failed_process': failed['process_id'], 'recovered_process': recovered['process_id']}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', type=Path)
    parser.add_argument('--pressure', choices=['during', 'after'])
    parser.add_argument('--io')
    args = parser.parse_args()
    if args.pressure:
        pressure(args.root, args.pressure == 'after')
        return
    if args.io:
        io_cell(args.root, args.io)
        return
    result = {'pressure': pressure(args.root), 'io_cells': [io_cell(args.root, f'io-voter-{v}-errno-{e}') for v in [1, 2, 3] for e in [5, 28]]}
    source = load(args.root / 'admission-after.metrics.json')
    # Isolated evidence controls: each must reject a distinct invalid statement.
    cases = []
    mutant = copy.deepcopy(source); mutant['metrics'][0]['latency']['outcomes'][0]['count'] += 1
    cases.append(('inconsistent-histogram', mutant, 'histogram conservation failed'))
    mutant = copy.deepcopy(source); mutant['metrics'][0]['latency']['outcomes'][0]['p99'] = {'lower_ns': 0, 'upper_ns': 0}
    cases.append(('invented-quantile', mutant, 'percentile is not the bucket containing its rank'))
    mutant = copy.deepcopy(source); mutant['metrics'].pop()
    cases.append(('missing-boundary', mutant, 'metric inventory changed'))
    for name, mutant, expected in cases:
        try:
            validate(mutant)
        except AssertionError as error:
            assert str(error) == expected, f'{name}: wrong rejection: {error}'
        else:
            raise AssertionError(f'{name}: invalid evidence was accepted')
    result['evidence_controls'] = [name for name, _, _ in cases]
    (args.root / 'latency-metrics-report.json').write_text(json.dumps(result, indent=2) + '\n')
    print('PASS: latency metrics cover real pressure, six I/O failures and recovery; three invalid evidence controls rejected')


if __name__ == '__main__':
    main()
