#!/usr/bin/env python3
"""Audit actual owner death, repeated missing-log refusal, and original-store recovery."""
import datetime
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent / 'history'))
from checker import coverage, load, verify_witness
from chaos_client import refused

PHASES = [f'store-loss-voter-{node}-log-missing' for node in (1, 2, 3)]


def require(value, message):
    if not value:
        raise ValueError(message)


def audit_cell(root, phase, overrides=None):
    require(phase in PHASES, 'unknown store-loss cell')
    victim = PHASES.index(phase) + 1
    sources = {}

    def read(path):
        relative = str(path.relative_to(root))
        data = (overrides or {}).get(relative)
        if data is None:
            data = path.read_bytes()
        sources[relative] = hashlib.sha256(data).hexdigest()
        return data

    def obj(path):
        return json.loads(read(path))

    def fields(path):
        pairs = [line.split('=', 1) for line in read(path).decode().splitlines()]
        require(all(len(pair) == 2 for pair in pairs), 'malformed status or receipt')
        result = dict(pairs)
        require(len(result) == len(pairs), 'duplicate status or receipt field')
        return result

    scene = root / phase
    stages = ('before', 'held', 'rejected-1', 'rejected-2', 'rejected-during-history', 'healed')
    pods = {s: obj(scene / s / 'pod.json') for s in stages}
    old, replacement = pods['before'], pods['held']
    namespace = old['metadata']['namespace']
    require(old['metadata']['uid'] != replacement['metadata']['uid'] and
            old['metadata']['name'] != replacement['metadata']['name'], 'original owner was not replaced')
    pvcs = {s: obj(scene / s / 'pvc.json') for s in stages}
    original_pvc = pvcs['before']
    times, statuses = {}, {}
    for stage in stages:
        pod, pvc = pods[stage], pvcs[stage]
        require(pod['metadata']['namespace'] == namespace and
                pod['metadata']['labels']['app'] == 'kv9' and
                pod['metadata']['labels']['kv9-node'] == str(victim), 'voter Pod identity mismatch')
        require(pvc['metadata']['namespace'] == namespace and
                pvc['metadata']['name'] == f'kv9-data-n{victim}' and
                pvc['metadata']['uid'] == original_pvc['metadata']['uid'] and
                pvc['spec']['volumeName'] == original_pvc['spec']['volumeName'], 'original PVC was not retained')
        require(any(v.get('persistentVolumeClaim', {}).get('claimName') == pvc['metadata']['name']
                    for v in pod['spec']['volumes']), 'Pod did not mount the original PVC')
        if stage != 'before':
            require(pod['metadata']['uid'] == replacement['metadata']['uid'], 'replacement Pod changed during the cell')
        lifecycle = read(scene / stage / 'lifecycle')
        require(lifecycle and lifecycle == read(scene / 'before/lifecycle'), 'original lifecycle changed')
        times[stage] = datetime.datetime.fromisoformat(read(scene / stage / 'at.txt').decode().strip())
        require(times[stage].tzinfo is not None, 'observation lacks an absolute timestamp')
        statuses[stage] = fields(scene / stage / 'status.txt')
        probe = read(scene / stage / 'local-probe.txt').decode().strip()
        require(probe == ('reachable' if stage in ('before', 'healed') else 'stopped-without-listener'),
                'missing live endpoint or stopped-process probe')
    require(all(times[a] < times[b] for a, b in zip(stages, stages[1:])), 'store-loss observations are reordered')
    fault = obj(scene / 'podchaos.json')
    require(fault['kind'] == 'PodChaos' and fault['metadata']['namespace'] == namespace and
            fault['metadata']['name'] == 'store-loss-kill' and not fault['metadata'].get('deletionTimestamp') and
            fault['spec']['action'] == 'pod-kill' and fault['spec']['mode'] == 'one' and
            fault['spec']['selector']['namespaces'] == [namespace] and
            fault['spec']['selector']['pods'] == {namespace: [old['metadata']['name']]}, 'incorrect owner-death fault')
    require(any(c['type'] == 'AllInjected' and c['status'] == 'True' for c in fault['status']['conditions']),
            'owner-death fault was not injected')
    victims = {r['id'] for r in fault['status']['experiment']['containerRecords']
               if r['phase'] == 'Injected' and r['injectedCount'] > 0 and
               any(e['operation'] == 'Apply' and e['type'] == 'Succeeded' for e in r['events'])}
    require(namespace + '/' + old['metadata']['name'] in victims, 'fault lacks the original victim')
    log = read(scene / 'original-raft.log')
    require(log, 'original durable log is empty')
    log_hash = hashlib.sha256(log).hexdigest()

    def check_hash(path, filename):
        require(read(path).decode().split() == [log_hash, filename], 'saved or restored log differs from the original')

    check_hash(scene / 'original-log.sha256', '/data/raft/raft.log')
    check_hash(scene / 'restored-log.sha256', '/data/raft/raft.log')
    pids = {}
    for stage in stages:
        if stage == 'held':
            continue
        pid = read(scene / stage / 'pid.txt').decode().strip()
        require(re.fullmatch('[1-9][0-9]*', pid), 'missing actual child PID')
        pids[stage] = pid
        if stage in ('before', 'healed'):
            s = statuses[stage]
            require(s['pid'] == pid and s['bootstrap_state'] == 'Serving' and s['fatal'] == '' and
                    s['raft_owner_started'] == s['raft_receive_authorized'] == 'true', 'original voter did not serve')
        else:
            require(read(scene / stage / 'exit.txt').decode().strip() == '1', 'startup did not refuse')
            output = read(scene / stage / 'process.log').decode()
            require('raft error: open /data/raft/raft.log:' in output and 'os error 2' in output and
                    'panicked' not in output, 'missing typed recovery-only refusal')
            require(read(scene / stage / 'raft-log.txt').decode().strip() == 'absent', 'rejected startup recreated the log')
            check_hash(scene / stage / 'saved-log.sha256', '/data/kv9-loss.saved-log')
            require(statuses[stage] == statuses['held'], 'stopped refusal reused fresh-looking runtime status')
    require(pids['rejected-1'] != pids['rejected-2'] and
            pids['rejected-2'] == pids['rejected-during-history'] and
            pids['healed'] not in (pids['rejected-1'], pids['rejected-2']), 'fresh start attempts are not distinct')
    before, healed = statuses['before'], statuses['healed']
    for key in ('node_id', 'cluster_id', 'root_digest', 'bootstrap_generation', 'store_incarnation'):
        require(before[key] and before[key] == healed[key], 'original root/store identity did not recover')
    require(before['node_id'] == str(victim), 'recovered wrong voter')
    receipt = fields(root / (phase + '-majority-put.out'))
    require(int(receipt['applied_term']) > 0 and
            int(healed['driver_applied_index']) >= int(receipt['applied_index']) > 0, 'original store did not apply the majority receipt')
    for suffix in ('majority-get', 'recovered-get'):
        require(fields(root / (phase + '-' + suffix + '.out'))['value_hex'] == '6166746572', 'acknowledged value did not survive')
    for suffix in ('before-put', 'majority-put', 'majority-get', 'recovered-get'):
        name = phase + '-' + suffix
        attempts = [json.loads(line) for line in read(root / (name + '-attempts.jsonl')).splitlines()]
        require(attempts and attempts[-1]['outcome'] == 'success', 'probe has no successful completion')
        for i, attempt in enumerate(attempts):
            require(attempt['attempt'] == i + 1 and attempt['node'] in ({1, 2, 3} - {victim} if suffix.startswith('majority') else {1, 2, 3}), 'probe targeted an invalid replica')
            require(attempt['ended'] >= attempt['started'] and 0 < attempt['remaining'] <= 25, 'invalid probe deadline')
            result = subprocess.CompletedProcess([], attempt['returncode'], attempt['stdout'], attempt['stderr'])
            if i + 1 < len(attempts):
                require(attempt['outcome'] == 'refused' and refused(result), 'probe retried an ambiguous result')
            else:
                require(result.returncode == 0 and result.stdout == read(root / (name + '.out')).decode(), 'probe receipt differs from its actual response')
    for kind in ('history', 'persistent'):
        observed = datetime.datetime.fromisoformat(read(root / f'{phase}-{kind}-observed-at.txt').decode().strip())
        require(times['rejected-2'] < observed < times['rejected-during-history'], 'client progress falls outside repeated-refusal window')
    return dict(phase=phase, victim=victim, namespace=namespace, cluster_id=healed['cluster_id'],
                root_digest=healed['root_digest'], store_incarnation=healed['store_incarnation'],
                log_sha256=log_hash, sources=sources)


