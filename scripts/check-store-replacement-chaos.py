#!/usr/bin/env python3
"""Audit fresh-PVC incarnation refusal and recovery of each original voter PVC."""
import datetime
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
import uuid

sys.path.insert(0, str(Path(__file__).resolve().parent / 'history'))
from checker import coverage, load, verify_witness
from chaos_client import refused

PHASES = [f'store-loss-voter-{node}-pvc-replacement' for node in (1, 2, 3)]
BUNDLE = ('kv9-root-descriptor', 'kv9-store-identity')


def require(value, message):
    if not value:
        raise ValueError(message)


def lifecycle(data):
    require(len(data) == 97 and data[:8] == b'KV9LIFE2' and
            hashlib.sha256(data[:65]).digest() == data[65:], 'invalid lifecycle framing or checksum')
    return dict(node=int.from_bytes(data[8:16], 'big'), incarnation=data[16:32].hex(),
                phase=data[32], root=data[33:65].hex())


def audit_cell(root, phase, overrides=None):
    require(phase in PHASES, 'unknown replacement-PVC cell')
    victim = PHASES.index(phase) + 1
    scene, sources = root / phase, {}

    def read(path):
        name = str(path.relative_to(root))
        data = (overrides or {}).get(name)
        if data is None:
            data = path.read_bytes()
        sources[name] = hashlib.sha256(data).hexdigest()
        return data

    def obj(path):
        return json.loads(read(path))

    def fields(path):
        pairs = [line.split('=', 1) for line in read(path).decode().splitlines()]
        require(all(len(pair) == 2 for pair in pairs), 'malformed status or receipt')
        result = dict(pairs)
        require(len(result) == len(pairs), 'duplicate status or receipt field')
        return result

    def when(path):
        value = datetime.datetime.fromisoformat(read(path).decode().strip())
        require(value.tzinfo is not None, 'observation lacks an absolute timestamp')
        return value

    stages = ('before', 'held', 'replacement-held', 'rejected-1', 'rejected-2', 'rejected-during-history', 'healed')
    pods = {stage: obj(scene / stage / 'pod.json') for stage in stages}
    old, held, replacement, healed = [pods[s] for s in ('before', 'held', 'replacement-held', 'healed')]
    namespace = old['metadata']['namespace']
    require(len({p['metadata']['uid'] for p in (old, held, replacement, healed)}) == 4,
            'original, held, replacement and restored Pods are not distinct')
    original_pvc = obj(scene / 'before/pvc.json')
    new_pvc = obj(scene / 'prepared-pvc.json')
    require(original_pvc['metadata']['uid'] != new_pvc['metadata']['uid'] and
            original_pvc['spec']['volumeName'] != new_pvc['spec']['volumeName'], 'replacement reused the original PVC or PV')
    for pvc, name in ((original_pvc, f'kv9-data-n{victim}'), (new_pvc, f'kv9-replacement-n{victim}')):
        uuid.UUID(pvc['metadata']['uid'])
        require(pvc['metadata']['namespace'] == namespace and pvc['metadata']['name'] == name,
                'PVC belongs to another voter or namespace')
    original_pv, new_pv = [obj(scene / f'{name}-pv.json') for name in ('original', 'replacement')]
    require(original_pv['metadata']['uid'] != new_pv['metadata']['uid'] and
            original_pv['spec']['hostPath']['path'] != new_pv['spec']['hostPath']['path'], 'replacement reused the original physical volume path')
    for pv, pvc in ((original_pv, original_pvc), (new_pv, new_pvc)):
        uuid.UUID(pv['metadata']['uid'])
        require(pv['metadata']['name'] == pvc['spec']['volumeName'] and
                all(pv['spec']['claimRef'][key] == pvc['metadata'][key] for key in ('name', 'namespace', 'uid')),
                'PVC/PV binding does not match')
    before_status = fields(scene / 'before/status.txt')
    original_record = read(scene / 'before/lifecycle')
    decoded = lifecycle(original_record)
    require(decoded == dict(node=victim, incarnation=before_status['store_incarnation'], phase=2,
                            root=before_status['root_digest']), 'original store was not Active under its retained root')
    prepared = read(scene / 'prepared.lifecycle')
    new_record = lifecycle(prepared)
    require(new_record['node'] == victim and new_record['phase'] == 0 and new_record['root'] == '0' * 64 and
            new_record['incarnation'] not in ('0' * 32, decoded['incarnation']), 'replacement was not independently Prepared')
    require(read(scene / 'replacement.prepare').decode().strip() ==
            f"store_prepared=true node_id={victim} store_incarnation={new_record['incarnation']}", 'prepared incarnation differs from its real CLI receipt')
    require(read(scene / 'owner-release.prepare').decode().strip() ==
            f"store_prepared=true node_id={victim} store_incarnation={decoded['incarnation']}", 'old owner release did not retain its incarnation')
    times = {s: when(scene / s / 'at.txt') for s in stages}
    require(all(times[a] < times[b] for a, b in zip(stages, stages[1:])) and
            times['before'] < when(scene / 'owner-release-at.txt') < times['held'], 'replacement observations are reordered')
    provisioning = obj(scene / 'preparation-pod.json')
    require(provisioning['metadata']['namespace'] == namespace and
            provisioning['metadata']['labels']['app'] == 'kv9-provisioning' and
            provisioning['spec']['nodeName'] == held['spec']['nodeName'], 'preparation ran outside the owned storage node')
    require({v['persistentVolumeClaim']['claimName'] for v in provisioning['spec']['volumes'] if 'persistentVolumeClaim' in v} ==
            {original_pvc['metadata']['name'], new_pvc['metadata']['name']}, 'preparation did not mount both recorded PVCs')
    for file in BUNDLE:
        require(read(scene / 'before' / file) and read(scene / 'forged' / file) == read(scene / 'before' / file),
                'replacement did not receive the exact original identity bundle')
    require(read(scene / 'forged/kv9-store-lifecycle') == prepared, 'bundle copy overwrote the prepared lifecycle')
    require(read(scene / 'init.exit').decode().strip() == '1' and
            b'config error: prepared store identity does not match root/store identity' in read(scene / 'init.err') and
            not read(scene / 'init.out') and b'panicked' not in read(scene / 'init.err') and
            read(scene / 'init-side-effects.txt').decode().strip() == 'refused-without-bundle-or-raft', 'initialization did not refuse the new incarnation before side effects')
    rejected_pids = {}
    for stage in stages:
        pod = pods[stage]
        uuid.UUID(pod['metadata']['uid'])
        require(pod['metadata']['namespace'] == namespace and pod['metadata']['labels']['app'] == 'kv9' and
                pod['metadata']['labels']['kv9-node'] == str(victim), 'Pod identity differs from the selected voter')
        is_new = stage not in ('before', 'held', 'healed')
        pvc = obj(scene / stage / 'pvc.json')
        expected = new_pvc if is_new else original_pvc
        require(pvc['metadata']['uid'] == expected['metadata']['uid'] and
                pvc['spec']['volumeName'] == expected['spec']['volumeName'] and
                any(v['name'] == 'data' and v.get('persistentVolumeClaim', {}).get('claimName') == expected['metadata']['name']
                    for v in pod['spec']['volumes']), 'runtime did not mount the intended PVC')
        require(read(scene / stage / 'kv9-store-lifecycle') == (prepared if is_new else original_record), 'runtime changed the wrong lifecycle identity')
        for file in BUNDLE:
            require(read(scene / stage / file) == read(scene / 'before' / file), 'runtime identity bundle changed')
        if is_new:
            require(pod['metadata']['uid'] == replacement['metadata']['uid'], 'replacement process attempts changed Pod identity')
            require(read(scene / stage / 'stopped.txt') == b'raft=absent\nstatus=absent\nlistener=absent\n', 'replacement opened Raft or published runtime state')
            if stage.startswith('rejected-'):
                pid = read(scene / stage / 'pid.txt').decode().strip()
                require(re.fullmatch('[1-9][0-9]*', pid), 'missing rejected child PID')
                rejected_pids[stage] = pid
                require(read(scene / stage / 'exit.txt').decode().strip() == '1', 'replacement startup did not refuse')
                log = read(scene / stage / 'process.log')
                require(b'config error: prepared store identity does not match root/store identity' in log and b'panicked' not in log,
                        'missing typed incarnation refusal')
        elif stage in ('before', 'healed'):
            s = fields(scene / stage / 'status.txt')
            require(s['pid'] == read(scene / stage / 'pid.txt').decode().strip() and
                    s['bootstrap_state'] == 'Serving' and s['fatal'] == '' and
                    s['raft_owner_started'] == s['raft_receive_authorized'] == 'true' and
                    read(scene / stage / 'local-probe.txt').decode().strip() == 'reachable', 'original store did not serve')
            for key in ('node_id', 'cluster_id', 'root_digest', 'bootstrap_generation', 'store_incarnation'):
                require(s[key] and s[key] == before_status[key], 'restored original store changed identity')
        else:
            require(read(scene / stage / 'local-probe.txt').decode().strip() == 'stopped-without-listener', 'original held Pod still served')
    require(rejected_pids['rejected-1'] != rejected_pids['rejected-2'] and
            rejected_pids['rejected-2'] == rejected_pids['rejected-during-history'], 'replacement did not refuse two distinct fresh starts')
    fault = obj(scene / 'podchaos.json')
    require(fault['kind'] == 'PodChaos' and fault['metadata']['namespace'] == namespace and
            fault['metadata']['name'] == 'store-loss-kill' and not fault['metadata'].get('deletionTimestamp') and
            fault['spec']['action'] == 'pod-kill' and fault['spec']['mode'] == 'one' and
            fault['spec']['selector']['namespaces'] == [namespace] and
            fault['spec']['selector']['pods'] == {namespace: [old['metadata']['name']]} and
            any(c['type'] == 'AllInjected' and c['status'] == 'True' for c in fault['status']['conditions']), 'incorrect replacement owner-death fault')
    require(any(r['id'] == namespace + '/' + old['metadata']['name'] and r['phase'] == 'Injected' and r['injectedCount'] > 0 and
                any(e['operation'] == 'Apply' and e['type'] == 'Succeeded' for e in r['events'])
                for r in fault['status']['experiment']['containerRecords']), 'fault lacks the original replacement victim')
    receipt = fields(root / f'{phase}-majority-put.out')
    recovered = fields(scene / 'healed/status.txt')
    require(int(receipt['applied_term']) > 0 and int(recovered['driver_applied_index']) >= int(receipt['applied_index']) > 0,
            'restored original PVC did not apply the majority receipt')
    for suffix in ('majority-get', 'recovered-get'):
        require(fields(root / f'{phase}-{suffix}.out')['value_hex'] == '6166746572', 'replacement interval lost the acknowledged value')
    for suffix in ('before-put', 'majority-put', 'majority-get', 'recovered-get'):
        name = phase + '-' + suffix
        attempts = [json.loads(line) for line in read(root / (name + '-attempts.jsonl')).splitlines()]
        require(attempts and attempts[-1]['outcome'] == 'success', 'missing successful probe')
        for i, a in enumerate(attempts):
            require(a['attempt'] == i + 1 and a['node'] in ({1, 2, 3} - {victim} if suffix.startswith('majority') else {1, 2, 3}) and
                    a['ended'] >= a['started'] and 0 < a['remaining'] <= 25, 'probe violated replica or deadline bounds')
            if i:
                require(a['remaining'] <= attempts[i - 1]['remaining'] and a['started'] >= attempts[i - 1]['ended'], 'probe regained its deadline budget')
            result = subprocess.CompletedProcess([], a['returncode'], a['stdout'], a['stderr'])
            require((a['outcome'] == 'refused' and refused(result)) if i + 1 < len(attempts) else
                    (result.returncode == 0 and result.stdout == read(root / (name + '.out')).decode()), 'probe retried an ambiguous result or changed its receipt')
    for kind in ('history', 'persistent'):
        require(times['rejected-2'] < when(root / f'{phase}-{kind}-observed-at.txt') < times['rejected-during-history'], 'client progress falls outside replacement refusal')
    return dict(phase=phase, victim=victim, namespace=namespace, cluster_id=recovered['cluster_id'], root_digest=recovered['root_digest'],
                original_incarnation=decoded['incarnation'], replacement_incarnation=new_record['incarnation'], sources=sources)


