#!/usr/bin/env python3
"""Aborting long-running intents (docs/RANGE-SPLIT.md §12).

With the range drivers paused (KV9_RANGE_DRIVER=0) so nothing seals: a split intent
is aborted and becomes history (children retired and reclaimed, the parent free for
a new split and for a move); a replica plan is aborted before its image is planned
and the driver detaches the learner it had attached; a drain is aborted and the
same store can be drained again; a split on a group with a live plan is refused
until that plan is aborted. Nothing is deleted but abandoned empty children.
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
    parser.add_argument('--base-port', type=int, default=27700)
    args = parser.parse_args()
    binary, admin_bin, repl_bin = args.bin.resolve(), args.admin_bin.resolve(), args.repl_bin.resolve()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ, KV9_CLUSTER_TOKEN='abort-e2e-cluster',
               KV9_CLIENT_TOKENS='admin=abort-e2e-client', KV9_CLIENT_TOKEN='abort-e2e-client',
               KV9_ADMIN_TOKEN='abort-e2e-client',
               KV9_BOOTSTRAP_TOKEN='abort-e2e-bootstrap', KV9_RANGE_DRIVER='0', KV9_RECLAIM_RETIRED='1')
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

        created, _ = admin('group-create', 'group', 'create', '--voters', '1,2,3', '--label', 'abort-e2e')
        task, region = created['result']['creation_task'], created['result']['region_id']
        keyspace, _ = admin('keyspace-create', 'keyspace', 'create', '--name', 'split-me', '--group', task)
        require(keyspace['result']['outcome'] == 'created', 'keyspace bound to the group')

        def group_view():
            described, _ = admin('describe', 'group', 'describe', task, partial=True)
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
        wait('keyspace routable', lambda: 'applied' in repl('probe-route', ['use split-me', 'put k/0 zero'], success=None), 120)
        repl('write-initial', ['use split-me'] + [f'put k/{c} {c}' for c in 'abcdefghijklmnopqrstuvwxyz'] + ['get k/a'])

        def states(creation_task):
            described, _ = admin('describe-states', 'group', 'describe', str(creation_task), partial=True)
            return sorted((v.get('observed') or {}).get('state', 'missing') for v in described.get('result', {}).get('replica_views', []))
        def live_view(creation_task):
            described, _ = admin('describe-live', 'group', 'describe', str(creation_task), partial=True)
            for v in described.get('result', {}).get('replica_views', []):
                o = v.get('observed') or {}
                if o.get('role') == 'Leader' and o.get('state') == 'active':
                    return o
            return None

        # ------------------------------------------------------------ abort a split
        planned, _ = admin('split', 'group', 'split', task, '--at', 'k/m', '--label', 'abort-e2e')
        intent = planned['result']
        low, high = intent['child_low'], intent['child_high']
        wait('both children serve', lambda: live_view(low['creation_task']) and live_view(high['creation_task']), 120)
        time.sleep(3)
        require(group_view()['splits'][0]['stage'] == 'intended', 'the paused driver sealed nothing')
        aborted, _ = admin('abort-split', 'task', 'abort', str(intent['intent_task']))
        require(aborted['result']['target'] == 'split' and aborted['result']['outcome'] == 'committed', f"abort: {aborted['result']}")
        again, _ = admin('abort-split-again', 'task', 'abort', str(intent['intent_task']))
        require(again['result']['outcome'] == 'confirmed', 'a repeated abort confirms')
        require(group_view()['splits'][0]['stage'] == 'aborted', f"stage: {group_view()['splits']}")
        for child in (low, high):
            wait(f'abandoned child {child["creation_task"]} is retired and reclaimed', lambda c=child: states(c['creation_task']) == ['reclaimed'] * 3, 120)
        out = repl('read-after-abort', ['use split-me', 'get k/a', 'put k/zz zz', 'get k/zz'])
        require('a' in out and 'zz' in out, f'the parent keeps serving: {out}')
        second, _ = admin('split-again', 'group', 'split', task, '--at', 'k/n', '--label', 'second')
        require(second['result']['outcome'] == 'committed' and second['result']['intent_task'] != intent['intent_task'], 'the parent is free for a new split')
        admin('abort-second', 'task', 'abort', str(second['result']['intent_task']))

        # ------------------------------------------------------------ abort a replica plan
        join(4)
        plan, _ = admin('add-replica', 'group', 'add-replica', task, '--to', '4', '--label', 'abort-plan')
        plan_task = plan['result']['plan_task']
        wait('the driver attached the learner', lambda: 4 in (live_view(task) or {}).get('learners', []), 120)
        _, code = admin('split-while-plan', 'group', 'split', task, '--at', 'k/q', '--label', 'blocked', success=False)
        require(code == 1, f'a split is refused while a plan is live (exit {code})')
        aborted, _ = admin('abort-plan', 'task', 'abort', str(plan_task))
        require(aborted['result']['target'] == 'plan' and aborted['result']['path'] == 'migration abort (no image planned)', f"plan abort: {aborted['result']}")
        wait('the driver detached the learner', lambda: (live_view(task) or {}).get('learners') == [], 120)
        plans = [p for p in group_view()['plans'] if p['task'] == plan_task]
        require(plans and plans[0]['stage'] == 'aborted', f'plan stage: {plans}')
        third, _ = admin('split-after-plan-abort', 'group', 'split', task, '--at', 'k/q', '--label', 'third')
        require(third['result']['outcome'] == 'committed', 'the parent is free once the plan is aborted')
        admin('abort-third', 'task', 'abort', str(third['result']['intent_task']))

        # ------------------------------------------------------------ abort a drain
        drained, _ = admin('drain-4', 'node', 'drain', '4')
        drain_task = drained['result']['task']
        listed, _ = admin('nodes-draining', 'node', 'list')
        require(next(n for n in listed['result']['nodes'] if n['node_id'] == 4)['draining'] is True, 'node 4 draining')
        aborted, _ = admin('abort-drain', 'task', 'abort', str(drain_task))
        require(aborted['result']['target'] == 'drain', f"drain abort: {aborted['result']}")
        listed, _ = admin('nodes-undrained', 'node', 'list')
        require(next(n for n in listed['result']['nodes'] if n['node_id'] == 4)['draining'] is False, 'node 4 no longer draining')
        redrain, _ = admin('drain-4-again', 'node', 'drain', '4')
        require(redrain['result']['outcome'] == 'committed' and redrain['result']['task'] != drain_task, 'the same store drains again as a new row')
        admin('abort-redrain', 'task', 'abort', str(redrain['result']['task']))
        _, code = admin('abort-unknown', 'task', 'abort', '999999', success=False)
        require(code == 1, f'an unknown task is a typed refusal (exit {code})')
        listed, _ = admin('group-list-final', 'group', 'list')
        require(listed['partial'] == [], f'every catalog family reads back: {listed["partial"]}')
        manifest_record['verdict'] = 'passed'
        print(f'PASS: operation abort (split intent {intent["intent_task"]} aborted -> children reclaimed, parent free; '
              f'plan {plan_task} aborted -> learner detached; drain {drain_task} aborted -> redrained as {redrain["result"]["task"]}; '
              f'split refused while a plan is live)', flush=True)
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
