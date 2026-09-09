#!/usr/bin/env python3
"""Local production-binary endpoint migration and supported writer upgrade acceptance."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import time


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def fields(text):
    result = {}
    for word in text.split():
        if '=' in word:
            key, value = word.split('=', 1)
            require(key not in result, f'duplicate output field: {key}')
            result[key] = value
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bin', required=True, type=Path)
    parser.add_argument('--previous-bin', required=True, type=Path)
    parser.add_argument('--wire-probe', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--base-port', type=int, default=26400)
    args = parser.parse_args()
    require(1024 < args.base_port < 32700, 'ports must stay below the ephemeral range')
    args.bin, args.previous_bin, output = args.bin.resolve(), args.previous_bin.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ, KV9_CLUSTER_TOKEN='endpoint-e2e-cluster',
               KV9_CLIENT_TOKENS='admin=endpoint-e2e-client', KV9_CLIENT_TOKEN='endpoint-e2e-client',
               KV9_BOOTSTRAP_TOKEN='endpoint-e2e-bootstrap')
    # Production verification must not inherit a testing partition or object-store fixture.
    require(not any(k.startswith(('KV9_TESTING_', 'KV9_OBJECT_STORE_')) for k in os.environ),
            'run this fixture without inherited testing/object-store configuration')
    processes, handles, commands, guards = {}, [], [], []
    args.wire_probe = args.wire_probe.resolve()
    manifest = dict(version=1, binaries={str(p): sha(p) for p in (args.bin, args.previous_bin, args.wire_probe)},
                    base_port=args.base_port, commands=commands, verdict='running',
                    source_sha256=sha(Path(__file__).resolve()),
                    revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
                    dirty=bool(subprocess.check_output(['git', 'status', '--porcelain'])))
    (output / 'runner.py').write_bytes(Path(__file__).read_bytes())
    root = output / 'root.bin'
    addresses = {i: f'127.0.0.1:{args.base_port+i}' for i in range(1, 4)}

    def command(label, binary, *arguments, success=True):
        started = time.time_ns()
        result = subprocess.run([str(binary), *map(str, arguments)], env=env, text=True,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=40)
        (output / f'{label}.log').write_text(result.stdout)
        commands.append(dict(label=label, argv=[str(binary), *map(str, arguments)],
                             started_ns=started, finished_ns=time.time_ns(), exit_code=result.returncode))
        require((result.returncode == 0) == success, f'{label}: unexpected exit {result.returncode}; see {output}')
        return result.stdout

    def start(node, binary, address, label, advertise=None):
        data = output / f'n{node}'
        argv = [str(binary), 'start', '--node-id', str(node), '--addr', address, '--data-dir', str(data)]
        if advertise:
            argv += ['--advertise-addr', advertise]
        log = (output / f'{label}.log').open('w')
        handles.append(log)
        process = subprocess.Popen(argv, env=env, stdout=log, stderr=subprocess.STDOUT)
        processes[node] = process
        commands.append(dict(label=label, argv=argv, pid=process.pid, started_ns=time.time_ns()))

    def stop(node):
        process = processes.pop(node)
        if process.poll() is None:
            process.send_signal(signal.SIGKILL)
        process.wait(timeout=10)

    def status(node):
        process = processes.get(node)
        if process is None or process.poll() is not None:
            return {}
        path = output / f'n{node}/status'
        if not path.exists():
            return {}
        value = dict(line.split('=', 1) for line in path.read_text().splitlines() if '=' in line)
        return value if value.get('pid') == str(process.pid) else {}

    def wait(label, predicate, seconds=60):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            result = predicate()
            if result:
                print(f'PASS: {label}', flush=True)
                return result
            time.sleep(.05)
        raise RuntimeError(f'timed out: {label}; evidence: {output}')

    def serving(node):
        return status(node).get('bootstrap_state') == 'Serving'

    def leader(nodes):
        snapshots = [status(i) for i in nodes]
        hints = {s.get('leader_id') for s in snapshots}
        if all(s.get('bootstrap_state') == 'Serving' for s in snapshots) and len(hints) == 1:
            candidate = next(iter(hints))
            if candidate and candidate.isdigit() and int(candidate) in nodes:
                return int(candidate)
        return None

    def client(label, operation, target, *arguments, success=True):
        return command(label, args.bin, 'client', operation, '--addr', target, *arguments, success=success)

    try:
        incarnations = []
        for node in addresses:
            guard = socket.socket()
            guard.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            guard.bind(('127.0.0.1', args.base_port + node))
            guards.append(guard)
            prepared = fields(command(f'old-prepare-{node}', args.previous_bin, 'store-prepare',
                                     '--node-id', node, '--data-dir', output / f'n{node}'))
            incarnations.append(f"{node}={prepared['store_incarnation']}")
        command('old-root-create', args.previous_bin, 'root-create', '--output', root,
                '--voters', ','.join(f'{i}@{a}' for i, a in addresses.items()),
                '--store-incarnations', ','.join(incarnations))
        for node in addresses:
            command(f'old-init-{node}', args.previous_bin, 'init', '--root', root,
                    '--node-id', node, '--data-dir', output / f'n{node}')
        for guard in guards:
            guard.close()
        guards.clear()
        for node, address in addresses.items():
            start(node, args.previous_bin, address, f'old-owner-{node}')
        old_leader = wait('previous production writers form a durable cluster', lambda: leader([1, 2, 3]))
        old_keyspace = fields(command('old-create-keyspace', args.previous_bin, 'client', 'create-keyspace',
                                     '--addr', addresses[old_leader], '--name', 'previous-writer-data', '--api-type', 'raw'))['keyspace_id']
        command('old-acknowledged-write', args.previous_bin, 'client', 'raw-put', '--addr', addresses[old_leader],
                '--keyspace', old_keyspace, '--key-hex', '6f6c64', '--value-hex', '64757261626c65')
        original = {}
        for node in addresses:
            data = output / f'n{node}'
            require((data / 'kv9-store-lifecycle').read_bytes()[:8] == b'KV9LIFE1', 'old writer fixture is not V1')
            original[node] = {name: sha(data / name) for name in ('kv9-root-descriptor', 'kv9-store-identity')}
            (output / f'old-status-{node}.txt').write_text((data / 'status').read_text())
        # Briefly expose both binaries only to test the wire refusal floor.
        # No assertion of mixed-generation quorum availability is made.
        first = next(node for node in addresses if node != old_leader)
        stop(first)
        start(first, args.bin, addresses[first], 'wire-probe-upgraded-owner')
        wait('upgraded process exposes its distinct internal service path', lambda: status(first))
        command('bidirectional-wire-floor', args.wire_probe, addresses[old_leader], addresses[first])
        # Offline upgrade is the supported operational boundary.
        for node in list(processes):
            stop(node)
        for node, address in addresses.items():
            start(node, args.bin, address, f'upgraded-owner-{node}')
        current_leader = wait('upgraded production writers recover the same cluster', lambda: leader([1, 2, 3]))
        recovered = fields(client('read-previous-writer-data', 'raw-get', addresses[current_leader],
                                  '--keyspace', old_keyspace, '--key-hex', '6f6c64'))
        require(recovered.get('value_hex') == '64757261626c65', 'upgrade lost the previous writer acknowledgement')
        for node in addresses:
            data = output / f'n{node}'
            require((data / 'kv9-store-lifecycle').read_bytes()[:8] == b'KV9LIFE2', 'writer floor not published')
            require(original[node] == {name: sha(data / name) for name in original[node]}, 'upgrade changed root/store identity')
        victim = next(node for node in addresses if node != current_leader)
        before = fields(client('endpoint-before', 'get-node-endpoint', addresses[current_leader], '--node-id', victim))
        require(before['generation'] == '0', 'unexpected initial endpoint generation')
        keyspace = fields(client('create-keyspace', 'create-keyspace', addresses[current_leader],
                                '--name', 'endpoint-migration', '--api-type', 'raw'))['keyspace_id']
        client('write-before-migration', 'raw-put', addresses[current_leader], '--keyspace', keyspace,
               '--key-hex', '6d696772617465', '--value-hex', '707265736572766564')
        stop(victim)
        data = output / f'n{victim}'
        protected = {str(p.relative_to(data)): sha(p) for p in data.rglob('*') if p.is_file()}
        refusal = command('previous-writer-refused', args.previous_bin, 'start', '--node-id', victim,
                          '--addr', addresses[victim], '--data-dir', data, success=False)
        require('invalid store lifecycle record format' in refusal, 'old writer did not refuse the format floor')
        require(protected == {str(p.relative_to(data)): sha(p) for p in data.rglob('*') if p.is_file()},
                'refused previous writer changed the upgraded durable store')
        old_guard = socket.socket()
        old_guard.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        old_guard.bind(('127.0.0.1', args.base_port + victim))
        guards.append(old_guard)  # Bound but not listening: the obsolete endpoint is unavailable.
        new_address = f'127.0.0.1:{args.base_port+10+victim}'
        start(victim, args.bin, f'0.0.0.0:{args.base_port+10+victim}', 'changed-owner-unconfirmed', new_address)
        wait('changed root voter keeps Raft authority while public serving is closed', lambda:
             status(victim).get('endpoint_ready') == 'false'
             and status(victim).get('raft_owner_started') == 'true'
             and status(victim).get('raft_receive_authorized') == 'true'
             and int(status(victim).get('endpoint_recovery_attempts', 0)) > 0)
        (output / 'unconfirmed-status.txt').write_text((data / 'status').read_text())
        survivors = [i for i in addresses if i != victim]
        current_leader = wait('surviving quorum retains a leader', lambda: leader(survivors))
        update = ['--cluster-id', before['cluster_id'], '--node-id', victim,
                  '--store-incarnation', before['store_incarnation'], '--expected-address', addresses[victim],
                  '--expected-generation', 0, '--new-address', new_address]
        mutation = fields(client('authorize-endpoint', 'change-node-endpoint', addresses[current_leader], *update))
        require(mutation['endpoint_outcome'] == 'changed' and mutation['generation'] == '1', 'CAS did not change once')
        confirmed = fields(client('duplicate-endpoint', 'change-node-endpoint', addresses[current_leader], *update))
        require(confirmed['endpoint_outcome'] == 'confirmed' and confirmed['generation'] == '1'
                and int(confirmed['confirmation_index']) > int(mutation['mutation_index']),
                'duplicate reused mutation position or advanced generation')
        wait('new endpoint applies fresh confirmation and enters Serving', lambda:
             serving(victim) and status(victim).get('endpoint_ready') == 'true'
             and status(victim).get('endpoint_confirmation_index', 'none') != 'none'
             and int(status(victim)['applied_index']) >= int(status(victim)['endpoint_confirmation_index']))
        migrated = status(victim)
        require(int(migrated['endpoint_confirmation_index']) > int(mutation['mutation_index']),
                'owner reused the operator mutation receipt')
        require(migrated['store_incarnation'] == before['store_incarnation'], 'migration replaced the store')
        require(original[victim] == {name: sha(data / name) for name in original[victim]}, 'migration changed durable identity')
        require(migrated['listen_addr'].startswith('0.0.0.0:'), 'bind/advertise separation was not exercised')
        (output / 'confirmed-status.txt').write_text((data / 'status').read_text())
        value = fields(client('read-after-migration', 'raw-get', addresses[current_leader],
                              '--keyspace', keyspace, '--key-hex', '6d696772617465'))
        require(value.get('value_hex') == '707265736572766564', 'acknowledged value changed during migration')
        addresses[victim] = new_address
        for node in list(processes):
            stop(node)
        start(victim, args.bin, new_address, 'stable-migrated-owner', new_address)
        wait('stable migrated voter recovers with every other database process stopped', lambda: serving(victim))
        stable = status(victim)
        require(stable['endpoint_recovery_attempts'] == stable['registration_attempts'] == '0',
                'stable restart depended on a live coordinator')
        (output / 'stable-status.txt').write_text((data / 'status').read_text())
        manifest.update(verdict='accepted', original_leader=old_leader, victim=victim,
                        mutation=mutation, duplicate=confirmed, recovered=migrated, stable=stable)
        print('PASS: production CLI endpoint CAS, retained-store migration, durable confirmation, V1 upgrade and downgrade refusal', flush=True)
    finally:
        for node in list(processes):
            stop(node)
        for handle in handles:
            handle.close()
        for guard in guards:
            guard.close()
        (output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')


if __name__ == '__main__':
    main()
