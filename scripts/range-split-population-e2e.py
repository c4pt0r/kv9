#!/usr/bin/env python3
"""Range split, parts 2 and 3: seal and population (docs/RANGE-SPLIT.md §3–§4).

Three stores hold a data group with a bound keyspace holding k/a..k/z. After
`kv9-admin group split <task> --at k/m` the cluster, by itself, installs the
children's half descriptors, seals the parent once both children serve, copies
each half from the local sealed parent replica into its child, verifies the copy
row for row and commits one evidence row per child: the split reaches stage
`populated`. The parent then refuses writes with the typed stale-epoch error and
still serves reads; nothing is rerouted (publication is part 4).
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
    parser.add_argument('--base-port', type=int, default=27500)
    args = parser.parse_args()
    binary, admin_bin, repl_bin = args.bin.resolve(), args.admin_bin.resolve(), args.repl_bin.resolve()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ, KV9_CLUSTER_TOKEN='split2-e2e-cluster',
               KV9_CLIENT_TOKENS='admin=split2-e2e-client', KV9_CLIENT_TOKEN='split2-e2e-client',
               KV9_ADMIN_TOKEN='split2-e2e-client',
               KV9_BOOTSTRAP_TOKEN='split2-e2e-bootstrap')
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

        # ------------------------------------------------------------ seal + population
        def stage():
            s = group_view().get('splits', [])
            return s[0]['stage'] if s else None
        wait('the split reaches stage populating or populated', lambda: stage() in ('populating', 'populated'), 180)
        wait('the split reaches stage populated', lambda: stage() == 'populated', 180)
        view = group_view()
        evidence = sorted(view['splits'][0]['evidence'], key=lambda e: e['child_task'])
        rows = {e['child_task']: e['rows'] for e in evidence}
        # k/0 (the route probe) and k/a..k/l are below k/m: 13 rows; k/m..k/z: 14.
        require(rows == {low['creation_task']: 13, high['creation_task']: 14},
                f'the low child holds k/0,k/a..k/l (13 rows) and the high child k/m..k/z (14): {rows}')
        require(len({e['digest'] for e in evidence}) == 2 and all(len(e['digest']) == 64 for e in evidence), f'two digests: {evidence}')
        listed, _ = admin('task-list-114', 'task', 'list', '--kind', '114')
        require(len(listed['result']['tasks']) == 2, f"two kind-114 rows: {listed['result']}")

        # ------------------------------------------------------------ the parent is sealed
        # Between the seal and the publication (part 4) the parent refuses reads and
        # writes alike with the typed stale-epoch error: the route still names it and
        # nothing else is routable yet. That window is the price of parts 2–3 alone.
        out = repl('read-after-seal', ['use split-me', 'get k/a'], success=None)
        require('stale region epoch' in out and 'applied' not in out, f'the sealed parent refuses with the typed error: {out}')
        out = repl('write-after-seal', ['use split-me', 'put k/new x'], success=None)
        require('stale region epoch' in out and 'applied' not in out, f'a write to the sealed parent is refused: {out}')
        live = group_leader_view()
        require(live is not None and sorted(live.get('voters', [])) == [1, 2, 3], f'parent voters unchanged: {live}')

        # The steps happened where their roles are: the seal at the parent leader, the
        # copies at the child leaders, the descriptors at the child leaders.
        steps = {}
        for node in processes:
            for e in json.loads(status(node).get('range_split', '{}')).get('events', []):
                steps.setdefault(e['step'], set()).add(node)
        require({'child_descriptor', 'seal', 'populate'} <= set(steps), f'every step recorded: {steps}')
        manifest_record['verdict'] = 'passed'
        print(f'PASS: range split population (parent {task} split at k/m -> children {low["creation_task"]}/{high["creation_task"]} '
              f'hold 13 + 14 rows with verified digests; parent sealed (typed refusal until publication); steps: '
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
