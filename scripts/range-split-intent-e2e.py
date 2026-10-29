#!/usr/bin/env python3
"""Range split, part 1: the committed split intent (docs/RANGE-SPLIT.md §2).

Three stores hold a data group with a bound keyspace under writes. `kv9-admin group
split <task> --at k/m` commits one intent and, in the same transaction, two
activated unbound children on the group's members. The refusal matrix holds (a
second intent for the parent, a divergent key under the same operation, a child as
parent), the exact resubmission confirms, the children are visible with no keyspace, and the
data stays readable throughout: the parent serves until the cluster, by itself,
seals it and publishes the children (parts 2–4), which then serve it.
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
    parser.add_argument('--base-port', type=int, default=27400)
    args = parser.parse_args()
    binary, admin_bin, repl_bin = args.bin.resolve(), args.admin_bin.resolve(), args.repl_bin.resolve()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ, KV9_CLUSTER_TOKEN='split-e2e-cluster',
               KV9_CLIENT_TOKENS='admin=split-e2e-client', KV9_CLIENT_TOKEN='split-e2e-client',
               KV9_ADMIN_TOKEN='split-e2e-client',
               KV9_BOOTSTRAP_TOKEN='split-e2e-bootstrap')
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

        created, _ = admin('group-create', 'group', 'create', '--voters', '1,2,3', '--label', 'split-e2e')
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

        # ------------------------------------------------------------ the intent
        before = group_view()
        require(before.get('splits') == [], f"no split before the verb: {before.get('splits')}")
        planned, _ = admin('split', 'group', 'split', task, '--at', 'k/m', '--label', 'split-e2e')
        r = planned['result']
        require(r['outcome'] == 'committed' and r['stage'] == 'intended', f'first split commits: {r}')
        intent_task, low, high = r['intent_task'], r['child_low'], r['child_high']
        require(low['creation_task'] != high['creation_task'] and low['region_id'] != high['region_id'], 'two distinct children')
        confirmed, _ = admin('split-again', 'group', 'split', task, '--at', 'k/m', '--label', 'split-e2e')
        require(confirmed['result']['outcome'] == 'confirmed' and confirmed['result']['intent_task'] == intent_task
                and confirmed['result']['child_low'] == low and confirmed['result']['child_high'] == high,
                f'the exact resubmission confirms the same rows: {confirmed["result"]}')

        # ------------------------------------------------------------ the refusals
        _, code = admin('split-divergent', 'group', 'split', task, '--at', 'k/n', '--label', 'split-e2e', success=False)
        require(code == 1, f'a divergent key under the same operation is a typed refusal (exit {code})')
        _, code = admin('split-second', 'group', 'split', task, '--at', 'k/n', '--label', 'another', success=False)
        require(code == 1, f'a second intent for the parent is a typed refusal (exit {code})')
        _, code = admin('split-child', 'group', 'split', str(low['creation_task']), '--at', 'k/c', success=False)
        require(code == 1, f'an unbound child is not a parent (exit {code})')
        _, code = admin('split-empty', 'group', 'split', task, '--at-hex', '', success=False)
        require(code == 2, f'an empty key is a usage error (exit {code})')

        # ------------------------------------------------------------ the shape
        view = group_view()
        splits = view.get('splits', [])
        require(len(splits) == 1 and splits[0]['task'] == intent_task and splits[0]['split_key'] == 'k/m'
                and splits[0]['stage'] == 'intended' and splits[0]['child_low'] == low and splits[0]['child_high'] == high,
                f'the parent row shows the intent: {splits}')
        listed, _ = admin('group-list', 'group', 'list')
        rows = {g['creation_task']: g for g in listed['result']}
        parent_members = sorted(m['node_id'] for m in rows[int(task)]['members'])
        for child in (low, high):
            row = rows[child['creation_task']]
            require(row['activated'] and row['keyspaces'] == [] and sorted(m['node_id'] for m in row['members']) == parent_members,
                    f'child {child} is activated, unbound and on the parent members: {row}')
        def children_serving():
            for child in (low, high):
                described, _ = admin('describe-child', 'group', 'describe', str(child['creation_task']), partial=True)
                views = described.get('result', {}).get('replica_views', [])
                if not any((v.get('observed') or {}).get('role') == 'Leader' for v in views):
                    return False
            return True
        wait('both children run as empty groups behind a leader', children_serving, 120)
        described, _ = admin('task-describe', 'task', 'describe', str(intent_task))
        require(described['result']['kind_name'] == 'split_intent', f"task {intent_task}: {described['result'].get('kind_name')}")

        # ------------------------------------------------------------ the data stays readable
        # The pipeline proceeds by itself: the parent serves until its seal, refuses with
        # the typed stale-epoch error during the window, and the children serve once
        # published. A reader that retries sees every key throughout.
        def readable():
            out = repl('read-after-intent', ['use split-me', 'get k/a', 'get k/m', 'get k/z', 'put k/zz zz', 'get k/zz'], success=None)
            if all(f'{c}' in out for c in ('a', 'm', 'z', 'zz')) and 'error' not in out.lower():
                return out
            require('stale region epoch' in out or 'error' not in out.lower(), f'unexpected refusal: {out[-300:]}')
            return None
        wait('the data is served through the parent or the children', readable, 300)
        wait('the split the intent started is published', lambda: group_view()['splits'][0]['stage'] == 'published', 300)
        wait('the data is served by the children', readable, 120)
        manifest_record['verdict'] = 'passed'
        print(f'PASS: range split intent (parent {task} region {region} split at k/m -> intent {intent_task}, children '
              f'{low["creation_task"]}/{high["creation_task"]} activated unbound on {parent_members}; refusals typed; data served throughout)', flush=True)
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