def audit(root, overrides=None):
    cells = [audit_cell(root, phase, overrides) for phase in PHASES]
    require(len({c['namespace'] for c in cells}) == len({c['cluster_id'] for c in cells}) ==
            len({c['root_digest'] for c in cells}) == 1 and
            len({c['store_incarnation'] for c in cells}) == 3, 'cells do not cover one three-store root')
    result = json.loads((root / 'history-checker.json').read_text())
    history = load(root / 'history.jsonl')
    require(result['verdict'] == 'valid' and verify_witness(history, result['witness']) and
            coverage(history) == result['coverage'], 'complete client history is invalid')
    for phase in PHASES:
        counts = result['coverage']['successful_by_phase'][phase]
        require(counts.get('put') and (counts.get('get') or counts.get('scan')),
                'log-loss window lacks completed client work')
    return dict(verdict='accepted', cells=cells)


def controls(root):
    phase = PHASES[0]
    fault = json.loads((root / phase / 'podchaos.json').read_text())
    fault['status']['experiment']['containerRecords'] = []
    cases = [
        ('missing-victim', f'{phase}/podchaos.json', json.dumps(fault).encode(), 'fault lacks the original victim'),
        ('accepted-start', f'{phase}/rejected-1/exit.txt', b'0\n', 'startup did not refuse'),
        ('recreated-log', f'{phase}/rejected-1/raft-log.txt', b'present\n', 'rejected startup recreated the log'),
        ('stale-attempt', f'{phase}/rejected-2/pid.txt', (root / phase / 'rejected-1/pid.txt').read_bytes(), 'fresh start attempts are not distinct'),
        ('wrong-restored-log', f'{phase}/restored-log.sha256', b'0' * 64 + b'  /data/raft/raft.log\n', 'saved or restored log differs from the original'),
    ]
    result = []
    for name, path, data, expected in cases:
        require(data != (root / path).read_bytes(), 'invalid evidence control anchor')
        audit_cell(root, phase)
        try:
            audit_cell(root, phase, {path: data})
        except ValueError as error:
            require(str(error) == expected, 'evidence control failed outside its intended gate')
        else:
            raise ValueError('corrupted store-loss evidence was accepted')
        audit_cell(root, phase)
        result.append(dict(name=name, path=path, expected_rejection=expected,
                           mutant_sha256=hashlib.sha256(data).hexdigest()))
    return result


if __name__ == '__main__':
    try:
        root = Path(sys.argv[1])
        result = audit(root)
        result['controls'] = controls(root)
        (root / 'store-loss-audit.json').write_text(json.dumps(result, indent=2) + '\n')
        print('PASS: all three voters refused repeated missing-log starts after PodChaos and recovered; five invalid evidence controls rejected')
    except (ValueError, KeyError, OSError, IndexError) as error:
        raise SystemExit(f'FAIL: {error}') from error
