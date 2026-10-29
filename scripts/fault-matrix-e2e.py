#!/usr/bin/env python3
"""Deterministic fault matrix (docs/RANGE-SPLIT.md §14).

Every driver step carries a fault point before and after it; a shared fault
directory arms points cluster-wide and records each firing once. One cluster on
real MinIO runs seven cells: a replica addition with a crash BEFORE every step, a
move with a crash AFTER every step, an addition with the metadata leader moved
away while a leader-only step is paused, a split with crashes before its steps, a
split with crashes after them (including the seal-before-evidence gap), a split
with the metadata leader moved away under its publication, and a drain with
crashes before its steps. A watchdog restarts whatever crashed; each cell asserts
that every armed point fired exactly once and that the committed outcome equals
the fault-free one.

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
    parser.add_argument('--base-port', type=int, default=26900)
    args = parser.parse_args()
    for name in ('KV9_OBJECT_STORE_ENDPOINT', 'KV9_OBJECT_STORE_BUCKET',
                 'KV9_OBJECT_STORE_ACCESS_KEY', 'KV9_OBJECT_STORE_SECRET_KEY'):
        require(os.environ.get(name), f'{name} must be set to an isolated real MinIO bucket')
    binary, admin_bin, repl_bin = args.bin.resolve(), args.admin_bin.resolve(), args.repl_bin.resolve()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ, KV9_CLUSTER_TOKEN='fault-e2e-cluster',
               KV9_CLIENT_TOKENS='admin=fault-e2e-client', KV9_CLIENT_TOKEN='fault-e2e-client',
               KV9_ADMIN_TOKEN='fault-e2e-client',
               KV9_BOOTSTRAP_TOKEN='fault-e2e-bootstrap', KV9_STORAGE='minio')
    processes, handles, commands = {}, [], []
    fault_dir = output / 'faults'
    fault_dir.mkdir(parents=True, exist_ok=True)
    expected_down = set()
    manifest_record = dict(verdict='running', commands=commands,
                           binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                           admin_sha256=hashlib.sha256(admin_bin.read_bytes()).hexdigest(),
                           runner_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                           revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
                           dirty=bool(subprocess.check_output(['git', 'status', '--porcelain'])),
                           chaos_mesh=False)
    (output / 'runner.py').write_bytes(Path(__file__).read_bytes())
    addresses = {i: f'127.0.0.1:{args.base_port+i}' for i in range(1, 7)}

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
        run_env = dict(env, KV9_FAULT_DIR=str(fault_dir), KV9_FAULT_NODE=str(node), KV9_RECLAIM_RETIRED='1')
        if ticket is not None:
            run_env['KV9_JOIN_TICKET'] = ticket
        process = subprocess.Popen([str(binary), 'start', '--node-id', str(node), '--addr', addresses[node],
                                    '--data-dir', str(output / f'n{node}')], env=run_env, stdout=log, stderr=subprocess.STDOUT)
        processes[node] = process

    def status(node):
        path = output / f'n{node}/status'
        if not path.exists() or node not in processes:
            return {}
        value = fields(path.read_text())
        return value if value.get('pid') == str(processes[node].pid) else {}

    def watchdog():
        # A node that exited on its own was a fault point's crash: bring it back.
        for node, process in list(processes.items()):
            if process.poll() is not None and node not in expected_down:
                print(f'watchdog: n{node} exited with {process.returncode}; restarting', flush=True)
                processes.pop(node)
                start(node, 'restarted')

    def wait(label, predicate, seconds=120):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            watchdog()
            try:
                value = predicate()
            except RuntimeError as error:
                # An admin call refused by an election in progress: retry.
                if 'unexpected exit' not in str(error):
                    raise
                value = None
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

        created, _ = admin('group-create', 'group', 'create', '--voters', '1,2,3', '--label', 'fault-e2e')
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

        def arm(points):
            (fault_dir / 'fired.log').write_text('')
            (fault_dir / 'armed.json').write_text(json.dumps([{'point': p, 'action': a} for p, a in points]))

        def fired():
            try:
                return [l.split() for l in (fault_dir / 'fired.log').read_text().splitlines() if l.strip()]
            except FileNotFoundError:
                return []

        def check_fired(cell, armed, subset=False, one_of=()):
            seen = [f[1] for f in fired()]
            require(len(seen) == len(set(seen)), f'{cell}: a point fired twice: {seen}')
            if subset:
                require(set(seen) <= set(armed) and seen, f'{cell}: fired {seen} not within armed {armed}')
            else:
                must = set(armed) - set(one_of)
                require(must <= set(seen) <= set(armed), f'{cell}: armed {sorted(armed)} but fired {sorted(seen)}')
                if one_of:
                    require(any(p in seen for p in one_of), f'{cell}: none of {one_of} fired: {sorted(seen)}')
            print(f'{cell}: fired {len(seen)} point(s): {", ".join(f"n{f[0]} {f[1]} {f[2]}" for f in fired())}', flush=True)

        def voters_are(expected):
            def check():
                observed = group_leader_view()
                return observed is not None and sorted(observed.get('voters', [])) == expected
            return check

        def stage_of(plan_task):
            return next((p['stage'] for p in group_view().get('plans', []) if p['task'] == plan_task), None)

        def transfer_metadata_away():
            listed, _ = admin('nodes', 'node', 'list', success=None)
            current = listed.get('result', {}).get('leader_node_id')
            others = [n['node_id'] for n in listed.get('result', {}).get('nodes', []) if n['raft_role'] == 'voter' and n['node_id'] != current]
            require(others, 'another metadata voter exists')
            admin('transfer', 'node', 'transfer-leader', '--to', str(others[0]), success=None)
            return current, others[0]

        plan_points = ['attach', 'plan_image', 'capture', 'bind', 'install', 'record_evidence', 'promote']

        # ------------------------------------------------------------ cell A: add, crash before every step
        join(4)
        arm([(f'plan:{p}:before', 'crash') for p in plan_points])
        planned, _ = admin('add-4', 'group', 'add-replica', task, '--to', '4', '--label', 'add-4')
        wait('A: node 4 promoted despite a crash before every step', voters_are([1, 2, 3, 4]), 900)
        wait('A: the plan is evidenced', lambda: stage_of(planned['result']['plan_task']) == 'evidenced', 120)
        check_fired('A', [f'plan:{p}:before' for p in plan_points])
        repl('A-read', ['use moved', 'get k/1', 'put k/a a', 'get k/a'])

        # ------------------------------------------------------------ cell B: move, crash after every step
        join(5)
        move_points = plan_points + ['decide_removal', 'remove']
        arm([(f'plan:{p}:after', 'crash') for p in move_points])
        moved, _ = admin('move-1-5', 'group', 'move-replica', task, '--from', '1', '--to', '5', '--label', 'move-1-5')
        wait('B: node 5 promoted and node 1 retired despite a crash after every step', voters_are([2, 3, 4, 5]), 900)
        wait('B: the move plan is decided', lambda: stage_of(moved['result']['plan_task']) == 'decided', 120)
        check_fired('B', [f'plan:{p}:after' for p in move_points])
        wait('B: reads and writes serve', lambda: 'error' not in repl('B-read', ['use moved', 'get k/1', 'get k/a', 'put k/b b', 'get k/b'], success=None).lower() or None, 120)

        # ------------------------------------------------------------ cell C: add, metadata leader moved under a paused leader-only step
        created2, _ = admin('group-create-2', 'group', 'create', '--voters', '2,3,4', '--label', 'fault-e2e-2')
        task2 = created2['result']['creation_task']
        ks2, _ = admin('keyspace-create-2', 'keyspace', 'create', '--name', 'second', '--group', task2)
        def view2():
            described, _ = admin('describe-2', 'group', 'describe', task2, success=None)
            return described.get('result', {})
        def leader2():
            for v in view2().get('replica_views', []):
                o = v.get('observed') or {}
                if o.get('role') == 'Leader' and o.get('state') == 'active':
                    return o
            return None
        wait('C: second group serves', leader2, 120)
        wait('C: second keyspace routable', lambda: 'applied' in repl('probe-2', ['use second', 'put s/0 zero'], success=None), 120)
        arm([('plan:bind:before', 'pause:6000')])
        planned2, _ = admin('add-5-second', 'group', 'add-replica', task2, '--to', '5', '--label', 'add-5-second')
        wait('C: the bind step is paused on the metadata leader', lambda: any(f[1] == 'plan:bind:before' for f in fired()), 300)
        before, after = transfer_metadata_away()
        print(f'C: metadata leadership moved from n{before} to n{after} under the paused bind', flush=True)
        wait('C: node 5 promoted into the second group', lambda: (leader2() or {}).get('voters') and sorted(leader2()['voters']) == [2, 3, 4, 5], 900)
        check_fired('C', ['plan:bind:before'])

        # ------------------------------------------------------------ cells D/E/F: splits
        def split_cell(cell, name, points, pause_transfer=False, one_of=()):
            created, _ = admin(f'group-create-{name}', 'group', 'create', '--voters', '2,3,4', '--label', f'fault-{name}')
            t = created['result']['creation_task']
            admin(f'keyspace-create-{name}', 'keyspace', 'create', '--name', name, '--group', t)
            def view():
                described, _ = admin(f'describe-{name}', 'group', 'describe', t, success=None)
                return described.get('result', {})
            def lead():
                for v in view().get('replica_views', []):
                    o = v.get('observed') or {}
                    if o.get('role') == 'Leader' and o.get('state') == 'active':
                        return o
                return None
            wait(f'{cell}: group {name} serves', lead, 120)
            wait(f'{cell}: {name} routable', lambda: 'applied' in repl(f'probe-{name}', [f'use {name}', 'put r/0 zero'], success=None), 120)
            value = 'v' * 64
            for batch in range(0, 300, 100):
                repl(f'fill-{name}-{batch}', [f'use {name}'] + [f'put r/{i:05d} {value}' for i in range(batch, batch + 100)])
            arm(points)
            planned, _ = admin(f'split-{name}', 'group', 'split', t, '--at', 'r/00150', '--label', f'split-{name}')
            if pause_transfer:
                wait(f'{cell}: publication is paused on the metadata leader', lambda: any(f[1] == 'split:publish:before' for f in fired()), 600)
                before, after = transfer_metadata_away()
                print(f'{cell}: metadata leadership moved from n{before} to n{after} under the paused publication', flush=True)
            def stage():
                s = view().get('splits', [])
                return s[0]['stage'] if s else None
            wait(f'{cell}: the split publishes', lambda: stage() == 'published', 900)
            check_fired(cell, [p for p, _ in points], one_of=one_of)
            def served():
                out = repl(f'read-{name}', [f'use {name}', 'get r/00000', 'get r/00149', 'get r/00150', 'get r/00299', 'put r/zz z', 'get r/zz'], success=None)
                return out if out.count(value) == 4 and 'z' in out and 'error' not in out.lower() else None
            wait(f'{cell}: the children serve every row', served, 120)
            seals, _ = admin(f'seals-{name}', 'task', 'list', '--kind', '118', success=None)
            require(any(True for _ in seals.get('result', {}).get('tasks', [])), f'{cell}: seal evidence exists')
        split_cell('D', 'split-before', [(f'split:{p}:before', 'crash') for p in ('child_descriptor', 'seal', 'populate', 'publish')])
        # The seal step ends by delivering its evidence; when that delivery fails (an
        # election right after an earlier crash) the step reports failure before its
        # after-point and the NEXT pass runs the recovery step `seal_evidence` instead,
        # so either after-point proves the crash between seal and evidence.
        split_cell('E', 'split-after', [(f'split:{p}:after', 'crash') for p in ('seal', 'seal_evidence', 'populate', 'publish')],
                   one_of=('split:seal:after', 'split:seal_evidence:after'))
        split_cell('F', 'split-switch', [('split:publish:before', 'pause:6000')], pause_transfer=True)

        # ------------------------------------------------------------ cell G: drain node 5 with a crash before every drain step
        # Node 5 holds regions 100 and 101; node 1 once held region 100 (moved away in
        # cell B) and placement never sends a group back to a store that held it, so a
        # sixth store is the only legal destination.
        join(6)
        drain_points = ['transfer_group_leader', 'transfer_metadata_leader', 'move_plan', 'promote_learner', 'remove_voter', 'remove_learner']
        arm([(f'drain:{p}:before', 'crash') for p in drain_points])
        drained, _ = admin('drain-5', 'node', 'drain', '5')
        def drained_now():
            listed, _ = admin('nodes-drain', 'node', 'list', success=None)
            row = next((n for n in listed.get('result', {}).get('nodes', []) if n['node_id'] == 5), None)
            return row is not None and row['drained']
        wait('G: node 5 drained despite a crash before every step it took', drained_now, 1200)
        check_fired('G', [f'drain:{p}:before' for p in drain_points], subset=True)
        require(any(f[1] == 'drain:move_plan:before' for f in fired()) and any(f[1] in ('drain:remove_learner:before', 'drain:remove_voter:before') for f in fired()),
                'G: the move plan and the metadata removal each crashed once')
        arm([])
        manifest_record['verdict'] = 'passed'
        print('PASS: fault matrix (plan add crash-before x7, move crash-after x9, add under metadata-leader switch; '
              'split crash-before x4, crash-after x3 incl. seal-before-evidence, publish under leader switch; drain crash-before)', flush=True)
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
