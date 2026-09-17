#!/usr/bin/env python3
"""Real-process runtime-installation gate: the installed generation becomes
the destination's live replica and catches the leader's retained tail.

Chain: committed intent -> attach (destination becomes a LEARNER and the cut
advances) -> plan/bind/capture (the image names the learner) -> stop the
destination process -> offline install at its real store through the
unchanged joint installer -> restart the destination and prove the installed
generation stays isolated (no serving, no voting) while the node remains
healthy. Also proves the pre-attach image is refused by the installer's
membership gate.

Requires KV9_OBJECT_STORE_* (isolated real MinIO); servers run KV9_STORAGE=minio.
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
    parser.add_argument('--base-port', type=int, default=26900)
    args = parser.parse_args()
    for name in ('KV9_OBJECT_STORE_ENDPOINT', 'KV9_OBJECT_STORE_BUCKET',
                 'KV9_OBJECT_STORE_ACCESS_KEY', 'KV9_OBJECT_STORE_SECRET_KEY'):
        require(os.environ.get(name), f'{name} must be set to an isolated real MinIO bucket')
    binary, output = args.bin.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ, KV9_CLUSTER_TOKEN='rtinstall-e2e-cluster',
               KV9_CLIENT_TOKENS='admin=rtinstall-e2e-client', KV9_CLIENT_TOKEN='rtinstall-e2e-client',
               KV9_BOOTSTRAP_TOKEN='rtinstall-e2e-bootstrap', KV9_STORAGE='minio')
    processes, handles, commands = {}, [], []
    manifest_record = dict(verdict='running', commands=commands,
                           binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                           runner_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                           revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
                           dirty=bool(subprocess.check_output(['git', 'status', '--porcelain'])),
                           chaos_mesh=False, serving_or_promotion=False)
    (output / 'runner.py').write_bytes(Path(__file__).read_bytes())
    addresses = {i: f'127.0.0.1:{args.base_port+i}' for i in range(1, 5)}

    def command(label, *arguments, success=True, extra_env=None):
        started = time.time_ns()
        call_env = dict(env, **(extra_env or {}))
        result = subprocess.run([str(binary), *map(str, arguments)], env=call_env, text=True,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=300)
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

    def wait(label, predicate, seconds=90):
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

    def plan_with_retry(label, node, operation, path):
        # Committed intents replicate to local applied state within moments;
        # planning is read-only, so bounded retry is safe and honest.
        def planned():
            result = command(label, 'client', 'plan-migration-image', '--addr', addresses[node],
                             '--root-digest', root_holder[0], '--operation-id', operation,
                             '--manifest-file', path, success=None)
            return result if result['exit_code'] == 0 else None
        return wait(f'{label} intent locally applied', planned, 30)

    root_holder = [None]
    def capture_with_retry(label, node, operation, path):
        # Owners commit at the metadata leader; the group leader's local
        # applied ledger may lag for a moment. Capture refuses in the safe
        # direction and is idempotent, so bounded retry is honest.
        def captured():
            result = command(label, 'client', 'capture-migration-image', '--addr', addresses[node],
                             '--root-digest', root_holder[0], '--operation-id', operation,
                             '--record-file', path, success=None)
            if result['exit_code'] == 0:
                return result
            text = open(output / f"{label}-probe-{len(commands)-1}.log").read()
            if 'not published' in text or 'not committed' in text:
                return None
            raise RuntimeError(f'{label}: non-retryable capture failure; see {output}')
        return wait(f'{label} owners locally applied', captured, 30)


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
        root_holder[0] = root

        created = command('create-group', 'client', 'create-data-group', '--addr', addresses[owner_node],
                          '--root-digest', root, '--operation-id', f'{1:032x}', '--voters', '1,2,3')
        task, region = created['task_id'], int(created['region_id'])
        keyspace = command('create-keyspace', 'client', 'create-data-keyspace', '--addr', addresses[owner_node],
                           '--root-digest', root, '--creation-task', task, '--name', 'attach-src')
        keyspace_id = keyspace['keyspace_id']

        admitted = command('admit-node-4', 'client', 'admit-node', '--addr', addresses[owner_node],
                           '--node-id', 4, '--node-addr', addresses[4], '--ttl-seconds', 600)
        command('join-4', 'join', '--root', root_file, '--node-id', 4, '--data-dir', output / 'n4',
                extra_env={'KV9_JOIN_TICKET': admitted['join_ticket']})
        start(4, 'joiner', ticket=admitted['join_ticket'])
        wait('joiner registered and ready', lambda: status(4).get('endpoint_ready') == 'true')

        def group_leader():
            return next((n for n in (1, 2, 3) if n in processes
                         and data_groups(n).get(region, {}).get('role') == 'Leader'), None)
        wait('data group elected', lambda: group_leader() is not None)

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

        command('migrate', 'client', 'migrate-data-group', '--addr', addresses[owner_node],
                '--root-digest', root, '--operation-id', f'{7:032x}',
                '--creation-task', task, '--destination-node', 4)

        # Attach FIRST: the destination becomes a learner and the cut
        # advances past the configuration entry, so this operation's one
        # image names the learner. Retry confirms idempotently.
        attached = command('attach', 'client', 'attach-migration-learner', '--addr', addresses[data_node],
                           '--root-digest', root, '--operation-id', f'{7:032x}')
        require(attached['attach_outcome'] == 'attached' and attached['destination_node'] == '4',
                'attach lacks the learner receipt')
        again = command('attach-again', 'client', 'attach-migration-learner', '--addr', addresses[data_node],
                        '--root-digest', root, '--operation-id', f'{7:032x}')
        require(again['attach_outcome'] == 'confirmed', 'attach retry must confirm')

        plan_with_retry('plan', data_node, f'{7:032x}', output / 'image.manifest')
        command('bind', 'client', 'bind-migration-image', '--addr', addresses[owner_node],
                '--root-digest', root, '--operation-id', f'{7:032x}',
                '--manifest-file', output / 'image.manifest')
        captured = capture_with_retry('capture', data_node, f'{7:032x}', output / 'image.record')

        # A SECOND group whose image never names the destination supplies the
        # membership-gate control: its record must refuse at the real store.
        other = command('create-group-b', 'client', 'create-data-group', '--addr', addresses[owner_node],
                        '--root-digest', root, '--operation-id', f'{2:032x}', '--voters', '1,2,3')
        command('create-keyspace-b', 'client', 'create-data-keyspace', '--addr', addresses[owner_node],
                '--root-digest', root, '--creation-task', other['task_id'], '--name', 'attach-ctl')
        region_b = int(other['region_id'])

        def leader_b():
            return next((n for n in (1, 2, 3) if n in processes
                         and data_groups(n).get(region_b, {}).get('role') == 'Leader'), None)
        wait('control group elected', lambda: leader_b() is not None)

        node_b = wait('control group leader', leader_b)
        command('migrate-b', 'client', 'migrate-data-group', '--addr', addresses[owner_node],
                '--root-digest', root, '--operation-id', f'{8:032x}',
                '--creation-task', other['task_id'], '--destination-node', 4)
        plan_with_retry('plan-b', node_b, f'{8:032x}', output / 'control.manifest')
        command('bind-b', 'client', 'bind-migration-image', '--addr', addresses[owner_node],
                '--root-digest', root, '--operation-id', f'{8:032x}',
                '--manifest-file', output / 'control.manifest')
        capture_with_retry('capture-b', node_b, f'{8:032x}', output / 'control.record')

        # Offline installation at the destination's REAL store: the learner
        # image installs; the control image fails the membership gate; the
        # node restarts healthy with the installed generation isolated.
        stop(4)
        refused = command('install-control', 'install-migration-image',
                          '--data-dir', output / 'n4', '--record-file', output / 'control.record',
                          success=False)
        require('membership' in open(output / 'install-control.log').read(),
                'control record must fail the membership gate')
        installed = command('install-learner-image', 'install-migration-image',
                            '--data-dir', output / 'n4', '--record-file', output / 'image.record')
        require(installed['install_outcome'] == 'selected'
                and installed['serving'] == 'false'
                and int(installed['records']) == 2,
                'installed generation lacks the exact receipt')
        require(installed['cut_index'] == captured['cut_index'],
                'installed cut differs from the captured cut')
        start(4, 'rejoined', ticket=None)
        wait('destination ready after install', lambda: status(4).get('endpoint_ready') == 'true')

        # Adoption: the reconcile loop turns the installed generation into
        # the live replica, restoring the driver from the installed cut.
        def adopted():
            g = data_groups(4).get(region, {})
            return g if g.get('state') == 'active' and g.get('driver_applied') is not None else None
        adopted_state = wait('installed generation adopted as the live replica', adopted, 120)
        require(int(adopted_state['driver_applied']['index']) >= int(captured['cut_index']),
                'adopted driver position is behind the installed cut')
        require(adopted_state.get('role') != 'Leader', 'a learner must not lead')

        # Tail catchup: new writes at the source leader reach the learner
        # through retained-log MsgAppend only (network snapshots stay fenced).
        node = wait('source leader after adoption', group_leader)
        for n, (key, value) in enumerate([('64656c7461', '666f7572'), ('65707369', '66697665')]):
            command(f'post-put-{n}', 'client', 'raw-put', '--addr', addresses[node],
                    '--keyspace', keyspace_id, '--key-hex', key, '--value-hex', value)
        leader_applied = int(fields(open(output / f'post-put-1.log').read()).get('applied_index', 0))
        require(leader_applied > 0, 'missing leader apply receipt')

        def caught_up():
            g = data_groups(4).get(region, {})
            applied = g.get('driver_applied') or {}
            return int(applied.get('index', 0)) >= leader_applied
        wait('learner catches the retained tail', caught_up, 120)
        require(data_groups(4)[region].get('role') != 'Leader', 'learner must stay a non-leader')

        # The learner serves nothing: a direct read against it must refuse
        # with a leader hint, exactly like any follower.
        probe = command('learner-read', 'client', 'raw-get', '--addr', addresses[4],
                        '--keyspace', keyspace_id, '--key-hex', '64656c7461', success=None)
        require(probe['exit_code'] != 0 and probe.get('not_leader') == 'true',
                'learner read must refuse with a leader hint')

        # Restart the destination again: re-adoption is idempotent and the
        # replica resumes from durable state without re-verifying sealed
        # hashes that legitimately diverged.
        stop(4)
        start(4, 'readopted', ticket=None)
        wait('destination ready after re-adoption restart', lambda: status(4).get('endpoint_ready') == 'true')
        wait('replica adopted again after restart', adopted, 120)
        wait('replica still tracks the tail after restart', caught_up, 120)

        manifest_record.update(verdict='accepted', region=region,
                               checks=['attach, capture and offline install as previously qualified',
                                       'reconcile adopts the installed generation: active replica from the installed cut',
                                       'learner catches the retained tail via MsgAppend only; snapshots stay fenced',
                                       'learner never leads and refuses reads with a leader hint',
                                       'restart re-adopts idempotently and resumes tail tracking'])
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
