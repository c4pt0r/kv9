#!/usr/bin/env python3
"""Check retained bytes and read-stage arithmetic; does not run runtime gates."""
import hashlib
import io
import json
from pathlib import Path
import tarfile

ROOT = Path(__file__).resolve().parent


def require(condition, message):
    if not condition:
        raise ValueError(message)


def bound(data, record):
    require(len(data) == record['bytes'], 'byte length differs')
    require(hashlib.sha256(data).hexdigest() == record['sha256'], 'hash differs')


def main():
    index = json.loads((ROOT / 'inventory.json').read_text())
    files = {}
    for name, record in index['archives'].items():
        data = (ROOT / name).read_bytes()
        bound(data, record)
        with tarfile.open(fileobj=io.BytesIO(data), mode='r:gz') as tar:
            members = tar.getmembers()
            require(len(members) == len(record['files']), 'archive member count differs')
            require({m.name for m in members} == set(record['files']), 'archive membership differs')
            for member in members:
                require(member.isfile() and member.name not in files, 'non-file or duplicate member')
                content = tar.extractfile(member).read()
                bound(content, record['files'][member.name])
                files[member.name] = content

    def read(name):
        return json.loads(files[name])

    analysis = read('kv9-read-wait-lifecycle-run-preparation/results-first/analysis.json')
    summary = read('kv9-read-lifecycle-results-summary-first/summary.json')
    require(analysis['accepted'] and analysis['complete'], 'diagnostic incomplete')
    require(analysis['instrumented'] and not analysis['throughput_acceptance'], 'diagnostic scope differs')
    require(len(analysis['cohorts']) == len(summary['rows']) == 4, 'cohort count differs')
    phases = ('queue', 'quorum', 'apply', 'notification', 'total')

    def histogram(document, phase):
        metric, = [m for m in document['metrics'] if m['name'] == 'raft_async_read_profile_' + phase]
        result, = [h for h in metric['latency']['outcomes'] if h['outcome'] == 'success']
        return result

    for cell, row in zip(analysis['cohorts'], summary['rows']):
        arm = cell['descriptor']['arm']
        require(arm == row['arm'], 'row identity differs')
        path = f"kv9-read-wait-lifecycle-isolation-first/cohorts/{cell['index']:03d}-{arm}/"
        before, after = [read(path + side + '-metrics.json') for side in ('before', 'after')]
        sums = []
        for phase in phases:
            count = total = 0
            for node in ('1', '2', '3'):
                b, a = histogram(before[node], phase), histogram(after[node], phase)
                require(a['count'] >= b['count'] and a['sum_ns'] >= b['sum_ns'], 'counter regressed')
                count += a['count'] - b['count']
                total += a['sum_ns'] - b['sum_ns']
            expected = row['phases'][phase]
            require(count == expected['count_delta'] > 0 and total == expected['sum_ns_delta'], 'raw delta differs')
            require(total / count / 1000 == expected['mean_us'], 'published mean differs')
            sums.append(total)
        require(sum(sums[:-1]) == sums[-1], 'stage partition differs')
        print(arm, ', '.join(f"{p}={row['phases'][p]['mean_us']:.3f} us" for p in phases))

    for group in ('workspace', 'controls'):
        require(read('kv9-read-window-' + group + '-first/result.json')['complete'], 'source gate incomplete')
    controls = read('kv9-read-window-controls-first/controls/manifest.json')
    require(controls['accepted'] and len(controls['controls']) == 14, 'semantic controls incomplete')
    require(all(c['accepted'] and len(c['runs']) == 3 and all(r['validation_passed'] for r in c['runs'])
                for c in controls['controls']), 'control triple incomplete')
    for group, count in (('formal', 29), ('admission-formal', 37)):
        gate = read('kv9-read-window-' + group + '-first/summary.json')
        require(gate['accepted'] and len(gate['records']) == count, 'formal record incomplete')
    print(f'PASS: {len(files)} retained files, four arithmetic rows and local completion records; no runtime/proof rerun')


if __name__ == '__main__':
    main()