def audit(root):
    cells = [audit_cell(root, phase) for phase in PHASES]
    require(len({c['namespace'] for c in cells}) == len({c['cluster_id'] for c in cells}) == len({c['root_digest'] for c in cells}) == 1 and
            len({c[k] for c in cells for k in ('original_incarnation', 'replacement_incarnation')}) == 6, 'replacement cells do not cover six distinct stores under one root')
    result = json.loads((root / 'history-checker.json').read_text())
    history = load(root / 'history.jsonl')
    require(result['verdict'] == 'valid' and verify_witness(history, result['witness']) and coverage(history) == result['coverage'], 'complete replacement history is invalid')
    for phase in PHASES:
        counts = result['coverage']['successful_by_phase'][phase]
        require(counts.get('put') and (counts.get('get') or counts.get('scan')), 'replacement window lacks completed client work')
    return dict(verdict='accepted', cells=cells)


def controls(root):
    phase = PHASES[0]
    pvc = json.loads((root / phase / 'prepared-pvc.json').read_text())
    pvc['metadata']['uid'] = json.loads((root / phase / 'before/pvc.json').read_text())['metadata']['uid']
    fault = json.loads((root / phase / 'podchaos.json').read_text())
    fault['status']['experiment']['containerRecords'] = []
    cases = [
        ('reused-pvc', 'prepared-pvc.json', json.dumps(pvc).encode(), 'replacement reused the original PVC or PV'),
        ('copied-active-lifecycle', 'prepared.lifecycle', (root / phase / 'before/lifecycle').read_bytes(), 'replacement was not independently Prepared'),
        ('accepted-init', 'init.exit', b'0\n', 'initialization did not refuse the new incarnation before side effects'),
        ('accepted-start', 'rejected-1/exit.txt', b'0\n', 'replacement startup did not refuse'),
        ('missing-victim', 'podchaos.json', json.dumps(fault).encode(), 'fault lacks the original replacement victim'),
        ('wrong-bundle', 'forged/kv9-store-identity', b'not-the-original-bundle', 'replacement did not receive the exact original identity bundle'),
    ]
    result = []
    for name, suffix, data, expected in cases:
        path = phase + '/' + suffix
        require(data != (root / path).read_bytes(), 'invalid replacement evidence control anchor')
        audit_cell(root, phase)
        try:
            audit_cell(root, phase, {path: data})
        except ValueError as error:
            require(str(error) == expected, 'replacement evidence control failed outside its intended gate')
        else:
            raise ValueError('corrupted replacement evidence was accepted')
        audit_cell(root, phase)
        result.append(dict(name=name, path=path, expected_rejection=expected, mutant_sha256=hashlib.sha256(data).hexdigest()))
    return result


if __name__ == '__main__':
    try:
        root = Path(sys.argv[1])
        result = audit(root)
        result['controls'] = controls(root)
        (root / 'store-replacement-audit.json').write_text(json.dumps(result, indent=2) + '\n')
        print('PASS: all three voters refused independent replacement PVCs and recovered original stores; six invalid evidence controls rejected')
    except (ValueError, KeyError, OSError, IndexError) as error:
        raise SystemExit(f'FAIL: {error}') from error
