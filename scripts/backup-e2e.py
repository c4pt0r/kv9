#!/usr/bin/env python3
"""Backup of a keyspace (docs/BACKUP-RESTORE.md, increment 1).

Stores 1,2,3 hold the metadata group and a data group with the keyspace `orders`
(300 rows), split at r/00150 so the keyspace spans two ranges; stores 4 and 5
join as spares.

Phase A, with the range drivers paused (KV9_RANGE_DRIVER=0): `backup create`
commits an intent that holds both ranges; a split and a replica plan on a held
range are refused with the backup named; a second `backup create` confirms the
live one; `task abort` makes it history and frees the ranges.

Phase B, drivers on, with a crash armed after the metadata leader's `bind` step:
`backup create --wait` completes: one anchor per range with the exact row count
and a digest, one bundle object, every object GET-verified; the crash did not
change the outcome. `backup verify` passes; a replica plan on an anchored range
runs (the capture slot is free again); `backup retire` retires the backup and
`backup list` hides it unless `--all`.

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
    parser.add_argument('--base-port', type=int, default=27850)
    args = parser.parse_args()
    for name in ('KV9_OBJECT_STORE_ENDPOINT', 'KV9_OBJECT_STORE_BUCKET',
                 'KV9_OBJECT_STORE_ACCESS_KEY', 'KV9_OBJECT_STORE_SECRET_KEY'):
        require(os.environ.get(name), f'{name} must be set to an isolated real MinIO bucket')
    binary, admin_bin, repl_bin = args.bin.resolve(), args.admin_bin.resolve(), args.repl_bin.resolve()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ, KV9_CLUSTER_TOKEN='backup-e2e-cluster',
               KV9_CLIENT_TOKENS='admin=backup-e2e-client', KV9_CLIENT_TOKEN='backup-e2e-client',
               KV9_ADMIN_TOKEN='backup-e2e-client',
               KV9_BOOTSTRAP_TOKEN='backup-e2e-bootstrap', KV9_STORAGE='minio')
    processes, handles, commands = {}, [], []
    fault_dir = output / 'faults'
    fault_dir.mkdir(parents=True, exist_ok=True)
    node_env = {}
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
        result = run(label, [admin_bin, '--endpoints', addresses[1] + ',' + addresses[2] + ',' + addresses[3], '--output', 'json', '--yes',
                             '--timeout', '20', *arguments], success=success)
        text = result.stdout[result.stdout.find('{'):] if '{' in result.stdout else ''
        try:
            return json.loads(text), result.returncode
        except json.JSONDecodeError:
            return {}, result.returncode

    def start(node, label, ticket=None):
        log = (output / f'{label}-{node}.log').open('w')
        handles.append(log)
        run_env = dict(env, KV9_FAULT_DIR=str(fault_dir), KV9_FAULT_NODE=str(node), KV9_RECLAIM_RETIRED='1', **node_env)
        if ticket is not None:
            run_env['KV9_JOIN_TICKET'] = ticket
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

    def watchdog():
        for node, process in list(processes.items()):
            if process.poll() is not None:
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
                if 'unexpected exit' not in str(error):
                    raise
                value = None
            if value:
                print(f'PASS: {label}', flush=True)
                return value
            time.sleep(.2)
        for node in list(processes):
            s = status(node)
            print(f'--- n{node}: role={s.get("role")} backup={s.get("backup")} plans={s.get("replica_plans")}', flush=True)
        raise RuntimeError(f'timed out: {label}; see {output}')

    def leader():
        return next((n for n in processes if status(n).get('role') == 'leader'
                     and status(n).get('endpoint_ready') == 'true'), None)

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

    def backups():
        listed, _ = admin('backups', 'backup', 'list', '--all', success=None)
        return listed.get('result', []) if isinstance(listed.get('result'), list) else []

    def backup_row(task):
        return next((b for b in backups() if b['task'] == task), None)

    def arm(points):
        (fault_dir / 'fired.log').write_text('')
        (fault_dir / 'armed.json').write_text(json.dumps([{'point': p, 'action': a} for p, a in points]))

    def fired():
        try:
            return [l.split() for l in (fault_dir / 'fired.log').read_text().splitlines() if l.strip()]
        except FileNotFoundError:
            return []

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
        join(4)
        join(5)

        created, _ = admin('group-create', 'group', 'create', '--voters', '1,2,3', '--label', 'backup-e2e')
        task, region = created['result']['creation_task'], created['result']['region_id']
        keyspace, _ = admin('keyspace-create', 'keyspace', 'create', '--name', 'orders', '--group', task)
        require(keyspace['result']['outcome'] == 'created', 'keyspace bound to the group')
        keyspace_id = keyspace['result']['id']

        def group_view(t):
            described, _ = admin('describe', 'group', 'describe', t)
            return described.get('result', {})

        def group_leader_view(t):
            for view in group_view(t).get('replica_views', []):
                observed = view.get('observed') or {}
                if observed.get('role') == 'Leader' and observed.get('state') == 'active':
                    return observed
            return None

        wait('data group active behind a leader', lambda: group_leader_view(task), 120)
        wait('keyspace routable', lambda: 'applied' in repl('probe-route', ['use orders', 'put r/00000 zero'], success=None), 120)
        for batch in range(0, 300, 100):
            repl(f'fill-{batch}', ['use orders'] + [f'put r/{i:05d} value-{i}' for i in range(batch, batch + 100)])

        # Split at r/00150 so the keyspace spans two ranges (two anchors, one bundle).
        split, _ = admin('split', 'group', 'split', task, '--at', 'r/00150', '--label', 'two-ranges')
        split_task = split['result']['intent_task']

        def published():
            groups, _ = admin('groups', 'group', 'list', success=None)
            live = [g for g in groups.get('result', []) if any(not k['sealed'] for k in g.get('keyspaces', []))]
            return live if len(live) == 2 else None
        live = wait('split published: two live ranges', published, 300)
        low, high = sorted(live, key=lambda g: min(k['start'] for k in g['keyspaces']))
        low_task, high_task = low['creation_task'], high['creation_task']
        low_region, high_region = low['region_id'], high['region_id']
        wait('both ranges serve', lambda: 'value-299' in repl('probe-high', ['use orders', 'get r/00299', 'get r/00001'], success=None), 120)

        # ------------------------------------------------------------ phase A: a held backup excludes, confirms, aborts
        node_env['KV9_RANGE_DRIVER'] = '0'
        for node in (1, 2, 3, 4, 5):
            stop(node)
            start(node, 'paused')
        wait('cluster back with the range drivers paused', lambda: leader() and all(status(n).get('endpoint_ready') == 'true' for n in processes), 120)
        held, _ = admin('backup-held', 'backup', 'create', '--keyspace', 'orders', '--label', 'held')
        held_task = held['result']['task']
        require(held['result']['outcome'] == 'committed' and sorted(held['result']['regions']) == sorted([low_region, high_region]),
                f'A: the intent freezes both ranges: {held["result"]}')
        again, _ = admin('backup-again', 'backup', 'create', '--keyspace', str(keyspace_id), '--label', 'other')
        require(again['result']['outcome'] == 'confirmed' and again['result']['task'] == held_task, 'A: the live backup confirms')
        refused, code = admin('split-refused', 'group', 'split', low_task, '--at', 'r/00050', '--label', 'while-held', success=False)
        require(code == 1 and 'backup' in refused.get('error', {}).get('message', ''), f'A: split refused naming the backup: {refused}')
        refused, code = admin('plan-refused', 'group', 'add-replica', high_task, '--to', '4', '--label', 'while-held', success=False)
        require(code == 1 and 'backup' in refused.get('error', {}).get('message', ''), f'A: plan refused naming the backup: {refused}')
        row = backup_row(held_task)
        require(row and row['stage'] == 'intended' and all(r['anchor'] is None for r in row['ranges']), f'A: held at intended: {row}')
        aborted, _ = admin('abort-held', 'task', 'abort', held_task)
        require(aborted['result']['target'] == 'backup', f'A: abort targets the backup: {aborted}')
        row = backup_row(held_task)
        require(row and row['stage'] == 'aborted', f'A: aborted is history: {row}')
        freed, _ = admin('split-after-abort', 'group', 'split', low_task, '--at', 'r/00050', '--label', 'after-abort')
        admin('abort-split', 'task', 'abort', freed['result']['intent_task'])
        print('PASS: A: a held backup excludes splits and plans, confirms, and aborts into history', flush=True)

        # ------------------------------------------------------------ phase B: a complete backup, with one crash
        node_env.pop('KV9_RANGE_DRIVER')
        for node in (1, 2, 3, 4, 5):
            stop(node)
            start(node, 'running')
        wait('cluster back with the range drivers on', lambda: leader() and all(status(n).get('endpoint_ready') == 'true' for n in processes), 120)
        wait('both ranges serve again', lambda: 'value-299' in repl('probe-again', ['use orders', 'get r/00299'], success=None), 120)
        arm([('backup:bind:after', 'crash')])
        planned, _ = admin('backup-create', 'backup', 'create', '--keyspace', 'orders', '--label', 'nightly')
        backup_task = planned['result']['task']
        require(planned['result']['sequence'] == 1, f'B: the second sequence: {planned["result"]}')

        def complete():
            row = backup_row(backup_task)
            return row if row and row['stage'] == 'complete' else None
        row = wait('B: the backup completes despite a crash after bind', complete, 600)
        require(any(f[1] == 'backup:bind:after' for f in fired()), 'B: the bind crash fired')
        anchors = {r['region']: r['anchor'] for r in row['ranges']}
        require(set(anchors) == {low_region, high_region} and all(anchors.values()), f'B: one anchor per range: {anchors}')
        require(anchors[low_region]['rows'] == 150 and anchors[high_region]['rows'] == 150, f'B: rows per range: {anchors}')
        require(all(a['restorable'] and a['objects'] >= 1 and a['object_bytes'] > 0 for a in anchors.values()), f'B: anchors verified: {anchors}')
        require(row['restorable'] and row['bundle']['key'].startswith('backups/') and row['bundle']['bytes'] > 0, f'B: bundle: {row["bundle"]}')
        verified, _ = admin('verify', 'backup', 'verify', backup_task)
        require(verified['result']['ok'] and verified['result']['bundle_ok'] and len(verified['result']['ranges']) == 2, f'B: verify: {verified}')
        described, _ = admin('describe-backup', 'backup', 'describe', backup_task)
        require(described['result']['stage'] == 'complete', 'B: describe shows complete')
        for kind, expected in ((119, 2), (120, 2), (122, 1), (117, 2)):
            rows, _ = admin(f'kind-{kind}', 'task', 'list', '--kind', str(kind))
            require(len(rows['result']['tasks']) == expected, f'B: kind {kind}: {len(rows["result"]["tasks"])} rows, expected {expected}')
        arm([])
        print('PASS: B: complete backup with two anchors, a verified bundle and the committed rows', flush=True)

        # ------------------------------------------------------------ phase C: the slot is free again; retire
        plan, _ = admin('add-after-backup', 'group', 'add-replica', low_task, '--to', '4', '--label', 'after-backup')

        def four_voters():
            observed = group_leader_view(low_task)
            return observed is not None and sorted(observed.get('voters', [])) == [1, 2, 3, 4]
        wait('C: a replica plan runs on an anchored range (the capture slot is free)', four_voters, 600)
        retired, _ = admin('retire', 'backup', 'retire', backup_task)
        require(retired['result']['outcome'] == 'committed', 'C: retired')
        again, _ = admin('retire-again', 'backup', 'retire', backup_task)
        require(again['result']['outcome'] == 'confirmed', 'C: retire confirms')
        visible, _ = admin('list-default', 'backup', 'list')
        require(all(b['task'] != backup_task for b in visible['result']), 'C: retired hidden by default')
        row = backup_row(backup_task)
        require(row['stage'] == 'retired' and row['retired_unix'], 'C: retired with --all')
        refused, code = admin('abort-complete', 'task', 'abort', backup_task, success=False)
        require(code == 1, 'C: a complete backup does not abort')
        print('PASS: C: the anchored range took a plan; the backup retired; list hides it', flush=True)

        manifest_record['verdict'] = 'passed'
        print(f'PASS: backup (keyspace orders in 2 ranges; held backup {held_task} excluded a split and a plan then aborted; '
              f'backup {backup_task} completed with 150+150 rows anchored and a verified bundle despite a crash after bind; '
              f'retired; plan on the anchored range reached voters 1,2,3,4)', flush=True)
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
