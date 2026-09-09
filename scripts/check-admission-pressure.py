#!/usr/bin/env python3
"""Audit the observed admission load, limits, actual fault and history overlap."""
import datetime
import json
from pathlib import Path
import sys


def fields(path):
    pairs = [line.split('=', 1) for line in path.read_text().splitlines()]
    assert len({p[0] for p in pairs}) == len(pairs), 'duplicate status field'
    return dict(pairs)


def timestamp(text):
    # Preserve nanoseconds when comparing the cross-process wall-clock evidence.
    head, fraction = text.strip().replace(',', '.').split('.', 1)
    digits = fraction[:9]
    assert len(digits) == 9 and digits.isdigit()
    seconds = datetime.datetime.fromisoformat(head + fraction[9:]).timestamp()
    return int(seconds) * 1_000_000_000 + int(digits)


def check(root):
    records = [json.loads(line) for line in (root / 'admission-pressure.jsonl').read_text().splitlines()]
    start, done = records[0], records[-1]
    assert start['type'] == 'start' and done['type'] == 'complete'
    assert start['concurrency'] == 64
    reads = records[1:-1]
    assert 1 < len(reads) < 100_000 and done['requests'] == len(reads)
    assert sorted(r['sequence'] for r in reads) == list(range(len(reads)))
    counts = dict(missing=0, request_count=0, request_too_large=0)
    for read in reads:
        assert read['type'] == 'read'
        assert start['time_ns'] <= read['start_ns'] <= read['end_ns'] <= done['time_ns']
        assert read['key_byte'] == 173 and read['key_bytes'] in [32, 2 * 1024 * 1024 + 1]
        assert read['encoded_bytes'] > read['key_bytes']
        counts[read['outcome']] += 1
        if read['sequence'] == 0:
            assert read['key_bytes'] == 2 * 1024 * 1024 + 1 and read['outcome'] == 'request_too_large'
        else:
            assert read['key_bytes'] == 32 and read['outcome'] in ['missing', 'request_count']
        assert read['code'] == (0 if read['outcome'] == 'missing' else 8)
    assert counts['request_too_large'] == 1
    assert done['count_refusals'] == counts['request_count'] > 0, 'actual count refusal evidence is required'
    assert done['missing'] == counts['missing'] > 0
    snapshots = [fields(root / f'admission-{phase}.status') for phase in ['before', 'during', 'after']]
    for state in snapshots:
        assert state['fatal'] == '' and state['bootstrap_state'] == 'Serving'
        assert state['role'] == 'leader' and state['node_id'] == state['leader_id']
        assert int(state['public_rpc_limit_requests']) == 8
        assert int(state['public_rpc_limit_encoded_bytes']) == 2 * 1024 * 1024
        assert 0 <= int(state['public_rpc_in_flight']) <= int(state['public_rpc_peak_requests']) <= 8, 'observed count bound exceeded'
        assert int(state['public_rpc_in_flight']) == int(state['public_rpc_running']) + int(state['public_rpc_queued'])
        assert 0 <= int(state['public_rpc_encoded_bytes']) <= int(state['public_rpc_peak_encoded_bytes']) <= 2 * 1024 * 1024
    before, _, after = snapshots
    assert before['node_id'] == after['node_id']
    assert int(after['public_rpc_in_flight']) == int(after['public_rpc_encoded_bytes']) == 0
    assert int(after['public_rpc_peak_requests']) == 8
    def counter(state, name):
        return int(dict(item.split('=') for item in state['public_rpc_raw_read'].split(','))[name])
    assert counter(after, 'refused_count') - counter(before, 'refused_count') >= counts['request_count']
    assert counter(after, 'request_too_large') - counter(before, 'request_too_large') == 1
    assert counter(after, 'completed') - counter(before, 'completed') >= counts['missing']
    ready = int((root / 'admission-ready.txt').read_text())
    observed = timestamp((root / 'public-admission-overload-history-observed-at.txt').read_text())
    assert start['time_ns'] < ready < observed < done['time_ns'], 'history must overlap armed pressure'
    assert any(r['outcome'] == 'request_count' and r['end_ns'] <= ready for r in reads)
    assert any(r['outcome'] == 'request_count' and r['start_ns'] > observed for r in reads)
    uids = set()
    effects = []
    for phase in ['before', 'after']:
        effect = fields(root / f'admission-{phase}-effect.txt')
        assert effect['connection'] == 'blocked' and effect['survivor'] != effect['isolated']
        effects.append(timestamp(effect['observed_at']))
        fault = json.loads((root / f'admission-{phase}-fault.json').read_text())
        assert fault['spec']['action'] == 'partition' and fault['spec']['direction'] == 'both'
        assert fault['spec']['selector']['labelSelectors']['kv9-node'] == effect['isolated']
        assert not fault['metadata'].get('deletionTimestamp')
        assert any(c['type'] == 'AllInjected' and c['status'] == 'True' for c in fault['status']['conditions']), 'fault was not injected'
        uids.add(fault['metadata']['uid'])
    assert len(uids) == 1 and effects[0] < start['time_ns'] < observed < effects[1] < done['time_ns']
    history = json.loads((root / 'public-admission-overload-history-progress.json').read_text())['public-admission-overload']
    assert history['put'] > 0 and (history.get('get', 0) + history.get('scan', 0)) > 0
    return dict(reads=len(reads), outcomes=counts, peak_requests=int(after['public_rpc_peak_requests']),
                peak_encoded_bytes=int(after['public_rpc_peak_encoded_bytes']), fault_uid=next(iter(uids)))


