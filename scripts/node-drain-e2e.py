#!/usr/bin/env python3
"""Drain a node (docs/DRAIN-NODE.md).

Stores 1,2,3 hold the metadata group and a data group with a bound keyspace;
stores 4 and 5 join as metadata learners. Node 1 is made the leader of both the
data group and the metadata group, then `kv9-admin node drain 1 --wait`. The
cluster hands both leaderships away, moves the group's replica off node 1 (to
the least-loaded outsider, node 4), promotes one caught-up learner before it
removes node 1 from a three-voter metadata group, and reports `drained`; writes
and reads continue throughout. Nothing is deleted.

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
    parser.add_argument('--base-port', type=int, default=26990)
    args = parser.parse_args()
    for name in ('KV9_OBJECT_STORE_ENDPOINT', 'KV9_OBJECT_STORE_BUCKET',
                 'KV9_OBJECT_STORE_ACCESS_KEY', 'KV9_OBJECT_STORE_SECRET_KEY'):
        require(os.environ.get(name), f'{name} must be set to an isolated real MinIO bucket')
    binary, admin_bin, repl_bin = args.bin.resolve(), args.admin_bin.resolve(), args.repl_bin.resolve()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ, KV9_CLUSTER_TOKEN='drain-e2e-cluster',
               KV9_CLIENT_TOKENS='admin=drain-e2e-client', KV9_CLIENT_TOKEN='drain-e2e-client',
               KV9_ADMIN_TOKEN='drain-e2e-client',
               KV9_BOOTSTRAP_TOKEN='drain-e2e-bootstrap', KV9_STORAGE='minio',
               KV9_DRAIN_PARALLEL='1')
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

    def admin(label, *arguments, success=True, partial=False):
        # One follower-or-leader endpoint is enough: kv9-admin resolves the leader.
        # `partial` accepts exit 4 too: a fan-out that reached the leader but not
        # a node we killed on purpose.
        result = run(label, [admin_bin, '--endpoints', addresses[1] + ',' + addresses[2] + ',' + addresses[3],
                             '--output', 'json', '--yes', '--timeout', '20', *arguments],
                     success=None if partial else success)
        if partial:
            require(result.returncode in (0, 4), f'{label}: unexpected exit {result.returncode}; see {output}')
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

    def repl(label, lines, success=True, via=None):
        peers = ','.join(f'{n}={a}' for n, a in addresses.items())
        # Any live metadata member: the REPL follows the route and the leader hint
        # from there. A drained node is alive but no longer a member, so the
        # post-drain reads name one explicitly.
        result = subprocess.run([str(repl_bin), '--addr', addresses[via or min(processes)], '--peers', peers, '--timeout-ms', '10000'],
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

        created, _ = admin('group-create', 'group', 'create', '--voters', '1,2,3', '--label', 'drain-e2e')
        task, region = created['result']['creation_task'], created['result']['region_id']
        keyspace, _ = admin('keyspace-create', 'keyspace', 'create', '--name', 'moved', '--group', task)
        require(keyspace['result']['outcome'] == 'created', 'keyspace bound to the group')

        def group_view():
            # Used inside waits: the drained node may be the metadata leader and is
            # killed mid-drain, so an election window is a retry, not a failure.
            described, _ = admin('describe', 'group', 'describe', task, success=None)
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

        # ------------------------------------------------------------ the setup
        join(4)
        join(5)
        def ids(text):
            return {int(x) for x in text.split(',') if x.strip()}
        wait('nodes 4 and 5 are metadata learners',
             lambda: {4, 5} <= ids(status(leader()).get('meta_learners', '')) if leader() else False)
        # Node 1 leads both the data group and the metadata group, so the drain
        # must hand both away before anything can be removed.
        def transfer(label, *arguments):
            # A transfer is refused while the transferee trails by a write that is
            # still in flight; that is a retry, not a failure.
            for attempt in range(20):
                _, code = admin(f'{label}-{attempt}', *arguments, success=None)
                if code == 0:
                    return
                time.sleep(1)
            raise RuntimeError(f'{label}: leadership transfer kept being refused; see {output}')
        if leader() != 1:
            transfer('meta-leader-to-1', 'node', 'transfer-leader', '--to', '1', '--wait')
        wait('node 1 leads the metadata group', lambda: leader() == 1)
        def group_leader_id():
            for r in group_view().get('replica_views', []):
                if (r.get('observed') or {}).get('role') == 'Leader':
                    return r['node_id']
            return None
        if group_leader_id() != 1:
            transfer('group-leader-to-1', 'group', 'transfer-leader', task, '--to', '1', '--wait')
        wait('node 1 leads the data group', lambda: group_leader_id() == 1)
        listed, _ = admin('nodes-before', 'node', 'list')
        before = {n['node_id']: n for n in listed['result']['nodes']}
        require(not before[1]['draining'] and not before[1]['drained'], 'node 1 is not draining before the verb')

        # ------------------------------------------------------------ the drain, under a fault
        drained, _ = admin('drain-1', 'node', 'drain', '1', partial=True)
        require(drained['result']['outcome'] == 'committed', f"drain outcome {drained['result'].get('outcome')}")
        drain_task = drained['result']['task']
        # The drained node is KILLED once the cluster has committed its first move plan
        # and restarted a few seconds later: the plan completes without it (the source's
        # removal needs only the group's quorum), and on return it hands away nothing it
        # no longer leads, then leaves the metadata group as planned.
        wait('the cluster committed the move plan', lambda: any((p.get('source') or {}).get('node_id') == 1 for p in group_view().get('plans', [])), 240)
        print('killing the drained node n1 mid-drain', flush=True)
        processes.pop(1).kill()
        time.sleep(5)
        start(1, 'restarted')
        wait('n1 is back', lambda: status(1).get('endpoint_ready') == 'true' or status(1).get('pid'), 120)
        def drained_now():
            listed, _ = admin('nodes-poll', 'node', 'list', partial=True)
            row = next((n for n in listed.get('result', {}).get('nodes', []) if n['node_id'] == 1), None)
            return row is not None and row['drained']
        wait('node 1 is drained despite the restart', drained_now, 900)

        # ------------------------------------------------------------ the checks
        listed, _ = admin('nodes-after', 'node', 'list', partial=True)
        after = {n['node_id']: n for n in listed['result']['nodes']}
        require(after[1]['draining'] and after[1]['drained'], f"node 1 row after: {after[1]}")
        require(after[1]['raft_role'] == 'none', f"node 1 still {after[1]['raft_role']} of the metadata group")
        voters = sorted(n for n, row in after.items() if row['raft_role'] == 'voter')
        require(len(voters) == 3 and 1 not in voters and {2, 3} <= set(voters),
                f'metadata voters after the drain: {voters}')
        promoted = (set(voters) - {2, 3}).pop()
        view = group_view()
        members = sorted(m['node_id'] for m in view['members'])
        require(members == [2, 3, 4], f'group members after the drain: {members} (placement picks the least-loaded outsider, node 4)')
        plans = [p for p in view['plans'] if (p.get('source') or {}).get('node_id') == 1]
        require(len(plans) == 1 and plans[0]['mode'] == 'move' and plans[0]['destination']['node_id'] == 4
                and plans[0]['stage'] == 'decided', f'drain move plan: {plans}')
        live = group_leader_view()
        require(live is not None and sorted(live.get('voters', [])) == [2, 3, 4], f'live group voters: {live}')
        require(group_leader_id() != 1 and leader() != 1, 'node 1 leads nothing')
        out = repl('read-after-drain', ['use moved', 'get k/1', 'get k/2', 'put k/3 three', 'get k/3'], via=2)
        require(all(v in out for v in ('one', 'two', 'three')), 'every key readable and writable after the drain')
        described, _ = admin('task-describe', 'task', 'describe', str(drain_task))
        require(described['result']['kind_name'] == 'node_drain', f"task {drain_task}: {described['result'].get('kind_name')}")
        # The drained node still runs, as a follower of nothing: its own status says so.
        line = json.loads(status(1).get('node_drain', '{}'))
        require(line.get('this_store_draining') is True, f'node 1 status: {line}')
        steps = {}
        for node in processes:
            for e in json.loads(status(node).get('node_drain', '{}')).get('events', []):
                steps.setdefault(e['step'], set()).add(node)
        require('move_plan' in steps and 'promote_learner' in steps and 'remove_voter' in steps,
                f'the metadata leader took every step: {steps}')
        # Node 1 was restarted mid-drain: its own in-memory event log (the leadership
        # hand-offs) is gone; what matters is the committed outcome, checked above,
        # and that it leads nothing now.
        require(group_leader_id() != 1 and leader() != 1, 'node 1 leads nothing after the restart')
        manifest_record['verdict'] = 'passed'
        print(f'PASS: node drain (node 1 led both groups, was killed and restarted mid-drain; moved region to 4, promoted {promoted}, removed 1; '
              f'members {members}, metadata voters {voters}; steps: '
              f'{ {k: sorted(v) for k, v in sorted(steps.items())} })', flush=True)
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
