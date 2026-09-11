#!/usr/bin/env python3
"""Verify portable bytes and recorded statistics; does not rerun runtime acceptance."""
import argparse
import gzip
import hashlib
import json
from pathlib import Path


def sha(data):
    return hashlib.sha256(data).hexdigest()


def main():
    if not __debug__:
        raise RuntimeError('Python assertions must be enabled')
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check-originals', action='store_true', help='also require local original bytes')
    parser.add_argument('--expected-index-sha256', help='optional independently supplied index identity')
    args = parser.parse_args()
    here = Path(__file__).resolve().parent
    index_bytes = (here / 'index.json').read_bytes()
    if args.expected_index_sha256:
        assert sha(index_bytes) == args.expected_index_sha256
    index = json.loads(index_bytes)
    assert index['version'] == 1 and index['cohorts'] == index['coverage_files'] == 72
    originals, paths = {}, {}
    for entry in index['files']:
        relative = Path(entry['path'])
        path = here / relative
        assert not relative.is_absolute() and '..' not in relative.parts
        assert path.resolve().is_relative_to(here) and path.is_file() and not path.is_symlink()
        data = path.read_bytes()
        assert len(data) == entry['stored_bytes'] and sha(data) == entry['stored_sha256'], relative
        assert entry['encoding'] in {'gzip', 'identity'}
        decoded = gzip.decompress(data) if entry['encoding'] == 'gzip' else data
        assert len(decoded) == entry['decoded_bytes'] and sha(decoded) == entry['decoded_sha256'], relative
        assert entry['source'] not in originals and str(relative) not in paths
        originals[entry['source']] = decoded
        paths[str(relative)] = decoded
        if args.check_originals:
            source = Path(entry['source'])
            assert source.is_file() and not source.is_symlink() and source.read_bytes() == decoded, source
    for entry in index['generated_files']:
        path = here / entry['path']
        assert path.resolve().is_relative_to(here) and not path.is_symlink()
        data = path.read_bytes()
        assert len(data) == entry['bytes'] and sha(data) == entry['sha256'], entry['path']
    for entry in index['aliases']:
        path = here / entry['path']
        assert path.resolve().is_relative_to(here) and not path.is_symlink()
        data = path.read_bytes()
        assert len(data) == entry['bytes'] and sha(data) == entry['sha256']
        assert data == originals[entry['source']]
    assert len(originals) == index['copies']
    assert sum(r['stored_bytes'] for r in index['files']) == index['stored_bytes']
    assert sum(r['decoded_bytes'] for r in index['files']) == index['decoded_bytes']

    def bundled_json(relative):
        return json.loads(paths[relative] if relative in paths else paths[relative + '.gz'])

    def original_json(name):
        return json.loads(originals[name])

    audit = bundled_json('audit/audit.json')
    inventory = bundled_json('audit/input-inventory.json')
    stats = bundled_json('statistics/statistics-first.json')
    inputs = bundled_json('statistics/statistics-inputs-first.json')
    gate = bundled_json('root/audit-terminal-second.json')
    assert audit['complete'] and audit['matched_diagnostic_accepted'] and stats['complete']
    assert audit['protocol_id'] == stats['protocol_id'] == index['protocol_id']
    assert gate['terminal'] and gate['exit_code'] == 0
    assert gate['timing_session'] == audit['parent_confirmed_session'] == 34148
    assert sha(paths.get('audit/audit.json', paths.get('audit/audit.json.gz'))) == inputs['audit_sha256'] == stats['accepted_audit_sha256']
    assert sha(paths.get('audit/input-inventory.json', paths.get('audit/input-inventory.json.gz'))) == stats['accepted_inventory_sha256']
    assert sha(paths['root/audit-terminal-second.json']) == inputs['gate_sha256']
    assert sha(paths['statistics/core.py']) == inputs['core_sha256']
    assert sha(paths['statistics/derive.py']) == inputs['reader_sha256']
    assert len(inputs['files']) == 144
    for name, binding in inputs['files'].items():
        data = originals[name]
        assert dict(bytes=len(data), sha256=sha(data)) == binding == inventory[name], name

    # Only the already recorded pure arithmetic core is executed, never a driver,
    # auditor, original absolute-path reader, process, or external command.
    core = {}
    exec(compile(paths['statistics/core.py'], 'bundled statistics/core.py', 'exec'), core)
    aggregate, phase, merge, compare, add = [core[n] for n in
        ['aggregate', 'phase_accounting', 'merge_histograms', 'compare', 'add_maps']]
    reports = {}
    assert len(audit['cases']) == len(stats['cases']) == 72
    for case, recorded in zip(audit['cases'], stats['cases']):
        ordinal = case['ordinal']
        assert ordinal == recorded['ordinal'] == len(reports)
        report_path = case['directory'] + '/run/report.json'
        coverage_path = case['directory'] + '/resource-coverage.json'
        assert sha(originals[report_path]) == case['report_sha256']
        report = original_json(report_path)
        report['_accepted_cpu'] = original_json(coverage_path)['cpu']
        reports[ordinal] = report
        for operation, audited in zip(report['metrics']['measurement']['statistics'], case['operations']):
            assert merge(p['whole_call']['raw'] for p in operation['populations']) == audited['whole_call_latency']
        result = aggregate([case], [report])
        for key, value in result.items():
            assert recorded[key] == value, (ordinal, key)
        assert case['whole_call_latency'] == result['whole_call_latency']
        role = case['arm'].split('-', 1)[0]
        assert recorded['role'] == role
        for key in ['repeat', 'concurrency', 'arm', 'workload', 'read_api', 'write_api',
                    'batch_size', 'read_percent', 'directory', 'report_sha256',
                    'all_success_single_attempt', 'dataset', 'storage']:
            assert recorded[key] == case[key], (ordinal, key)
        assert recorded['phases'] == {name: phase(metrics, role != 'redis')
                                      for name, metrics in report['metrics'].items()}
    assert len(stats['pooled']) == 36 and len(stats['pairs']) == 24 and len(stats['pooled_pairs']) == 12
    for row in stats['pooled']:
        selected = [c for c in stats['cases'] if (c['concurrency'], c['workload'], c['role']) ==
                    (row['concurrency'], row['workload'], row['role'])]
        assert len(selected) == 2 and {c['repeat'] for c in selected} == {0, 1}
        ordinals = [c['ordinal'] for c in selected]
        assert row['source_ordinals'] == ordinals
        result = aggregate([audit['cases'][i] for i in ordinals], [reports[i] for i in ordinals])
        for key, value in result.items():
            assert row[key] == value, (ordinals, key)
    for row in stats['pairs']:
        selected = {c['role']: c for c in stats['cases'] if
            (c['concurrency'], c['workload'], c['repeat']) == (row['concurrency'], row['workload'], row['repeat'])}
        assert set(selected) == {'old', 'new', 'redis'}
        assert row['comparison'] == compare(selected['old'], selected['new'])
    for row in stats['pooled_pairs']:
        selected = {c['role']: c for c in stats['pooled'] if
            (c['concurrency'], c['workload']) == (row['concurrency'], row['workload'])}
        assert set(selected) == {'old', 'new', 'redis'}
        assert row['comparison'] == compare(selected['old'], selected['new'])
        assert row['redis_throughput_ratio'] == selected['redis']['successful_calls_per_second'] / selected['new']['successful_calls_per_second']
        assert row['candidate_to_redis_mean_ratio'] == selected['new']['whole_call_latency']['mean_ns'] / selected['redis']['whole_call_latency']['mean_ns']
    for key in ['calls', 'input_items', 'issued', 'attempts', 'dropped_slots']:
        assert stats['totals'][key] == sum(c[key] for c in stats['cases'])
    for key in ['outcomes', 'reasons', 'attempt_reasons']:
        assert stats['totals'][key] == add(c[key] for c in stats['cases'])
    for name, totals in stats['phase_totals'].items():
        selected = [c['phases'][name] for c in stats['cases']]
        for key in ['calls', 'input_items', 'attempts', 'extra_attempts']:
            assert totals[key] == sum(c[key] for c in selected)
        for key in ['outcomes', 'reasons', 'attempt_reasons']:
            assert totals[key] == add(c[key] for c in selected)
    for key, value in stats['checks'].items():
        assert audit[key] == value
    print(json.dumps(dict(complete=True, verification_scope='Bundled byte identity and statistics quantities; not runtime acceptance',
        copies=len(originals), original_bytes_checked=args.check_originals, cases=72, pooled_rows=36,
        repeat_comparisons=24, pooled_comparisons=12, totals=stats['totals'],
        index_sha256=sha(index_bytes)), sort_keys=True))


if __name__ == '__main__':
    main()
