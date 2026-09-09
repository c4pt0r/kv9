#!/usr/bin/env python3
"""Audit first-formation fault evidence and its subsequent accepted KV history."""
import hashlib
import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent / 'history'))
from checker import coverage, load, verify_witness


def require(condition, message):
    if not condition:
        raise ValueError(message)


def audit(root, overrides=None):
    scene = root / 'formation'
    sources = {}

    def read(path):
        data = (overrides or {}).get(str(path.relative_to(root)), path.read_bytes())
        sources[str(path.relative_to(root))] = hashlib.sha256(data).hexdigest()
        return data

    def obj(path):
        return json.loads(read(path))

    def status(path):
        pairs = [line.split('=', 1) for line in read(path).decode().splitlines()]
        require(all(len(pair) == 2 for pair in pairs), 'malformed status')
        fields = dict(pairs)
        require(len(fields) == len(pairs), 'duplicate status field')
        return fields

    pods = {phase: [obj(scene / phase / f'n{i}.pod.json') for i in (1, 2, 3)]
            for phase in ('isolated', 'replaced', 'recovered')}
    namespace = pods['isolated'][0]['metadata']['namespace']
    old_names = set()
    roots, clusters, incarnations = set(), set(), set()
    for i in (1, 2, 3):
        old, replaced, recovered = [pods[p][i - 1] for p in pods]
        require(len({p['metadata']['uid'] for p in (old, replaced, recovered)}) == 3,
                'each voter must run in three distinct Pod instances')
        for pod in (old, replaced, recovered):
            require(pod['metadata']['namespace'] == namespace and
                    pod['metadata']['labels']['kv9-node'] == str(i), 'Pod identity mismatch')
            volumes = pod['spec']['volumes']
            require(any(v.get('persistentVolumeClaim', {}).get('claimName') == f'kv9-data-n{i}'
                        for v in volumes), 'original PVC was not retained')
        old_names.add(namespace + '/' + old['metadata']['name'])
        require(read(scene / f'n{i}.old-uid').decode() == old['metadata']['uid'], 'victim UID mismatch')
        lifecycle = [read(scene / p / f'n{i}.lifecycle') for p in pods]
        require(lifecycle[0] and lifecycle[0] == lifecycle[1] == lifecycle[2],
                'durable lifecycle changed across formation recovery')
        before = status(scene / 'isolated' / f'n{i}.status')
        after = status(scene / 'recovered' / f'n{i}.status')
        require(before['bootstrap_state'] == 'Discovering' and before['raft_committed'] == '0'
                and before['cluster_id'] == '', 'catalog formed before the injected crash')
        require(after['bootstrap_state'] == 'Serving' and int(after['raft_committed']) > 0
                and after['cluster_id'], 'catalog formation did not resume')
        for fields in (before, after):
            require(fields['node_id'] == str(i) and fields['fatal'] == '' and
                    fields['raft_owner_started'] == fields['raft_receive_authorized'] == 'true',
                    'owner did not run with receive authority')
        for key in ('root_digest', 'bootstrap_generation', 'store_incarnation'):
            require(before[key] and before[key] == after[key], 'root/store identity changed')
        roots.add(after['root_digest'])
        clusters.add(after['cluster_id'])
        incarnations.add(after['store_incarnation'])
        for phase in pods:
            wanted = 'held-without-listener' if phase == 'replaced' else 'reachable'
            require(read(scene / phase / f'n{i}.local-probe').decode().strip() == wanted,
                    'missing live endpoint or replacement hold probe')
    require(len(roots) == len(clusters) == 1 and len(incarnations) == 3, 'inconsistent recovered root')
    expected_edges = {f'n{i} -> n{j} blocked' for i in (1, 2, 3) for j in (1, 2, 3) if i != j}
    require(read(scene / 'tcp-matrix.txt').decode().splitlines() and
            set(read(scene / 'tcp-matrix.txt').decode().splitlines()) == expected_edges,
            'missing actual all-peer partition effect')
    for filename, kind, action in [('networkchaos.json', 'NetworkChaos', 'partition'),
                                    ('podchaos.json', 'PodChaos', 'pod-kill')]:
        fault = obj(scene / filename)
        require(fault['kind'] == kind and fault['metadata']['namespace'] == namespace and
                fault['spec']['action'] == action and fault['spec']['mode'] == 'all',
                'incorrect formation fault')
        require(any(c['type'] == 'AllInjected' and c['status'] == 'True'
                    for c in fault['status']['conditions']), 'fault was not fully injected')
        victims = {r['id'] for r in fault['status']['experiment']['containerRecords']
                   if r['phase'] == 'Injected' and r['injectedCount'] > 0 and
                   any(e['operation'] == 'Apply' and e['type'] == 'Succeeded' for e in r['events'])}
        require(old_names <= victims, 'fault did not reach every original voter')
    history = obj(root / 'history-checker.json')
    require(history['verdict'] == 'valid', 'subsequent KV history was not accepted')
    operations = load(root / 'history.jsonl')
    require(verify_witness(operations, history['witness']) and history['coverage'] == coverage(operations),
            'subsequent KV history witness or coverage is invalid')
    successes = history['coverage']['successful']
    require(all(successes.get(k, 0) > 0 for k in ('put', 'get', 'delete', 'scan', 'delete_range', 'create_keyspace')),
            'incomplete subsequent client history')
    read(root / 'history.jsonl')
    return dict(verdict='accepted', namespace=namespace, original_victims=sorted(old_names),
                recovered_cluster=next(iter(clusters)), sources=sources)


def controls(root):
    reused = json.loads((root / 'formation/replaced/n1.pod.json').read_text())
    reused['metadata']['uid'] = json.loads((root / 'formation/isolated/n1.pod.json').read_text())['metadata']['uid']
    missing = json.loads((root / 'formation/podchaos.json').read_text())
    missing['status']['experiment']['containerRecords'] = []
    before = (root / 'formation/isolated/n1.status').read_text()
    cases = [
        ('stale-pod', 'formation/replaced/n1.pod.json', json.dumps(reused).encode(),
         'each voter must run in three distinct Pod instances'),
        ('missing-victims', 'formation/podchaos.json', json.dumps(missing).encode(),
         'fault did not reach every original voter'),
        ('already-formed', 'formation/isolated/n1.status', before.replace('raft_committed=0\n', 'raft_committed=1\n').encode(),
         'catalog formed before the injected crash'),
        ('unobserved-partition', 'formation/tcp-matrix.txt', b'n1 -> n2 blocked\n',
         'missing actual all-peer partition effect'),
    ]
    result = []
    for name, path, changed, expected in cases:
        require(changed != (root / path).read_bytes(), 'invalid evidence control anchor')
        audit(root)
        try:
            audit(root, {path: changed})
        except ValueError as error:
            require(str(error) == expected, 'evidence control failed outside its intended gate')
        else:
            raise ValueError('invalid formation evidence was accepted')
        audit(root)
        result.append(dict(name=name, path=path, expected_rejection=expected,
                           mutant_sha256=hashlib.sha256(changed).hexdigest()))
    return result


if __name__ == '__main__':
    try:
        root = Path(sys.argv[1])
        result = audit(root)
        result['controls'] = controls(root)
        (root / 'formation-audit.json').write_text(json.dumps(result, indent=2) + '\n')
        print('PASS: original-store formation recovered after all-voter Chaos Mesh faults with accepted KV history')
    except (ValueError, KeyError, OSError, IndexError) as error:
        raise SystemExit(f'FAIL: {error}') from error