def controls(root):
    import shutil
    import tempfile
    cases = [('no-refusals', 'actual count refusal evidence is required'),
             ('exceeded-capacity', 'observed count bound exceeded'),
             ('unarmed-fault', 'fault was not injected'),
             ('nonoverlapping-history', 'history must overlap armed pressure')]
    for name, expected in cases:
        with tempfile.TemporaryDirectory(prefix='kv9-pressure-control.') as tmp:
            tree = Path(tmp)
            for pattern in ['admission-*', 'public-admission-overload-history-*']:
                for path in root.glob(pattern):
                    if path.is_file(): shutil.copy2(path, tree / path.name)
            if name == 'no-refusals':
                path = tree / 'admission-pressure.jsonl'
                data = [json.loads(line) for line in path.read_text().splitlines()]
                for row in data:
                    if row.get('outcome') == 'request_count': row.update(outcome='missing', code=0)
                data[-1]['missing'] += data[-1]['count_refusals']
                data[-1]['count_refusals'] = 0
                path.write_text(''.join(json.dumps(row) + '\n' for row in data))
            elif name == 'exceeded-capacity':
                path = tree / 'admission-after.status'
                path.write_text(path.read_text().replace('public_rpc_peak_requests=8', 'public_rpc_peak_requests=9'))
            elif name == 'unarmed-fault':
                path = tree / 'admission-after-fault.json'
                data = json.loads(path.read_text())
                for condition in data['status']['conditions']:
                    if condition['type'] == 'AllInjected': condition['status'] = 'False'
                path.write_text(json.dumps(data))
            else:
                path = tree / 'admission-ready.txt'
                path.write_text(str(timestamp((tree / 'public-admission-overload-history-observed-at.txt').read_text()) + 1))
            try:
                check(tree)
            except AssertionError as error:
                assert str(error) == expected, f'{name}: failed outside intended assertion: {error}'
            else:
                raise AssertionError(f'{name}: invalid pressure evidence was accepted')
    assert check(root)
    print('PASS: 4 invalid admission pressure controls rejected')


if __name__ == '__main__':
    root = Path(sys.argv[1])
    print(json.dumps(check(root), sort_keys=True))
    if sys.argv[2:] == ['--self-test']:
        controls(root)
    else:
        assert len(sys.argv) == 2, 'unknown checker arguments'
