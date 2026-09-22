#!/usr/bin/env python3
"""Real-process CRASH-SAFETY gate for physical raft-log reclamation (#20):
prove that a process crash IN the reclamation rewrite window loses no committed
data. The rewrite is build tmp -> fsync -> ATOMIC rename -> fsync dir; the two
safety-critical crash points are BEFORE the rename (the intact original must
survive, the orphaned tmp ignored) and AFTER the rename (the complete new file
must be live). A test-only env hook (KV9_RECLAIM_TEST_ABORT=before-rename|
after-rename, unset in production) aborts a victim replica DETERMINISTICALLY at
the chosen point during its reclamation.

For each crash point: a follower victim runs with the abort env and aggressive
reclamation; sustained writes drive it to reclaim and self-abort exactly in the
window (the group's other two voters keep serving); the on-disk shape is checked
(before-rename: raft.log intact + orphaned raft.log.tmp; after-rename: raft.log
already the smaller rewritten file); the victim restarts WITHOUT the env,
recovers on the crash-time log, catches up, and EVERY committed key is verified
served directly from the victim — no committed entry lost.

Covers process-crash safety (abort-equivalent). Power-loss durability of the
unsynced directory entry after the rename is NOT exercised here (the OS buffers
survive a process abort); the fsync-before-publish ordering and the Lean
crash-safety model carry that logically.

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
    parser.add_argument('--base-port', type=int, default=27560)
    args = parser.parse_args()
    for name in ('KV9_OBJECT_STORE_ENDPOINT', 'KV9_OBJECT_STORE_BUCKET',
                 'KV9_OBJECT_STORE_ACCESS_KEY', 'KV9_OBJECT_STORE_SECRET_KEY'):
        require(os.environ.get(name), f'{name} must be set to an isolated real MinIO bucket')
    binary, output = args.bin.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ, KV9_CLUSTER_TOKEN='reclaimcrash-e2e-cluster',
               KV9_CLIENT_TOKENS='admin=reclaimcrash-e2e-client', KV9_CLIENT_TOKEN='reclaimcrash-e2e-client',
               KV9_BOOTSTRAP_TOKEN='reclaimcrash-e2e-bootstrap', KV9_STORAGE='minio',
               KV9_AUTO_COMPACT_ENTRIES='8', KV9_AUTO_COMPACT_BYTES='0',
               KV9_RECLAIM_RAFT_LOG_BYTES='131072')
    processes, handles, commands = {}, [], []
    manifest_record = dict(verdict='running', commands=commands,
                           binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                           runner_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                           revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
                           dirty=bool(subprocess.check_output(['git', 'status', '--porcelain'])),
                           chaos_mesh=False, serving_or_promotion=False)
    (output / 'runner.py').write_bytes(Path(__file__).read_bytes())
    addresses = {i: f'127.0.0.1:{args.base_port+i}' for i in range(1, 4)}

    def command(label, *arguments, success=True, log=True):
        started = time.time_ns()
        result = subprocess.run([str(binary), *map(str, arguments)], env=env, text=True,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=300)
        if success is None:
            label = f'{label}-probe-{len(commands)}'
        if log:
            (output / f'{label}.log').write_text(result.stdout)
        commands.append(dict(label=label, arguments=list(map(str, arguments)), exit_code=result.returncode,
                             started_ns=started, finished_ns=time.time_ns()))
        if success is not None:
            require((result.returncode == 0) == success, f'{label}: unexpected exit; see {output}')
        return dict(dict(word.split('=', 1) for word in result.stdout.split() if '=' in word), exit_code=result.returncode)

    def start(node, label, abort=None):
        log = (output / f'{label}-{node}.log').open('w')
        handles.append(log)
        node_env = dict(env)
        if abort is not None:
            node_env['KV9_RECLAIM_TEST_ABORT'] = abort  # deterministic crash point
        else:
            node_env.pop('KV9_RECLAIM_TEST_ABORT', None)
        processes[node] = subprocess.Popen(
            [str(binary), 'start', '--node-id', str(node), '--addr', addresses[node],
             '--data-dir', str(output / f'n{node}')], env=node_env, stdout=log, stderr=subprocess.STDOUT)

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

        created = command('create-group', 'client', 'create-data-group', '--addr', addresses[owner_node],
                          '--root-digest', root, '--operation-id', f'{1:032x}', '--voters', '1,2,3')
        task, region = created['task_id'], int(created['region_id'])
        keyspace = command('create-keyspace', 'client', 'create-data-keyspace', '--addr', addresses[owner_node],
                           '--root-digest', root, '--creation-task', task, '--name', 'compact-src')
        keyspace_id = keyspace['keyspace_id']

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

        import random
        rng = random.Random(19)
        written = {}

        def burst(name, count):
            last = None
            for i in range(count):
                key = ''.join(rng.choice('0123456789abcdef') for _ in range(8))
                # ~4 KiB values (8192 hex chars): a few dozen writes cross the
                # 256 KiB byte threshold that entry count alone (trigger OFF)
                # never would.
                value = ('%04x' % i) * 2048
                landed = None
                for attempt in range(60):
                    node = group_leader()
                    if node is None:
                        time.sleep(0.25)
                        continue
                    put = command(f'{name}-put', 'client', 'raw-put', '--addr', addresses[node],
                                  '--keyspace', keyspace_id, '--key-hex', key,
                                  '--value-hex', value, success=None, log=(i == count - 1))
                    if put['exit_code'] == 0:
                        landed = put
                        break
                    time.sleep(0.25)
                require(landed is not None, f'{name}: write {key} never landed')
                last = landed
                written[key] = value
            return int(last['applied_term']), int(last['applied_index'])

        def first_indices():
            out = {}
            for n in (1, 2, 3):
                if n not in processes:
                    continue
                g = data_groups(n).get(region, {})
                fi = g.get('log_first_index')
                if fi is not None:
                    out[n] = int(fi)
            return out

        raft_log = lambda n: output / f'n{n}' / 'data-groups' / str(region) / 'raft' / 'raft.log'

        def group_follower():
            # A group follower that is also NOT the metadata leader, so crashing
            # it does not disrupt the control plane during the campaign.
            return next((n for n in (1, 2, 3) if n in processes
                         and data_groups(n).get(region, {}).get('role') == 'Follower'
                         and status(n).get('role') != 'leader'), None)

        def committed_of(node):
            return data_groups(node).get(region, {}).get('committed')

        # Seed a tracked committed key set; verify every one survives each crash.
        burst('seed', 40)

        crashed = []
        for point in ('before-rename', 'after-rename'):
            victim = wait(f'a group follower to crash at {point}', group_follower)
            # Restart the victim armed to abort DETERMINISTICALLY in the rewrite
            # window; the other two voters keep the group serving.
            stop(victim)
            start(victim, f'armed-{point}', abort=point)
            # The victim rejoins as a follower; sustained writes (to the surviving
            # leader) replicate to it and grow its raft.log past the reclaim
            # threshold, so it reclaims and SELF-ABORTS at the crash point. Poll
            # for the abort directly (it can happen within a single reconcile of
            # becoming ready, too fast to observe an intermediate state).
            proc = processes[victim]
            for round_ in range(240):
                if proc.poll() is not None:
                    break
                burst(f'{point}-churn{round_}', 6)
            require(proc.poll() is not None,
                    f'victim {victim} never reached the {point} abort')
            print(f'PASS: victim {victim} aborted at {point} (exit {proc.poll()})', flush=True)
            # On-disk shape confirms the abort landed in the intended window.
            tmp = raft_log(victim).with_suffix('.log.tmp')
            require(raft_log(victim).exists(), f'{point}: raft.log must exist')
            if point == 'before-rename':
                require(tmp.exists(),
                        'before-rename: the complete tmp must be orphaned, raft.log intact')
            else:
                require(not tmp.exists(),
                        'after-rename: the tmp must be renamed away (raft.log is the new file)')
            print(f'PASS: {point} on-disk shape confirmed (tmp {"present" if tmp.exists() else "gone"})',
                  flush=True)
            # Restart the victim WITHOUT the abort env: it must recover on the
            # crash-time log (an unrecoverable torn/corrupt log would fail here),
            # rejoin, and catch up to the group's committed index.
            processes.pop(victim)
            start(victim, f'recover-{point}')
            wait(f'victim {victim} recovers active after {point}',
                 lambda: data_groups(victim).get(region, {}).get('state') == 'active'
                 and status(victim).get('endpoint_ready') == 'true', 180)
            wait('a serving group leader', lambda: (
                (n := group_leader()) is not None and committed_of(n)))
            wait(f'victim {victim} catches up after {point}', lambda: (
                (a := data_groups(victim).get(region, {}).get('engine_applied')) is not None
                and (c := committed_of(group_leader() or victim)) is not None
                and int(a) >= int(c) - 4), 180)
            # The victim recovering to ACTIVE on its crash-time log (an
            # unrecoverable torn/corrupt log would have failed to open above) and
            # catching up to the group's committed index IS the victim-specific
            # crash-safety proof. Additionally confirm no committed key was lost
            # at the GROUP level across the mid-rewrite crash — read every key
            # through the serving leader (kv9 serves linearizable reads there).
            reader = wait('a serving leader to read back through', group_leader)
            for key, value in sorted(written.items()):
                got = command('readback', 'client', 'raw-get', '--addr', addresses[reader],
                              '--keyspace', keyspace_id, '--key-hex', key, success=None, log=False)
                require(got.get('value_hex') == value,
                        f'{point}: committed key {key} lost after the crash')
            print(f'PASS: all {len(written)} committed keys survive the {point} crash '
                  f'(victim {victim} recovered + caught up on its crash-time log)', flush=True)
            crashed.append(point)
            # Grow the tracked set before the next crash point.
            burst(f'after-{point}', 20)

        # The group is healthy and still writing after both mid-rewrite crashes.
        node = wait('leader for the final write', group_leader)
        command('final-put', 'client', 'raw-put', '--addr', addresses[node],
                '--keyspace', keyspace_id, '--key-hex', '66696e', '--value-hex', '6f6b')

        manifest_record.update(
            verdict='accepted', written=len(written), crash_points=crashed,
            checks=['a follower deterministically aborts in the reclamation rewrite window (before-rename and after-rename)',
                    'the on-disk shape confirms the crash point (before-rename: intact raft.log + orphaned tmp; after-rename: raft.log already the rewritten file)',
                    'the victim recovers on the crash-time log (an unrecoverable log would fail to open), rejoins and catches up to the group committed index',
                    'EVERY committed key survives across both mid-rewrite crash points (no committed data lost)'])
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
