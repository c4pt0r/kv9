#!/usr/bin/env python3
"""Split recovery under faults (docs/RANGE-SPLIT.md §13).

Three stores, a group with 3000 rows, population slowed to 1.5 s per page. During
the split the node leading the low child is KILLED in the middle of its copy; the
surviving replicas elect another leader, which copies the half again from its own
sealed parent replica (idempotent puts, the cursor was that node's memory), the
digests verify, the split publishes and every row is served. The killed node then
restarts, catches up as a follower of both children and retires its parent replica
on the committed sealed binding.
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
    parser.add_argument('--base-port', type=int, default=27800)
    args = parser.parse_args()
    binary, admin_bin, repl_bin = args.bin.resolve(), args.admin_bin.resolve(), args.repl_bin.resolve()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ, KV9_CLUSTER_TOKEN='recover-e2e-cluster',
               KV9_CLIENT_TOKENS='admin=recover-e2e-client', KV9_CLIENT_TOKEN='recover-e2e-client',
               KV9_ADMIN_TOKEN='recover-e2e-client',
               KV9_BOOTSTRAP_TOKEN='recover-e2e-bootstrap', KV9_RANGE_POPULATE_DELAY_MS='1500', KV9_RECLAIM_RETIRED='1')
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

        created, _ = admin('group-create', 'group', 'create', '--voters', '1,2,3', '--label', 'recover-e2e')
        task, region = created['result']['creation_task'], created['result']['region_id']
        keyspace, _ = admin('keyspace-create', 'keyspace', 'create', '--name', 'split-me', '--group', task)
        require(keyspace['result']['outcome'] == 'created', 'keyspace bound to the group')

        def group_view():
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
        wait('keyspace routable', lambda: 'applied' in repl('probe-route', ['use split-me', 'put k/0 zero'], success=None), 120)
        value = 'v' * 100
        for batch in range(0, 3000, 100):
            repl(f'fill-{batch}', ['use split-me'] + [f'put key/{i:05d} {value}' for i in range(batch, batch + 100)])

        def restart(node, label):
            process = processes.pop(node, None)
            if process is not None:
                if process.poll() is None:
                    process.kill()
                process.wait(timeout=10)
            start(node, label)

        def leader_of(creation_task):
            # Inside a wait: an election or an unreachable endpoint right after the
            # kill is a retry, not a failure.
            described, _ = admin('describe-leader', 'group', 'describe', str(creation_task), success=None)
            for v in described.get('result', {}).get('replica_views', []):
                o = v.get('observed') or {}
                if o.get('role') == 'Leader' and o.get('state') == 'active':
                    return v['node_id']
            return None

        def populations(node):
            try:
                return json.loads(status(node).get('range_split', '{}')).get('populations', {})
            except json.JSONDecodeError:
                return {}

        # ------------------------------------------------------------ the split under a fault
        planned, _ = admin('split', 'group', 'split', task, '--at', 'key/01500', '--label', 'recover-e2e')
        intent = planned['result']
        low = intent['child_low']
        wait('the low child serves behind a leader', lambda: leader_of(low['creation_task']) is not None, 120)
        victim = wait('the low child leader is copying its half',
                      lambda: next((n for n in processes if populations(n).get(str(low['region_id']), {}).get('rows_copied', 0) > 0), None), 180)
        copied_before = populations(victim)[str(low['region_id'])]['rows_copied']
        print(f'killing n{victim} after it copied {copied_before} rows of the low child', flush=True)
        processes.pop(victim).kill()
        wait('another replica leads the low child', lambda: leader_of(low['creation_task']) not in (None, victim), 120)
        wait('the split still publishes', lambda: group_view()['splits'][0]['stage'] == 'published', 600)
        evidence = {e['child_task']: e['rows'] for e in group_view()['splits'][0]['evidence']}
        # k/0 (the route probe) sorts below key/01500: 1501 rows low, 1500 high.
        require(evidence.get(low['creation_task']) == 1501 and evidence.get(intent['child_high']['creation_task']) == 1500,
                f'both halves verified whole after the kill: {evidence}')
        def served():
            out = repl('read-after', ['use split-me', 'get key/00000', 'get key/01499', 'get key/01500', 'get key/02999', 'put key/zz x', 'get key/zz'], success=None)
            return out if out.count(value) == 4 and 'x' in out and 'error' not in out.lower() else None
        wait('the children serve every row and new writes', served, 120)

        # ------------------------------------------------------------ the killed node returns
        restart(victim, 'restarted')
        wait(f'n{victim} is back and ready', lambda: status(victim).get('endpoint_ready') == 'true', 120)
        def rejoined():
            groups = {g['region']: g for g in json.loads(status(victim).get('data_groups', '[]'))}
            parent = groups.get(region, {})
            kids = [groups.get(intent['child_low']['region_id'], {}), groups.get(intent['child_high']['region_id'], {})]
            return parent.get('state') in ('retired', 'reclaimed') and all(k.get('state') == 'active' for k in kids)
        wait(f'n{victim} retired its parent replica and follows both children', rejoined, 180)
        manifest_record['verdict'] = 'passed'
        print(f'PASS: split recovery (n{victim} killed after copying {copied_before} rows; another leader recopied, '
              f'1501 + 1500 rows verified, split published, rows served; n{victim} restarted, parent retired, children followed)', flush=True)
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
