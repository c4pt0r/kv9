#!/usr/bin/env python3
"""Verify portable bytes and recompute pooled latency/rates from original reports."""
import gzip
import hashlib
import json
import math
from pathlib import Path
import tarfile

HERE = Path(__file__).resolve().parent


def require(ok, message):
    if not ok:
        raise ValueError(message)


def main():
    manifest = json.loads((HERE / 'manifest.json').read_text())
    for name, pin in manifest['files'].items():
        require(Path(name).name == name, 'unsafe package name')
        path = HERE / name
        require(path.is_file() and not path.is_symlink(), 'nonordinary package file')
        with path.open('rb') as stream:
            sha = hashlib.file_digest(stream, 'sha256').hexdigest()
        require(path.stat().st_size == pin['bytes'] and sha == pin['sha256'],
                'package bytes differ: ' + name)
    inventory = json.loads((HERE / 'input-inventory.json').read_text())
    expected = {row['member']: row for row in inventory}
    require(len(expected) == len(inventory) == manifest['members'], 'member inventory differs')
    seen, decoded, retained = set(), 0, {}
    with gzip.open(HERE / 'original-evidence.tar.gz', 'rb') as stream:
        with tarfile.open(fileobj=stream, mode='r|') as archive:
            for member in archive:
                require(member.isfile() and member.name in expected and member.name not in seen,
                        'unexpected archive member')
                row = expected[member.name]
                require(member.size == row['bytes'], 'member size differs')
                keep = member.name.endswith('/run/report.json') or member.name == 'audit/audit.json'
                sha, count, chunks = hashlib.sha256(), 0, []
                with archive.extractfile(member) as contents:
                    while block := contents.read(1024 * 1024):
                        sha.update(block)
                        count += len(block)
                        if keep:
                            chunks.append(block)
                require(count == row['bytes'] and sha.hexdigest() == row['sha256'],
                        'member bytes differ: ' + member.name)
                if keep:
                    retained[member.name] = json.loads(b''.join(chunks))
                seen.add(member.name)
                decoded += count
        while stream.read(1024 * 1024):
            pass
    require(seen == set(expected) and decoded == manifest['decoded_bytes'],
            'incomplete decoded archive')
    audit = retained['audit/audit.json']
    require(audit['complete'] and audit['matched_diagnostic_accepted']
            and audit['all_success_single_attempt'] and len(audit['cases']) == 16,
            'original audit is not accepted')
    require(len(retained) == 25, 'all 24 original reports are required')
    summary = json.loads((HERE / 'performance-summary.json').read_text())
    require(summary['complete'] and summary['audit_sha256'] == expected['audit/audit.json']['sha256'],
            'summary audit binding differs')
    for row in summary['rows']:
        for arm in ('old', 'new'):
            cases = [c for c in audit['cases'] if c['concurrency'] == row['concurrency']
                     and c['workload'] == row['workload'] and c['arm'] == arm+'-'+row['workload']]
            require(len(cases) == 2 and {c['repeat'] for c in cases} == {0, 1}, 'pool population differs')
            buckets, count, total, elapsed, items = [0]*3776, 0, 0, 0, 0
            for case in cases:
                name = 'timing/'+Path(case['directory']).name+'/run/report.json'
                report = retained[name]
                require(expected[name]['sha256'] == case['report_sha256']
                        and report['measured_completed'] == case['calls'], 'original report binding differs')
                elapsed += report['cohort_elapsed_ns']
                items += case['input_items']
                for op in report['metrics']['measurement']['statistics']:
                    for pop in op['populations']:
                        raw = pop['whole_call']['raw']
                        count += raw['count']
                        total += raw['sum_ns']
                        for i, n in enumerate(raw['buckets']):
                            buckets[i] += n
            require(sum(buckets) == count == sum(c['calls'] for c in cases), 'histogram count differs')
            actual = row[arm]
            for key, value in [('calls_per_second', count*1e9/elapsed),
                               ('items_per_second', items*1e9/elapsed), ('mean_us', total/count/1000)]:
                require(math.isclose(actual[key], value, rel_tol=1e-12), 'pooled statistic differs: '+key)
            rank, cumulative = (count*99+99)//100, 0
            for i, n in enumerate(buckets):
                cumulative += n
                if cumulative >= rank:
                    shift = (i-64)//64 if i >= 64 else 0
                    lower = (64+(i-64)%64) << shift if i >= 64 else i
                    upper = lower+(1 << shift)-1
                    require(actual['latency']['p99'] == dict(lower_ns=lower, upper_ns=upper),
                            'merged p99 differs')
                    require(actual['p99_us'] == dict(lower=lower/1000, upper=upper/1000),
                            'p99 unit conversion differs')
                    break
    require(summary['calls'] == sum(c['calls'] for c in audit['cases'])
            and summary['input_items'] == sum(c['input_items'] for c in audit['cases']), 'total counts differ')
    print(json.dumps(dict(complete=True, members=len(seen), decoded_bytes=decoded,
                          original_reports=24, timed_calls=summary['calls'], pooled_cells=4)))


if __name__ == '__main__':
    main()
