#!/usr/bin/env python3
"""Audit actual retained-PVC migration, exact recovery receipts and both full histories."""
import datetime
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import uuid

from workload_report import bounded, require, strict_json, validate
from chaos_client import refused
from checker import coverage, load, verify_witness

PHASES = ('endpoint-migration-pending', 'endpoint-migration-recovered')
BUNDLE = ('kv9-root-descriptor', 'kv9-store-identity', 'kv9-store-lifecycle')


def audit_cell(root, overrides=None):
    sources, scene = {}, root / 'endpoint-migration'

    def read(path):
        name = str(path.relative_to(root))
        data = (overrides or {}).get(name)
        if data is None:
            data = bounded(path, 2 * 1024 * 1024)
        sources[name] = hashlib.sha256(data).hexdigest()
        return data

    def obj(path):
        return strict_json(read(path))

    def fields(path):
        pairs = [line.split('=', 1) for line in read(path).decode().splitlines()]
        require(all(len(pair) == 2 for pair in pairs), 'malformed migration status or receipt')
        value = dict(pairs)
        require(len(value) == len(pairs), 'duplicate migration status or receipt field')
        return value

    def when(path):
        at = datetime.datetime.fromisoformat(read(path).decode().strip())
        require(at.tzinfo is not None, 'migration timestamp lacks a timezone')
        return at

    stages = ('before', 'held', 'unconfirmed', 'pending', 'confirmed')
    pods = {stage: obj(scene / stage / 'pod.json') for stage in stages}
    pvc = obj(scene / 'before/pvc.json')
    namespace = pods['before']['metadata']['namespace']
    require(len({pods[s]['metadata']['uid'] for s in ('before', 'held', 'unconfirmed')}) == 3 and
            pods['confirmed']['metadata']['uid'] == pods['pending']['metadata']['uid'] == pods['unconfirmed']['metadata']['uid'],
            'migration did not replace the killed owner and retain the configured successor')
    require(pvc['metadata']['namespace'] == namespace and pvc['metadata']['name'] == 'kv9-data-n4',
            'migration PVC belongs to another member')
    uuid.UUID(pvc['metadata']['uid'])
    before = fields(scene / 'before/status.txt')
    lifecycle = read(scene / 'before/kv9-store-lifecycle')
    require(len(lifecycle) == 97 and lifecycle[:8] == b'KV9LIFE2'
            and hashlib.sha256(lifecycle[:65]).digest() == lifecycle[65:]
            and int.from_bytes(lifecycle[8:16], 'big') == 4 and lifecycle[32] == 2
            and lifecycle[16:32].hex() == before['store_incarnation'] and lifecycle[33:65].hex() == before['root_digest'],
            'migration lacks the original Active lifecycle certificate')
    initial = fields(scene / 'endpoint-before.out')
    require(initial['node_id'] == '4' and initial['active'] == initial['found'] == 'true'
            and initial['generation'] == '0' and initial['address'] == pods['before']['status']['podIP'] + ':20160',
            'migration did not begin with the admitted Pod endpoint')
    require(initial['cluster_id'] == before['cluster_id'] and initial['store_incarnation'] == before['store_incarnation'],
            'operator read differs from the original owner identity')
    require(read(scene / 'owner-release.prepare').decode().strip() ==
            f"store_prepared=true node_id=4 store_incarnation={initial['store_incarnation']}",
            'old owner did not release the same store lock')
    times = {stage: when(scene / stage / 'at.txt') for stage in stages}
    require(all(times[a] < times[b] for a, b in zip(stages, stages[1:])), 'migration observations are reordered')
    for stage, pod in pods.items():
        uuid.UUID(pod['metadata']['uid'])
        require(pod['metadata']['namespace'] == namespace and pod['metadata']['labels']['app'] == 'kv9'
                and pod['metadata']['labels']['kv9-node'] == '4', 'migration Pod has the wrong identity')
        claim = obj(scene / stage / 'pvc.json')
        require(claim['metadata']['uid'] == pvc['metadata']['uid'] and claim['spec']['volumeName'] == pvc['spec']['volumeName']
                and any(v.get('persistentVolumeClaim', {}).get('claimName') == pvc['metadata']['name']
                        for v in pod['spec']['volumes']), 'migration replaced the original PVC')
        for name in BUNDLE:
            require(read(scene / stage / name) == read(scene / 'before' / name), 'migration changed durable root/store authority')
        if stage == 'held':
            continue
        status = fields(scene / stage / 'status.txt')
        require(all(status[k] == before[k] for k in ('cluster_id', 'root_digest', 'store_incarnation', 'node_id', 'bootstrap_generation')),
                'migration status changed immutable identity')
        require(status['fatal'] == '' and status['raft_receive_authorized'] == status['raft_owner_started'] == 'true',
                'migration lost durable Raft authority')
        require(status['meta_voters'] == '1,2,3' and status['meta_learners'] == '4', 'endpoint migration changed Raft membership')
        require(read(scene / stage / 'proc-stat.txt').decode().split()[0] == status['pid'], 'migration status names another process')
        command = read(scene / stage / 'cmdline').split(b'\0')
        require(command[0] == b'/usr/local/bin/kv9' and b'start' in command and b'--node-id' in command,
                'migration status was not produced by a real node owner')
        if stage in ('unconfirmed', 'pending'):
            require(status['bootstrap_state'] == 'Joining' and status['endpoint_ready'] == 'false',
                    'unauthorized changed endpoint entered Serving')
        else:
            require(status['bootstrap_state'] == 'Serving' and status['endpoint_ready'] == 'true',
                    'authorized endpoint did not serve')
    migrated = fields(scene / 'confirmed/status.txt')
    mutation, duplicate = fields(scene / 'mutation.out'), fields(scene / 'duplicate.out')
    service = obj(scene / 'migrated-service.json')
    target = service['spec']['clusterIP'] + ':20160'
    require(service['metadata']['namespace'] == namespace and service['spec']['selector'] == {'app': 'kv9', 'kv9-node': '4'}
            and service['spec']['ports'][0]['port'] == service['spec']['ports'][0]['targetPort'] == 20160,
            'migrated Service does not route to the retained owner')
    require(target != initial['address'] and pods['unconfirmed']['status']['podIP'] + ':20160' != initial['address'],
            'migration reused the obsolete address')
    for receipt in (mutation, duplicate):
        require(receipt['node_id'] == '4' and receipt['store_incarnation'] == initial['store_incarnation']
                and receipt['active'] == 'true' and receipt['generation'] == '1'
                and receipt['address'] == target and receipt['previous_address'] == initial['address'],
                'endpoint receipt describes another transition')
    require(mutation['endpoint_outcome'] == 'changed' and duplicate['endpoint_outcome'] == 'confirmed'
            and int(duplicate['confirmation_term']) > 0 and int(mutation['mutation_term']) > 0
            and int(duplicate['confirmation_index']) > int(mutation['mutation_index']) > 0,
            'duplicate endpoint CAS reused its mutation receipt')
    require(int(migrated['endpoint_confirmation_term']) > 0
            and int(migrated['driver_applied_index']) >= int(migrated['endpoint_confirmation_index']) > int(mutation['mutation_index']),
            'migrated owner did not apply a fresh exact confirmation')
    record = read(scene / 'confirmed/kv9-serving-endpoint')
    require(40 < len(record) <= 4096 and record[:8] == b'KV9ENDP2'
            and hashlib.sha256(record[:-32]).digest() == record[-32:], 'invalid durable serving record checksum')
    saved = strict_json(record[8:-32])
    require(saved == dict(node=4, incarnation=initial['store_incarnation'], root=before['root_digest'], address=target,
                          generation=1, term=int(migrated['endpoint_confirmation_term']), index=int(migrated['endpoint_confirmation_index'])),
            'saved serving authority differs from the exact applied confirmation')
    require(migrated['registration_attempts'] == '0' and migrated['registration_receipt_index'] == 'none',
            'migration reused a join ticket instead of endpoint confirmation')
    for name in ('kill.injected.json', 'kill.after-history.json'):
        fault = obj(scene / name)
        require(fault['kind'] == 'PodChaos' and fault['metadata']['name'] == 'endpoint-migration-kill'
                and fault['metadata']['namespace'] == namespace and fault['spec']['action'] == 'pod-kill'
                and fault['spec']['selector']['namespaces'] == [namespace]
                and fault['spec']['selector']['pods'] == {namespace: [pods['before']['metadata']['name']]},
                'migration fault did not select the original owner')
        require(any(c['type'] == 'AllInjected' and c['status'] == 'True' for c in fault['status']['conditions'])
                and any(r['id'] == namespace + '/' + pods['before']['metadata']['name'] and r['phase'] == 'Injected'
                        and r['injectedCount'] > 0 and any(e['operation'] == 'Apply' and e['type'] == 'Succeeded' for e in r['events'])
                        for r in fault['status']['experiment']['containerRecords']), 'migration fault lacks an actual injected owner death')
    for phase, prefix in zip(PHASES, ('pending', 'recovered')):
        first, last = [when(scene / f'{prefix}-{side}-history-probed-at.txt') for side in ('before', 'after')]
        for side in ('before', 'after'):
            require(read(scene / f'{prefix}-{side}-history-old-probe.exit').strip() in (b'1', b'124')
                    and read(scene / f'{prefix}-{side}-history-new-probe.exit').strip() == b'0',
                    'obsolete endpoint remained reachable during migration history')
        for kind in ('history', 'persistent'):
            require(first < when(root / f'{phase}-{kind}-observed-at.txt') < last,
                    'client progress lies outside the observed endpoint failure')
        require(times['unconfirmed'] < first and (last < times['pending'] if prefix == 'pending' else first > times['confirmed']),
                'history phase does not match endpoint recovery state')
    require(refused(subprocess.CompletedProcess([], int(read(scene / 'new-endpoint-get.exit')),
        read(scene / 'new-endpoint-get.out').decode(), read(scene / 'new-endpoint-get.err').decode())),
        'new endpoint did not deliver the real learner refusal')
    return dict(accepted=True, namespace=namespace, node=4, original_address=initial['address'], new_address=target,
                original_incarnation=initial['store_incarnation'], mutation_index=int(mutation['mutation_index']),
                confirmation_index=int(migrated['endpoint_confirmation_index']), sources=sources)


