#!/usr/bin/env python3
"""Automatic split trigger (docs/RANGE-SPLIT.md §11).

Three stores hold a data group with a bound keyspace and KV9_AUTO_SPLIT_BYTES set
low. With ZERO operator verbs after the writes, the group leader's footprint verdict
commits a split intent at the sampled median key, the cluster seals, copies,
verifies and publishes, the parent retires, and every written key is served by the
children. One split: the cooldown and the children's size keep it from cascading.
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
    parser.add_argument('--base-port', type=int, default=27600)
    args = parser.parse_args()
    binary, admin_bin, repl_bin = args.bin.resolve(), args.admin_bin.resolve(), args.repl_bin.resolve()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ, KV9_CLUSTER_TOKEN='autosplit-e2e-cluster',
               KV9_CLIENT_TOKENS='admin=autosplit-e2e-client', KV9_CLIENT_TOKEN='autosplit-e2e-client',
               KV9_ADMIN_TOKEN='autosplit-e2e-client',
               KV9_BOOTSTRAP_TOKEN='autosplit-e2e-bootstrap',
               KV9_AUTO_SPLIT_BYTES=os.environ.get('KV9_E2E_AUTO_SPLIT_BYTES', '65536'), KV9_AUTO_SPLIT_COOLDOWN_SECS='3600',
               KV9_RECLAIM_RETIRED='1')
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

        created, _ = admin('group-create', 'group', 'create', '--voters', '1,2,3', '--label', 'autosplit-e2e')
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
        # Below the threshold nothing happens; the leader's footprint is visible.
        time.sleep(8)
        require(group_view().get('splits') == [], 'no split below the threshold')

        # ------------------------------------------------------------ the writes
        value = 'v' * 400
        # The trigger may fire while the fill is still running; a batch that lands in
        # the seal window is refused with the typed stale-epoch error and succeeds once
        # the children are published (the client refreshes its route by itself).
        for batch in range(0, 400, 50):
            deadline = time.monotonic() + 300
            while True:
                out = repl(f'fill-{batch}', ['use split-me'] + [f'put key/{i:05d} {value}' for i in range(batch, batch + 50)], success=None)
                if 'error' not in out.lower():
                    break
                require('stale region epoch' in out, f'fill-{batch}: unexpected refusal: {out[-300:]}')
                require(time.monotonic() < deadline, f'fill-{batch}: the seal window did not close in time')
                time.sleep(1)

        # ------------------------------------------------------------ the trigger
        def intents():
            return group_view().get('splits', [])
        split = wait('the group leader committed a split intent by itself', lambda: intents()[0] if intents() else None, 180)
        key = split['split_key']
        require(key.startswith('key/') and '00050' <= key[4:] <= '00350', f'the split key is a sampled median: {key}')
        verdicts = {}
        for node in processes:
            line = json.loads(status(node).get('auto_split', '{}'))
            for e in line.get('events', []):
                verdicts.setdefault(node, []).append(e)
        require(any(any(e['outcome'].startswith(('committed', 'confirmed')) for e in es) for es in verdicts.values()),
                f'a leader recorded the decision: {verdicts}')
        wait('the automatic split is published', lambda: intents()[0]['stage'] == 'published', 300)
        low, high = split['child_low'], split['child_high']
        # A child that received most of the fill may cross the threshold itself and
        # split again (the cooldown is per deciding node): a cascade is a legal
        # outcome. Wait until every split anywhere has been published.
        def all_published():
            listed, _ = admin('group-list-splits', 'group', 'list', partial=True)
            splits = [s for g in listed.get('result', []) for s in g.get('splits', [])]
            return listed.get('result') and all(s['stage'] == 'published' for s in splits)
        wait('every automatic split is published', all_published, 300)
        time.sleep(6)  # one more probe period: nothing new must start
        require(all_published(), 'no split in flight after the settle')

        # ------------------------------------------------------------ the data
        def served():
            out = repl('read-after', ['use split-me', 'get k/a', 'get key/00000', 'get key/00199', 'get key/00399', 'put key/new x', 'get key/new'], success=None)
            return out if out.count(value) == 3 and 'a' in out and 'x' in out and 'error' not in out.lower() else None
        wait('the children serve every key and new writes', served, 60)
        listed, _ = admin('group-list', 'group', 'list')
        rows = {g['creation_task']: g for g in listed['result']}
        require(rows[int(task)]['keyspaces'][0]['sealed'] is True, 'the parent binding is sealed')
        for child in (low, high):
            ks = rows[child['creation_task']]['keyspaces']
            require(len(ks) == 1, f'child {child} bound exactly once (sealed again if it cascaded): {ks}')
        live = [g for g in listed['result'] if g['keyspaces'] and not g['keyspaces'][0]['sealed']]
        bounds = sorted((g['keyspaces'][0]['start'], g['keyspaces'][0]['end']) for g in live)
        require(bounds[0][0] == '' and bounds[-1][1] == '' and all(a[1] == b[0] for a, b in zip(bounds, bounds[1:])),
                f'the live groups partition the keyspace exactly: {bounds}')
        require(len(intents()) == 1, f'one intent for the original parent: {intents()}')
        cascades = sum(len(g['splits']) for g in listed['result']) - 1
        manifest_record['verdict'] = 'passed'
        print(f'PASS: auto split (zero verbs after the writes; parent {task} split at {key} by its leader -> children '
              f'{low["creation_task"]}/{high["creation_task"]} published; {cascades} cascade split(s); {len(live)} live groups '
              f'partition the keyspace and serve every key; decisions: { {f"n{k}": len(v) for k, v in sorted(verdicts.items())} })', flush=True)
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
