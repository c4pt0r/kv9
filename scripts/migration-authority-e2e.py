#!/usr/bin/env python3
"""Local durable migration-authority gate over real processes; no transfer,
learner, serving or scaling capability is exercised or claimed."""
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
    parser.add_argument('--base-port', type=int, default=26840)
    args = parser.parse_args()
    require(1024 < args.base_port < 32700, 'use non-ephemeral ports')
    binary, output = args.bin.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    require(not any(k.startswith(('KV9_TESTING_', 'KV9_OBJECT_STORE_', 'KV9_JOIN_')) for k in os.environ),
            'run without inherited testing/object-store/join configuration')
    env = dict(os.environ, KV9_CLUSTER_TOKEN='migrate-e2e-cluster',
               KV9_CLIENT_TOKENS='admin=migrate-e2e-client', KV9_CLIENT_TOKEN='migrate-e2e-client',
               KV9_BOOTSTRAP_TOKEN='migrate-e2e-bootstrap')
    processes, handles, guards, commands = {}, [], [], []
    manifest_record = dict(verdict='running', commands=commands,
                           binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                           runner_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                           revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
                           dirty=bool(subprocess.check_output(['git', 'status', '--porcelain'])),
                           chaos_mesh=False, scaling_benchmark=False, transfer_or_serving=False)
    (output / 'runner.py').write_bytes(Path(__file__).read_bytes())
    addresses = {i: f'127.0.0.1:{args.base_port+i}' for i in range(1, 5)}

    def command(label, *arguments, success=True, token=None, extra_env=None):
        started = time.time_ns()
        call_env = dict(env if token is None else dict(env, KV9_CLIENT_TOKEN=token), **(extra_env or {}))
        result = subprocess.run([str(binary), *map(str, arguments)], env=call_env, text=True,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=40)
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
        commands.append(dict(label=label, node=node, pid=process.pid, started_ns=time.time_ns()))

    def stop(node):
        process = processes.pop(node)
        if process.poll() is None:
            process.kill()
        process.wait(timeout=10)

    def status(node):
        process = processes.get(node)
        if process is None or process.poll() is not None:
            return {}
        path = output / f'n{node}/status'
        if not path.exists():
            return {}
        value = fields(path.read_text())
        return value if value.get('pid') == str(process.pid) else {}

    def wait(label, predicate):
        deadline = time.monotonic() + 60
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

    def migrate(label, node, operation, root, task, destination, **kwargs):
        return command(label, 'client', 'migrate-data-group', '--addr', addresses[node],
                       '--root-digest', root, '--operation-id', f'{operation:032x}',
                       '--creation-task', task, '--destination-node', destination, **kwargs)

    def bind_image(label, node, operation, root, path, **kwargs):
        return command(label, 'client', 'bind-migration-image', '--addr', addresses[node],
                       '--root-digest', root, '--operation-id', f'{operation:032x}',
                       '--manifest-file', path, **kwargs)

    def read_owner(label, node, root, owner, **kwargs):
        return command(label, 'client', 'retention-owner', '--addr', addresses[node],
                       '--root-digest', root, '--owner-id', owner, **kwargs)

    try:
        incarnations = []
        for node in list(addresses)[:3]:
            guard = socket.socket()
            guard.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            guard.bind(('127.0.0.1', args.base_port + node))
            guards.append(guard)
            prepared = command(f'prepare-{node}', 'store-prepare', '--node-id', node, '--data-dir', output / f'n{node}')
            incarnations.append(f"{node}={prepared['store_incarnation']}")
        root_file = output / 'root.bin'
        command('root-create', 'root-create', '--output', root_file,
                '--voters', ','.join(f'{n}@{addresses[n]}' for n in list(addresses)[:3]),
                '--store-incarnations', ','.join(incarnations))
        for node in list(addresses)[:3]:
            command(f'init-{node}', 'init', '--root', root_file, '--node-id', node, '--data-dir', output / f'n{node}')
        for guard in guards:
            guard.close()
        guards.clear()
        for node in list(addresses)[:3]:
            start(node, 'initial')
        owner_node = wait('metadata leader', leader)
        wait('all endpoints ready', lambda: all(status(n).get('endpoint_ready') == 'true' for n in processes))
        root = status(owner_node)['root_digest']
        cluster = status(owner_node)['cluster_id']

        created = command('create-group', 'client', 'create-data-group', '--addr', addresses[owner_node],
                          '--root-digest', root, '--operation-id', f'{1:032x}', '--voters', '1,2,3')
        task, region = created['task_id'], int(created['region_id'])
        keyspace = command('create-keyspace', 'client', 'create-data-keyspace', '--addr', addresses[owner_node],
                           '--root-digest', root, '--creation-task', task, '--name', 'migrate-authority')
        require(int(keyspace['region_id']) == region, 'keyspace bound to another group')

        # A genuine fourth store joins through the production admission flow.
        admitted = command('admit-node-4', 'client', 'admit-node', '--addr', addresses[owner_node],
                           '--node-id', 4, '--node-addr', addresses[4], '--ttl-seconds', 600)
        ticket = admitted['join_ticket']
        guard = socket.socket()
        guard.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        guard.bind(('127.0.0.1', args.base_port + 4))
        guards.append(guard)
        prepared4 = command('join-4', 'join', '--root', root_file, '--node-id', 4,
                            '--data-dir', output / 'n4', extra_env={'KV9_JOIN_TICKET': ticket})
        require(prepared4.get('mode') == 'join', 'joiner provisioning must use the join path')
        guard.close()
        guards.clear()
        start(4, 'joiner', ticket=ticket)
        wait('joiner registered and ready', lambda: status(4).get('endpoint_ready') == 'true')

        # Refusals precede any committed intent.
        migrate('reject-credential', owner_node, 7, root, task, 4, success=False, token='wrong-token')
        follower = next(n for n in (1, 2, 3) if n != owner_node)
        refused = migrate('refuse-follower', follower, 7, root, task, 4, success=False)
        require(refused.get('not_leader', '').split(' ')[0] == 'true', 'follower must return an explicit leader refusal')
        migrate('reject-initial-replica', owner_node, 7, root, task, 1, success=False)
        migrate('reject-unknown-creation', owner_node, 7, root, int(task) + 999, 4, success=False)

        first = migrate('request-first', owner_node, 7, root, task, 4)
        require(first['migration_outcome'] == 'requested', 'new intent lacks a mutation receipt')
        require(int(first['region_id']) == region, 'intent bound to another group')
        require(first['destination_incarnation'] == prepared4['store_incarnation'],
                'intent must bind the destination current exact incarnation')
        require(first['capability'] == 'image_owner_binding_only', 'receipt overclaims capability')
        retry = migrate('confirm-first', owner_node, 7, root, task, 4)
        require(retry['migration_outcome'] == 'confirmed'
                and retry['task_id'] == first['task_id']
                and int(retry['confirmation_index']) > int(first['mutation_index']),
                'retry changed immutable identity or reused the original receipt')
        migrate('reject-second-live-migration', owner_node, 8, root, task, 4, success=False)

        # One canonical image description; the manifest is data, not authority.
        sha = hashlib.sha256(b'migration-authority-sst').hexdigest()
        image = {'scope': {'cluster': cluster, 'region': region, 'conf_ver': 1, 'version': 1},
                 'term': 3, 'index': 10,
                 'files': [{'key': f'clusters/{cluster}/regions/{region}/sst/{sha}', 'sha256': sha,
                            'cf': 0, 'smallest': [97], 'largest': [109], 'size': 1024, 'count': 3}]}
        manifest_path = output / 'image.manifest'
        manifest_path.write_bytes(b'KV9CHECKPOINT\x01' + json.dumps(image, separators=(',', ':')).encode())
        foreign = dict(image, scope=dict(image['scope'], region=region + 1))
        foreign_path = output / 'foreign.manifest'
        foreign_path.write_bytes(b'KV9CHECKPOINT\x01' + json.dumps(foreign, separators=(',', ':')).encode())
        second = dict(image, index=11)
        second_path = output / 'second.manifest'
        second_path.write_bytes(b'KV9CHECKPOINT\x01' + json.dumps(second, separators=(',', ':')).encode())

        bind_image('reject-uncommitted-operation', owner_node, 9, root, manifest_path, success=False)
        bind_image('reject-foreign-scope', owner_node, 7, root, foreign_path, success=False)
        bound = bind_image('bind-image', owner_node, 7, root, manifest_path)
        require(bound['binding_outcome'] == 'bound' and bound['capability'] == 'tracking_only_pins',
                'binding lacks exact owner receipts')
        owners = {'source': bound['source_owner'], 'destination': bound['destination_owner']}
        observed = {name: read_owner(f'read-{name}', owner_node, root, owner)['owner_hex']
                    for name, owner in owners.items()}
        rebound = bind_image('rebind-image', owner_node, 7, root, manifest_path)
        require(rebound == bound, 'rebinding the same image changed the owners')
        bind_image('reject-second-image', owner_node, 7, root, second_path, success=False)

        # Leader loss: a successor confirms the exact rows and identical owners.
        stop(owner_node)
        successor = wait('metadata leader after process death', leader)
        after_loss = migrate('confirm-after-leader-loss', successor, 7, root, task, 4)
        require(after_loss['migration_outcome'] == 'confirmed'
                and after_loss['task_id'] == first['task_id'], 'leader loss changed the committed intent')
        rebound = bind_image('rebind-after-leader-loss', successor, 7, root, manifest_path)
        require(rebound == bound, 'leader loss changed the bound owners')
        start(owner_node, 'returned')
        wait('returned voter serves again', lambda: status(owner_node).get('endpoint_ready') == 'true')

        # Full restart: committed authority and identical owner bytes survive.
        for node in list(processes):
            stop(node)
        for node in (1, 2, 3):
            start(node, 'all-reopened')
        start(4, 'all-reopened-joiner')
        successor = wait('metadata leader after all-process restart', leader)
        wait('all endpoints ready after restart', lambda: all(status(n).get('endpoint_ready') == 'true' for n in processes))
        confirm = migrate('confirm-after-restart', successor, 7, root, task, 4)
        require(confirm['migration_outcome'] == 'confirmed'
                and confirm['task_id'] == first['task_id'], 'restart changed the committed intent')
        for name, owner in owners.items():
            after = read_owner(f'read-{name}-after-restart', successor, root, owner)
            require(after.get('found') == 'true' and after['owner_hex'] == observed[name],
                    f'restart changed the committed {name} owner observation')
        manifest_record.update(verdict='accepted', region=region, migration_task=first['task_id'],
                               owners=owners, independent_processes=4,
                               checks=['credential/root/follower refusals',
                                       'production fourth-store admission and join',
                                       'destination bound to current exact incarnation',
                                       'idempotent intent confirmation; one live migration per group',
                                       'image owners bound once; second image and foreign scope refused',
                                       'exact confirmations and identical owner bytes after leader loss',
                                       'exact rows and owner observations after all-process restart'])
    except Exception as error:
        manifest_record.update(verdict='failed', error=str(error))
        raise
    finally:
        for node in list(processes):
            stop(node)
        for handle in handles:
            handle.close()
        for guard in guards:
            guard.close()
        (output / 'result.json').write_text(json.dumps(manifest_record, indent=2) + '\n')


if __name__ == '__main__':
    main()