def main():
    root = Path(sys.argv[1])
    result = audit_cell(root)
    history = load(root / 'history.jsonl')
    checked = strict_json(bounded(root / 'history-checker.json', 16 * 1024 * 1024))
    require(checked['verdict'] == 'valid' and verify_witness(history, checked['witness']) and coverage(history) == checked['coverage'],
            'migration CLI history is incomplete or invalid')
    persistent = validate(root / 'persistent-run', root / 'persistent-build', seconds=60)
    require(persistent['full_history_independently_checked'], 'migration persistent history was not checked in full')
    for phase in PHASES:
        counts = checked['coverage']['successful_by_phase'][phase]
        require(counts.get('put') and (counts.get('get') or counts.get('scan')), 'migration interval lacks complete client progress')
    scene = root / 'endpoint-migration'
    fault = strict_json((scene / 'kill.injected.json').read_bytes())
    fault['status']['experiment']['containerRecords'] = []
    cases = [
        ('missing-owner-death', 'kill.injected.json', json.dumps(fault).encode(), 'migration fault lacks an actual injected owner death'),
        ('old-endpoint-live', 'pending-after-history-old-probe.exit', b'0\n', 'obsolete endpoint remained reachable during migration history'),
        ('changed-store', 'confirmed/kv9-store-identity', b'foreign-store', 'migration changed durable root/store authority'),
        ('serving-record-corrupt', 'confirmed/kv9-serving-endpoint', b'corrupt', 'invalid durable serving record checksum'),
        ('unauthorized-serving', 'unconfirmed/status.txt', (scene / 'unconfirmed/status.txt').read_bytes().replace(b'endpoint_ready=false', b'endpoint_ready=true'),
         'unauthorized changed endpoint entered Serving'),
    ]
    controls = []
    for name, suffix, mutant, reason in cases:
        relative = 'endpoint-migration/' + suffix
        require(mutant != (root / relative).read_bytes(), 'missing migration control anchor')
        audit_cell(root)
        try:
            audit_cell(root, {relative: mutant})
        except ValueError as error:
            require(str(error) == reason, f'{name}: wrong corrupted-evidence rejection: {error}')
        else:
            raise ValueError(f'{name}: corrupted migration evidence accepted')
        audit_cell(root)
        controls.append(dict(name=name, source=relative, mutant_sha256=hashlib.sha256(mutant).hexdigest(), reason=reason))
    result.update(controls=controls, complete_cli_history=True, complete_persistent_history=True)
    (root / 'endpoint-migration-audit.json').write_text(json.dumps(result, indent=2) + '\n')
    print('PASS: actual retained-PVC endpoint migration and both complete histories; five invalid evidence controls rejected')


if __name__ == '__main__':
    try:
        main()
    except (ValueError, KeyError, OSError, IndexError) as error:
        raise SystemExit(f'FAIL: {error}') from error
