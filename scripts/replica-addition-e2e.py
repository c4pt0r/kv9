#!/usr/bin/env python3
"""Replica addition and move driven by the cluster (docs/REPLICA-ADDITION.md).

Three voters hold a data group with a bound keyspace. A fourth registered store
joins; one `kv9-admin group add-replica` later the group votes with four members
and nobody ran attach/plan/bind/capture/install/evidence/promote by hand: each
party took its step from the committed plan, the destination installed the image
inside its running process. Then a fifth store joins and `group move-replica
--from 1 --to 5` grows to five and retires node 1, leaving voters 2,3,4,5. Writes
and linearizable reads continue throughout; refusals (a destination that is an
initial replica, a source that is not) stay refusals.

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
    out = {}
    for line in text.splitlines():
        if '=' in line:
            key, value = line.split('=', 1)
            out[key] = value
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bin', type=Path, required=True)
    parser.add_argument('--admin-bin', type=Path, required=True)
    parser.add_argument('--repl-bin', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--base-port', type=int, default=26960)
    args = parser.parse_args()
    for name in ('KV9_OBJECT_STORE_ENDPOINT', 'KV9_OBJECT_STORE_BUCKET',
                 'KV9_OBJECT_STORE_ACCESS_KEY', 'KV9_OBJECT_STORE_SECRET_KEY'):
        require(os.environ.get(name), f'{name} must be set to an isolated real MinIO bucket')
    binary, admin_bin, repl_bin = args.bin.resolve(), args.admin_bin.resolve(), args.repl_bin.resolve()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ, KV9_CLUSTER_TOKEN='addition-e2e-cluster',
               KV9_CLIENT_TOKENS='admin=addition-e2e-client', KV9_CLIENT_TOKEN='addition-e2e-client',
               KV9_ADMIN_TOKEN='addition-e2e-client',
               KV9_BOOTSTRAP_TOKEN='addition-e2e-bootstrap', KV9_STORAGE='minio')
    processes, handles, commands = {}, [], []
    manifest_record = dict(verdict='running', commands=commands,
                           binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                           admin_sha256=hashlib.sha256(admin_bin.read_bytes()).hexdigest(),
                           runner_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                           revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
                           dirty=bool(subprocess.check_output(['git', 'status', '--porcelain'])),
                           chaos_mesh=False)
    (output / 'runner.py').write_bytes(Path(__file__).read_bytes())
    addresses = {i: f'127.0.0.1:{args.base_port+i}' for i in range(1, 6)}

    def run(label, argv, success=True, extra_env=None, timeout=300):
        started = time.time_ns()
        result = subprocess.run([str(a) for a in argv], env=dict(env, **(extra_env or {})), text=True,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=timeout)
        if success is None:
            label = f'{label}-probe-{len(commands)}'
        (output / f'{label}.log').write_text(result.stdout)
        commands.append(dict(label=label, arguments=[str(a) for a in argv], exit_code=result.returncode,
                             started_ns=started, finished_ns=time.time_ns()))
        if success is not None:
            require((result.returncode == 0) == success, f'{label}: unexpected exit {result.returncode}; see {output}')
        return result

    def command(label, *arguments, success=True, extra_env=None):
        result = run(label, [binary, *arguments], success=success, extra_env=extra_env)
        return dict(dict(word.split('=', 1) for word in result.stdout.split() if '=' in word), exit_code=result.returncode)

    def admin(label, *arguments, success=True):
        # One follower-or-leader endpoint is enough: kv9-admin resolves the leader.
        result = run(label, [admin_bin, '--endpoints', addresses[1] + ',' + addresses[2], '--output', 'json', '--yes',
                             '--timeout', '20', *arguments], success=success)
        # kv9-admin prints its plan on stderr (merged here for the log); the JSON
        # document starts at the first brace.
        text = result.stdout[result.stdout.find('{'):] if '{' in result.stdout else ''
        try:
            return json.loads(text), result.returncode
        except json.JSONDecodeError:
            return {}, result.returncode

    def start(node, label, ticket=None):
        log = (output / f'{label}-{node}.log').open('w')
        handles.append(log)
        run_env = env if ticket is None else dict(env, KV9_JOIN_TICKET=ticket)
        process = subprocess.Popen([str(binary), 'start', '--node-id', str(node), '--addr', addresses[node],
                                    '--data-dir', str(output / f'n{node}')], env=run_env, stdout=log, stderr=subprocess.STDOUT)
        processes[node] = process

    def status(node):
        path = output / f'n{node}/status'
        if not path.exists() or node not in processes:
            return {}
        value = fields(path.read_text())
        return value if value.get('pid') == str(processes[node].pid) else {}

    def wait(label, predicate, seconds=120):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            value = predicate()
            if value:
                print(f'PASS: {label}', flush=True)
                return value
            time.sleep(.2)
        for node in list(processes):
            s = status(node)
            print(f'--- n{node}: role={s.get("role")} plans={s.get("replica_plans")} groups={s.get("data_groups")}', flush=True)
        raise RuntimeError(f'timed out: {label}; see {output}')

    def leader():
        return next((n for n in processes if status(n).get('role') == 'leader'
                     and status(n).get('endpoint_ready') == 'true'), None)

    def data_groups(node):
        return {g['region']: g for g in json.loads(status(node).get('data_groups', '[]'))}

    def join(node):
        planned, _ = admin(f'admit-{node}', 'node', 'admit', node, '--addr', addresses[node], '--ttl', '600')
        ticket = planned['result']['join_ticket']
        command(f'join-{node}', 'join', '--root', output / 'root.bin', '--node-id', node, '--data-dir', output / f'n{node}',
                extra_env={'KV9_JOIN_TICKET': ticket})
        start(node, 'joiner', ticket=ticket)
        wait(f'node {node} registered and ready', lambda: status(node).get('endpoint_ready') == 'true')

    def repl(label, lines, success=True):
        peers = ','.join(f'{n}={a}' for n, a in addresses.items())
        result = subprocess.run([str(repl_bin), '--addr', addresses[2], '--peers', peers, '--timeout-ms', '10000'],
                                input='\n'.join(lines) + '\n', env=env, text=True,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=120)
        (output / f'{label}.log').write_text(result.stdout)
        if success is not None:
            require((result.returncode == 0) == success, f'{label}: repl exit {result.returncode}; see {output}')
        return result.stdout

    try:
        guards, incarnations = [], []
        for node in (1, 2, 3):
            guard = socket.socket()
            guard.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            guard.bind(('127.0.0.1', args.base_port + node))
            guards.append(guard)
            prepared = command(f'prepare-{node}', 'store-prepare', '--node-id', node, '--data-dir', output / f'n{node}')
            incarnations.append(f"{node}={prepared['store_incarnation']}")
        command('root-create', 'root-create', '--output', output / 'root.bin',
                '--voters', ','.join(f'{n}@{addresses[n]}' for n in (1, 2, 3)),
                '--store-incarnations', ','.join(incarnations))
        for node in (1, 2, 3):
            command(f'init-{node}', 'init', '--root', output / 'root.bin', '--node-id', node, '--data-dir', output / f'n{node}')
        for guard in guards:
            guard.close()
        for node in (1, 2, 3):
            start(node, 'initial')
        wait('metadata leader', leader)
        wait('all endpoints ready', lambda: all(status(n).get('endpoint_ready') == 'true' for n in processes))

        created, _ = admin('group-create', 'group', 'create', '--voters', '1,2,3', '--label', 'addition-e2e')
        task, region = created['result']['creation_task'], created['result']['region_id']
        keyspace, _ = admin('keyspace-create', 'keyspace', 'create', '--name', 'moved', '--group', task)
        require(keyspace['result']['outcome'] == 'created', 'keyspace bound to the group')

        def group_view():
            described, _ = admin('describe', 'group', 'describe', task)
            return described.get('result', {})

        def group_leader_view():
            for view in group_view().get('replica_views', []):
                observed = view.get('observed') or {}
                if observed.get('role') == 'Leader' and observed.get('state') == 'active':
                    return observed
            return None

        wait('data group active behind a leader', group_leader_view, 120)
        # The public route for the bound keyspace is published a moment after the
        # group activates; probe until the first write lands.
        wait('keyspace routable', lambda: 'applied' in repl('probe-route', ['use moved', 'put k/0 zero'], success=None), 120)
        repl('write-initial', ['use moved', 'put k/1 one', 'put k/2 two', 'get k/1'])

        # ------------------------------------------------------------ add-replica to node 4
        join(4)
        planned, _ = admin('add-replica', 'group', 'add-replica', task, '--to', '4', '--label', 'add-4')
        require(planned['result']['outcome'] == 'planned', 'first plan is a mutation')
        again, _ = admin('add-replica-again', 'group', 'add-replica', task, '--to', '4', '--label', 'add-4')
        require(again['result']['outcome'] == 'confirmed' and again['result']['plan_task'] == planned['result']['plan_task'],
                'an identical plan confirms the same row')
        # A destination that already is an initial replica is refused by the intent planner.
        refused, code = admin('add-replica-initial', 'group', 'add-replica', task, '--to', '2', '--label', 'bad', success=None)
        require(code == 1 and 'error' in refused, 'an initial replica cannot be the destination')

        def four_voters():
            observed = group_leader_view()
            return observed is not None and sorted(observed.get('voters', [])) == [1, 2, 3, 4]
        wait('node 4 promoted to voter by the cluster', four_voters, 420)

        def stage_of(plan_task):
            return next((p['stage'] for p in group_view().get('plans', []) if p['task'] == plan_task), None)
        require(stage_of(planned['result']['plan_task']) == 'evidenced', 'the addition plan reached evidence')
        tasks, _ = admin('tasks', 'task', 'list', '--kind', '104')
        require(len(tasks['result']['tasks']) == 1, 'exactly one evidence row')
        repl('write-after-add', ['use moved', 'put k/3 three', 'get k/1', 'mget k/2 k/3'])

        # ------------------------------------------------------------ move-replica 1 -> 5
        join(5)
        # The source of a move must be a committed member: node 9 never was one.
        bad, code = admin('move-from-non-member', 'group', 'move-replica', task, '--from', '9', '--to', '5', '--label', 'bad', success=None)
        require(code == 1, 'the source of a move must be a committed member')
        moved, _ = admin('move-replica', 'group', 'move-replica', task, '--from', '1', '--to', '5', '--label', 'move-1-5')
        require(moved['result']['outcome'] == 'planned', 'the move plan is a mutation')

        def moved_voters():
            observed = group_leader_view()
            return observed is not None and sorted(observed.get('voters', [])) == [2, 3, 4, 5]
        wait('node 5 promoted and node 1 retired by the cluster', moved_voters, 480)
        require(stage_of(moved['result']['plan_task']) == 'decided', 'the move plan reached its removal decision')
        out = repl('read-after-move', ['use moved', 'get k/1', 'get k/2', 'get k/3', 'put k/4 four', 'get k/4'])
        require(all(v in out for v in ('one', 'two', 'three', 'four')), 'every key readable after the move')

        # Each party recorded only its own steps.
        steps = {}
        for node in processes:
            for event in json.loads(status(node).get('replica_plans', '{}')).get('events', []):
                steps.setdefault(event['step'], set()).add(node)
        require({'attach', 'plan_image', 'bind', 'capture', 'install', 'record_evidence', 'promote', 'decide_removal', 'remove'} <= set(steps),
                f'every step of the table ran: {sorted(steps)}')
        require(steps['install'] <= {4, 5} and steps['record_evidence'] <= {4, 5}, 'only destinations install and record')
        manifest_record['verdict'] = 'passed'
        print('PASS: replica addition and move driven by the cluster (add → voters 1,2,3,4; move 1→5 → voters 2,3,4,5; '
              f'steps by node: {{{", ".join(f"{k}: {sorted(v)}" for k, v in sorted(steps.items()))}}})', flush=True)
    except Exception as error:
        manifest_record['verdict'] = f'failed: {error}'
        raise
    finally:
        for node in list(processes):
            process = processes.pop(node)
            if process.poll() is None:
                process.kill()
            process.wait(timeout=10)
        for handle in handles:
            handle.close()
        (output / 'manifest.json').write_text(json.dumps(manifest_record, indent=2) + '\n')


if __name__ == '__main__':
    main()
