#!/usr/bin/env python3
"""Real-process source-capture gate over a live cluster with real MinIO.

Covers the committed capture RPC end to end: leader-only capture at the exact
durable cut, pin-before-upload owners, canonical record framing, idempotent
re-capture of the same cut, refusal of a second image after the cut advances,
and follower/uncommitted refusals. Installation at a destination inside the
image configuration is proven by the component loop test; this gate asserts
the RPC surface and committed-state semantics only. No serving, learner or
transfer capability is exercised or claimed.

Requires KV9_OBJECT_STORE_* in the environment (an isolated real MinIO
bucket); servers additionally run with KV9_STORAGE=minio.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import time


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def fields(text):
    return dict(line.split('=', 1) for line in text.splitlines() if '=' in line)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bin', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--base-port', type=int, default=26880)
    args = parser.parse_args()
    for name in ('KV9_OBJECT_STORE_ENDPOINT', 'KV9_OBJECT_STORE_BUCKET',
                 'KV9_OBJECT_STORE_ACCESS_KEY', 'KV9_OBJECT_STORE_SECRET_KEY'):
        require(os.environ.get(name), f'{name} must be set to an isolated real MinIO bucket')
    binary, output = args.bin.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ, KV9_CLUSTER_TOKEN='capture-e2e-cluster',
               KV9_CLIENT_TOKENS='admin=capture-e2e-client', KV9_CLIENT_TOKEN='capture-e2e-client',
               KV9_BOOTSTRAP_TOKEN='capture-e2e-bootstrap', KV9_STORAGE='minio')
    processes, handles, commands = {}, [], []
    manifest_record = dict(verdict='running', commands=commands,
                           binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                           runner_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                           revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
                           dirty=bool(subprocess.check_output(['git', 'status', '--porcelain'])),
                           chaos_mesh=False, transfer_or_serving=False)
    (output / 'runner.py').write_bytes(Path(__file__).read_bytes())
    addresses = {i: f'127.0.0.1:{args.base_port+i}' for i in range(1, 5)}

    def command(label, *arguments, success=True, token=None, extra_env=None):
        started = time.time_ns()
        call_env = dict(env if token is None else dict(env, KV9_CLIENT_TOKEN=token), **(extra_env or {}))
        result = subprocess.run([str(binary), *map(str, arguments)], env=call_env, text=True,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=180)
        if success is None:
            label = f'{label}-probe-{len(commands)}'
        (output / f'{label}.log').write_text(result.stdout)
        commands.append(dict(label=label, arguments=list(map(str, arguments)), exit_code=result.returncode,
                             started_ns=started, finished_ns=time.time_ns()))
        if success is not None:
            require((result.returncode == 0) == success, f'{label}: unexpected exit; see {output}')
        return dict(dict(word.split('=', 1) for word in result.stdout.split() if '=' in word), exit_code=result.returncode)

    def start(node, label, ticket=None):
        log = (output / f'{label}-{node}.log').open('w')
        handles.append(log)
        run_env = env if ticket is None else dict(env, KV9_JOIN_TICKET=ticket)
        process = subprocess.Popen([str(binary), 'start', '--node-id', str(node), '--addr', addresses[node],
                                    '--data-dir', str(output / f'n{node}')], env=run_env, stdout=log, stderr=subprocess.STDOUT)
        processes[node] = process

    def stop(node):
        process = processes.pop(node)
        if process.poll() is None:
            process.kill()
        process.wait(timeout=10)

    def status(node):
        path = output / f'n{node}/status'
        if not path.exists() or node not in processes:
            return {}
        value = fields(path.read_text())
        return value if value.get('pid') == str(processes[node].pid) else {}

    def wait(label, predicate, seconds=60):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            value = predicate()
            if value:
                print(f'PASS: {label}', flush=True)
                return value
            time.sleep(.05)
        raise RuntimeError(f'timed out: {label}; see {output}')

    def leader():
        return next((n for n in processes if status(n).get('role') == 'leader'
                     and status(n).get('endpoint_ready') == 'true'), None)

    def data_groups(node):
        return {g['region']: g for g in json.loads(status(node).get('data_groups', '[]'))}

    try:
        guards, incarnations = [], []
        for node in (1, 2, 3):
            guard = socket.socket()
            guard.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            guard.bind(('127.0.0.1', args.base_port + node))
            guards.append(guard)
            prepared = command(f'prepare-{node}', 'store-prepare', '--node-id', node, '--data-dir', output / f'n{node}')
            incarnations.append(f"{node}={prepared['store_incarnation']}")
        root_file = output / 'root.bin'
        command('root-create', 'root-create', '--output', root_file,
                '--voters', ','.join(f'{n}@{addresses[n]}' for n in (1, 2, 3)),
                '--store-incarnations', ','.join(incarnations))
        for node in (1, 2, 3):
            command(f'init-{node}', 'init', '--root', root_file, '--node-id', node, '--data-dir', output / f'n{node}')
        for guard in guards:
            guard.close()
        for node in (1, 2, 3):
            start(node, 'initial')
        owner_node = wait('metadata leader', leader)
        wait('all endpoints ready', lambda: all(status(n).get('endpoint_ready') == 'true' for n in processes))
        root = status(owner_node)['root_digest']

        created = command('create-group', 'client', 'create-data-group', '--addr', addresses[owner_node],
                          '--root-digest', root, '--operation-id', f'{1:032x}', '--voters', '1,2,3')
        task, region = created['task_id'], int(created['region_id'])
        keyspace = command('create-keyspace', 'client', 'create-data-keyspace', '--addr', addresses[owner_node],
                           '--root-digest', root, '--creation-task', task, '--name', 'capture-src')
        keyspace_id = keyspace['keyspace_id']

        admitted = command('admit-node-4', 'client', 'admit-node', '--addr', addresses[owner_node],
                           '--node-id', 4, '--node-addr', addresses[4], '--ttl-seconds', 600)
        command('join-4', 'join', '--root', root_file, '--node-id', 4, '--data-dir', output / 'n4',
                extra_env={'KV9_JOIN_TICKET': admitted['join_ticket']})
        start(4, 'joiner', ticket=admitted['join_ticket'])
        wait('joiner registered and ready', lambda: status(4).get('endpoint_ready') == 'true')

        def group_leader():
            snapshots = {n: data_groups(n) for n in (1, 2, 3) if n in processes}
            return next((n for n, s in snapshots.items()
                         if s.get(region, {}).get('role') == 'Leader'), None)
        wait('data group elected and routed', lambda: group_leader() is not None)

        def routable():
            node = group_leader()
            if node is None:
                return False
            probe = command('route', 'client', 'raw-get', '--addr', addresses[node],
                            '--keyspace', keyspace_id, '--key-hex', '70726f6265', success=None)
            return probe['exit_code'] == 0
        wait('public route serves', routable, 120)
        data_node = group_leader()
        for n, (key, value) in enumerate([('616c706861', '6f6e65'), ('62657461', '74776f')]):
            command(f'put-{n}', 'client', 'raw-put', '--addr', addresses[data_node],
                    '--keyspace', keyspace_id, '--key-hex', key, '--value-hex', value)

        migrated = command('migrate', 'client', 'migrate-data-group', '--addr', addresses[owner_node],
                           '--root-digest', root, '--operation-id', f'{7:032x}',
                           '--creation-task', task, '--destination-node', 4)
        require(migrated['migration_outcome'] == 'requested', 'intent lacks a mutation receipt')

        # Plan at the group leader, bind owners at the metadata leader, then
        # capture at the group leader; capture verifies the committed pins
        # from local applied state before any upload.
        planned = command('plan', 'client', 'plan-migration-image', '--addr', addresses[data_node],
                          '--root-digest', root, '--operation-id', f'{7:032x}',
                          '--manifest-file', output / 'image.manifest')
        require(int(planned['cut_index']) > 0, 'plan lacks an exact cut')
        # Capturing before binding must refuse: pins precede any upload.
        command('capture-unpinned', 'client', 'capture-migration-image', '--addr', addresses[data_node],
                '--root-digest', root, '--operation-id', f'{7:032x}',
                '--record-file', output / 'never.record', success=False)
        owner_node = wait('metadata leader before binding', leader)
        command('bind', 'client', 'bind-migration-image', '--addr', addresses[owner_node],
                '--root-digest', root, '--operation-id', f'{7:032x}',
                '--manifest-file', output / 'image.manifest')

        # Capture refusals: uncommitted operation; a follower for the group.
        command('capture-uncommitted', 'client', 'capture-migration-image', '--addr', addresses[data_node],
                '--root-digest', root, '--operation-id', f'{9:032x}',
                '--record-file', output / 'never.record', success=False)
        follower = next(n for n in (1, 2, 3) if n != data_node)
        refused = command('capture-follower', 'client', 'capture-migration-image', '--addr', addresses[follower],
                          '--root-digest', root, '--operation-id', f'{7:032x}',
                          '--record-file', output / 'never.record', success=False)
        require('not_leader=true' in open(output / 'capture-follower.log').read(),
                'follower capture must return an explicit leader refusal')

        captured = command('capture', 'client', 'capture-migration-image', '--addr', addresses[data_node],
                           '--root-digest', root, '--operation-id', f'{7:032x}',
                           '--record-file', output / 'image.record')
        record = (output / 'image.record').read_bytes()
        require(record[:8] == b'KV9RSN01', 'record magic differs')
        require(hashlib.sha256(record).hexdigest() == captured['image_digest'], 'record digest differs')
        require(int(captured['cut_index']) > 0 and int(captured['cut_term']) > 0, 'missing exact cut')
        require(int(captured['objects']) >= 1, 'empty closure')
        for name in ('source_owner', 'destination_owner'):
            owner = command(f'read-{name}', 'client', 'retention-owner', '--addr', addresses[owner_node],
                            '--root-digest', root, '--owner-id', captured[name])
            require(owner.get('found') == 'true', f'{name} not committed')

        # Re-capturing the unchanged cut is idempotent: same image digest,
        # same owners. After the cut advances the subject changes, so the
        # same operation refuses a second image: one image per operation.
        again = command('capture-again', 'client', 'capture-migration-image', '--addr', addresses[data_node],
                        '--root-digest', root, '--operation-id', f'{7:032x}',
                        '--record-file', output / 'image-again.record')
        require(again['image_digest'] == captured['image_digest'], 're-capture changed the image')
        command('put-advance', 'client', 'raw-put', '--addr', addresses[data_node],
                '--keyspace', keyspace_id, '--key-hex', '67616d6d61', '--value-hex', '7468726565')
        command('capture-second-image', 'client', 'capture-migration-image', '--addr', addresses[data_node],
                '--root-digest', root, '--operation-id', f'{7:032x}',
                '--record-file', output / 'second.record', success=False)

        manifest_record.update(verdict='accepted', region=region,
                               image_digest=captured['image_digest'],
                               cut=dict(term=int(captured['cut_term']), index=int(captured['cut_index'])),
                               owners=dict(source=captured['source_owner'],
                                           destination=captured['destination_owner']),
                               checks=['leader-only capture with typed follower refusal',
                                       'uncommitted operation refused',
                                       'canonical KV9RSN01 record with matching digest and exact cut',
                                       'source and destination owners committed before upload',
                                       'idempotent re-capture of an unchanged cut',
                                       'second image after cut advance refused: one image per operation'])
    except Exception as error:
        manifest_record.update(verdict='failed', error=str(error))
        raise
    finally:
        for node in list(processes):
            stop(node)
        for handle in handles:
            handle.close()
        (output / 'result.json').write_text(json.dumps(manifest_record, indent=2) + '\n')


if __name__ == '__main__':
    main()
