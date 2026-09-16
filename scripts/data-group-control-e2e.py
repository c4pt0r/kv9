#!/usr/bin/env python3
"""Local three-process durable group-control gate; not a scaling benchmark."""
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
    parser.add_argument('--base-port', type=int, default=26810)
    parser.add_argument('--exercise-raw', action='store_true', help='also bind new Raw keyspaces and verify public data-group IO')
    args = parser.parse_args()
    require(1024 < args.base_port < 32700, 'use non-ephemeral ports')
    binary, output = args.bin.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    require(not any(k.startswith(('KV9_TESTING_', 'KV9_OBJECT_STORE_')) for k in os.environ),
            'run without inherited testing/object-store configuration')
    env = dict(os.environ, KV9_CLUSTER_TOKEN='group-e2e-cluster',
               KV9_CLIENT_TOKENS='admin=group-e2e-client', KV9_CLIENT_TOKEN='group-e2e-client',
               KV9_BOOTSTRAP_TOKEN='group-e2e-bootstrap')
    processes, handles, guards, commands = {}, [], [], []
    manifest = dict(verdict='running', commands=commands,
                    binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                    runner_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                    revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
                    dirty=bool(subprocess.check_output(['git', 'status', '--porcelain'])),
                    chaos_mesh=False, scaling_benchmark=False, public_data_routing=args.exercise_raw)
    (output / 'runner.py').write_bytes(Path(__file__).read_bytes())
    addresses = {i: f'127.0.0.1:{args.base_port+i}' for i in range(1, 4)}

    def command(label, *arguments, success=True, token=None):
        started = time.time_ns()
        call_env = env if token is None else dict(env, KV9_CLIENT_TOKEN=token)
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

    def start(node, label):
        log = (output / f'{label}-{node}.log').open('w')
        handles.append(log)
        process = subprocess.Popen([str(binary), 'start', '--node-id', str(node), '--addr', addresses[node],
                                    '--data-dir', str(output / f'n{node}')], env=env, stdout=log, stderr=subprocess.STDOUT)
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
        deadline = time.monotonic() + 40
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

    def groups(node):
        return {g['region']: g for g in json.loads(status(node).get('data_groups', '[]'))}

    def ready(regions):
        snapshots = [groups(n) for n in processes]
        for region in regions:
            if any(region not in s for s in snapshots):
                return False
            observations = [s[region] for s in snapshots]
            if not all(s['state'] == 'active' and s['committed'] > 0
                       and s['driver_applied'] is not None
                       and s['driver_applied']['index'] >= s['committed']
                       and s['driver_applied']['term'] == s['term'] for s in observations):
                return False
            if len({(s['leader'], s['term']) for s in observations}) != 1:
                return False
            if observations[0]['leader'] not in processes or sum(s['role'] == 'Leader' for s in observations) != 1:
                return False
        return True

    def capture(label):
        for node in processes:
            (output / f'{label}-{node}.json').write_text(json.dumps(status(node), indent=2) + '\n')

    def create(label, node, operation, root, **kwargs):
        return command(label, 'client', 'create-data-group', '--addr', addresses[node],
                       '--root-digest', root, '--operation-id', f'{operation:032x}', '--voters', '1,2,3', **kwargs)

    keyspaces = {}

    def data_leader(region):
        return next((n for n in processes if groups(n).get(region, {}).get('role') == 'Leader'), None)

    def raw_get(region, expected, label):
        node = data_leader(region)
        if node is None:
            return False
        result = command(label, 'client', 'raw-get', '--addr', addresses[node], '--keyspace', keyspaces[region],
                         '--key-hex', '73616d652d6b6579', success=None)
        return result['exit_code'] == 0 and result.get('value_hex') == expected

    def raw_put(region, value, label):
        node = data_leader(region)
        require(node is not None, 'data leader absent before one-shot write')
        receipt = command(label, 'client', 'raw-put', '--addr', addresses[node], '--keyspace', keyspaces[region],
                          '--key-hex', '73616d652d6b6579', '--value-hex', value)
        require(int(receipt['applied_term']) > 0 and int(receipt['applied_index']) > 0, 'missing exact write receipt')

    def bind(request, label, node, root):
        region = int(request['region_id'])
        result = command(label, 'client', 'create-data-keyspace', '--addr', addresses[node], '--root-digest', root,
                         '--creation-task', request['task_id'], '--name', f'raw-group-{region}')
        require(int(result['region_id']) == region, 'namespace bound to another group')
        keyspaces[region] = result['keyspace_id']
        # Read-only probes may retry unconfirmed reads. Writes are never retried.
        def available():
            node = data_leader(region)
            if node is None:
                return False
            r = command(f'{label}-ready', 'client', 'raw-get', '--addr', addresses[node], '--keyspace', keyspaces[region],
                        '--key-hex', '73616d652d6b6579', success=None)
            return r['exit_code'] == 0 and r.get('found') == 'false'
        wait(f'public Raw route for group {region}', available)

    try:
        incarnations = []
        for node in addresses:
            guard = socket.socket()
            guard.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            guard.bind(('127.0.0.1', args.base_port + node))
            guards.append(guard)
            prepared = command(f'prepare-{node}', 'store-prepare', '--node-id', node, '--data-dir', output / f'n{node}')
            incarnations.append(f"{node}={prepared['store_incarnation']}")
        root_file = output / 'root.bin'
        command('root-create', 'root-create', '--output', root_file,
                '--voters', ','.join(f'{n}@{a}' for n, a in addresses.items()),
                '--store-incarnations', ','.join(incarnations))
        for node in addresses:
            command(f'init-{node}', 'init', '--root', root_file, '--node-id', node, '--data-dir', output / f'n{node}')
        for guard in guards:
            guard.close()
        guards.clear()
        for node in addresses:
            start(node, 'initial')
        owner = wait('metadata leader', leader)
        wait('all endpoints ready', lambda: all(status(n).get('endpoint_ready') == 'true' for n in processes))
        root = status(owner)['root_digest']
        create('reject-credential', owner, 9, root, success=False, token='wrong-token')
        create('reject-root', owner, 9, 'ff' * 32, success=False)
        follower = next(n for n in processes if n != owner)
        refused = create('refuse-follower', follower, 9, root, success=False)
        require(refused.get('not_leader', '').split(' ')[0] == 'true', 'follower must return an explicit leader refusal')
        first = create('request-first', owner, 1, root)
        second = create('request-second', owner, 2, root)
        regions = [int(first['region_id']), int(second['region_id'])]
        require(first['group_outcome'] == second['group_outcome'] == 'requested', 'new desires lack mutation receipts')
        require(first['readiness'] == second['readiness'] == 'not_asserted', 'receipt overclaims readiness')
        wait('two independently elected durable groups', lambda: ready(regions))
        if args.exercise_raw:
            bind(first, 'bind-first', owner, root)
            bind(second, 'bind-second', owner, root)
            for region in regions:
                raw_put(region, f'{region:08x}', f'write-{region}-before-loss')
                wait(f'public read for {region}', lambda r=region: raw_get(r, f'{r:08x}', f'read-{r}-before-loss'))
            victim = data_leader(regions[0])
            stop(victim)
            wait('data groups elect after data-leader process death', lambda: ready(regions))
            for region in regions:
                wait(f'acknowledged data survives for {region}', lambda r=region: raw_get(r, f'{r:08x}', f'read-{r}-after-data-loss'))
                raw_put(region, f'{region:08x}', f'write-{region}-after-data-loss')
            start(victim, 'data-leader-returned')
            wait('data-leader voter catches up', lambda: ready(regions))
            owner = wait('metadata leader before next failure', leader)
        capture('before-loss')
        stop(owner)
        successor = wait('metadata leader after process death', leader)
        retry = create('confirm-first-after-loss', successor, 1, root)
        require(retry['group_outcome'] == 'confirmed' and retry['region_id'] == first['region_id']
                and retry['intent_digest'] == first['intent_digest'], 'retry changed immutable identity')
        require(int(retry['confirmation_index']) > int(first['mutation_index']), 'confirmation reused the original receipt')
        third = create('request-third-with-one-node-down', successor, 3, root)
        regions.append(int(third['region_id']))
        wait('surviving replicas activate a new group', lambda: ready(regions))
        if args.exercise_raw:
            bind(third, 'bind-third-with-voter-down', successor, root)
            raw_put(regions[-1], f'{regions[-1]:08x}', 'write-third-with-voter-down')
        start(owner, 'returned')
        wait('returned replica reconciles missing desire', lambda: ready(regions))
        capture('returned')
        for node in list(processes):
            stop(node)
        for node in addresses:
            start(node, 'all-reopened')
        wait('all processes recover all active groups', lambda: ready(regions))
        if args.exercise_raw:
            for region in regions:
                wait(f'public data recovered for {region}', lambda r=region: raw_get(r, f'{r:08x}', f'read-{r}-all-reopened'))
        capture('all-reopened')
        # Corrupt only one stopped replica's one group; other groups and metadata survive.
        stop(owner)
        missing = output / f'n{owner}/data-groups/{regions[0]}/raft/raft.log'
        missing.rename(missing.with_suffix('.retained'))
        start(owner, 'one-invalid-group')
        wait('invalid group stays isolated', lambda: groups(owner).get(regions[0], {}).get('state') == 'failed'
             and status(owner).get('endpoint_ready') == 'true' and ready(regions[1:]))
        require(not missing.exists(), 'recovery recreated a missing active log')
        capture('one-invalid-group')
        if args.exercise_raw:
            for region in regions[1:]:
                wait(f'healthy data group remains readable {region}', lambda r=region: raw_get(r, f'{r:08x}', f'read-{r}-beside-invalid'))
        for node in processes:
            require(len(groups(node)) == 3, 'invalid requests or retries created an extra group')
        manifest.update(verdict='accepted', regions=regions, keyspaces=keyspaces, independent_processes=3,
                        checks=['credential/root/follower refusals', 'online request and autonomous activation',
                                'new exact confirmation after leader loss', 'new group with one voter down',
                                'returning voter catches durable desires', 'all-process recovery',
                                'one missing data log isolated without recreation'])
    except Exception as error:
        manifest.update(verdict='failed', error=str(error))
        raise
    finally:
        for node in list(processes):
            stop(node)
        for handle in handles:
            handle.close()
        for guard in guards:
            guard.close()
        (output / 'result.json').write_text(json.dumps(manifest, indent=2) + '\n')


if __name__ == '__main__':
    main()
