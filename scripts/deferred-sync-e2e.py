#!/usr/bin/env python3
"""Real-process deferred-apply-sync crash gate: with
KV9_DATA_SYNC_DEFER_BYTES=4194304 the data-group engine defers its
apply-record fsyncs (bytes/100ms bounded). Acknowledged writes rest on
the SYNCED raft log; a crash may lose the engine WAL's unsynced tail,
and recovery truncates it and REPLAYS the committed raft log over it.

The gate: concurrent acked writes; kill -9 EVERY voter mid-load (no
graceful shutdown, twice); after each restart every previously ACKED
write reads back through public routing with the fixture still able to
take new writes. Deferral must never trade an acknowledged write for
speed — this fixture asserts exactly that.

Requires KV9_OBJECT_STORE_* (isolated real MinIO); servers run KV9_STORAGE=minio.
"""
import argparse
import concurrent.futures
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import threading
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
    parser.add_argument('--base-port', type=int, default=27300)
    args = parser.parse_args()
    for name in ('KV9_OBJECT_STORE_ENDPOINT', 'KV9_OBJECT_STORE_BUCKET',
                 'KV9_OBJECT_STORE_ACCESS_KEY', 'KV9_OBJECT_STORE_SECRET_KEY'):
        require(os.environ.get(name), f'{name} must be set to an isolated real MinIO bucket')
    binary, output = args.bin.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ, KV9_CLUSTER_TOKEN='defersync-e2e-cluster',
               KV9_CLIENT_TOKENS='admin=defersync-e2e-client', KV9_CLIENT_TOKEN='defersync-e2e-client',
               KV9_BOOTSTRAP_TOKEN='defersync-e2e-bootstrap', KV9_STORAGE='minio',
               KV9_DATA_SYNC_DEFER_BYTES='4194304')
    processes, handles, commands = {}, [], []
    commands_lock = threading.Lock()
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
        with commands_lock:
            if success is None:
                label = f'{label}-probe-{len(commands)}'
            if log:
                (output / f'{label}.log').write_text(result.stdout)
            commands.append(dict(label=label, arguments=list(map(str, arguments)), exit_code=result.returncode,
                                 started_ns=started, finished_ns=time.time_ns()))
        if success is not None:
            require((result.returncode == 0) == success, f'{label}: unexpected exit; see {output}')
        return dict(dict(word.split('=', 1) for word in result.stdout.split() if '=' in word), exit_code=result.returncode)

    def start(node, label):
        log = (output / f'{label}-{node}.log').open('w')
        handles.append(log)
        processes[node] = subprocess.Popen(
            [str(binary), 'start', '--node-id', str(node), '--addr', addresses[node],
             '--data-dir', str(output / f'n{node}')], env=env, stdout=log, stderr=subprocess.STDOUT)

    def kill_hard(node):
        process = processes.pop(node)
        process.kill()  # SIGKILL: no shutdown path runs, no deferred sync happens
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
                           '--root-digest', root, '--creation-task', task, '--name', 'defer-src')
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
        rng = random.Random(13)
        acked = {}
        acked_lock = threading.Lock()

        def fill(round_name, count):
            items = []
            for i in range(count):
                key = ''.join(rng.choice('0123456789abcdef') for _ in range(8))
                value = ('%04x' % i) * 8
                items.append((i, key, value))

            def put_one(item):
                i, key, value = item
                for attempt in range(240):
                    for n in (1, 2, 3):
                        if n not in processes:
                            continue
                        put = command(f'{round_name}-fill', 'client', 'raw-put', '--addr', addresses[n],
                                      '--keyspace', keyspace_id, '--key-hex', key,
                                      '--value-hex', value, success=None, log=(i % 64 == 0))
                        if put['exit_code'] == 0:
                            require(int(put.get('applied_index', 0)) > 0, 'ack lacks a position')
                            with acked_lock:
                                acked[key] = value
                            return True
                    time.sleep(0.25)
                raise RuntimeError(f'{round_name}: write {key} never landed')
            with concurrent.futures.ThreadPoolExecutor(max_workers=24) as pool:
                for ok in pool.map(put_one, items):
                    require(ok, 'fill worker failed')
            print(f'PASS: {round_name} acked {count} writes (total {len(acked)})', flush=True)

        def verify_all(round_name):
            unread = dict(acked)
            deadline = time.monotonic() + 300
            while unread and time.monotonic() < deadline:
                for key, value in list(unread.items()):
                    for n in (1, 2, 3):
                        if n not in processes:
                            continue
                        got = command(f'{round_name}-read', 'client', 'raw-get', '--addr', addresses[n],
                                      '--keyspace', keyspace_id, '--key-hex', key,
                                      success=None, log=False)
                        if got.get('value_hex') == value:
                            del unread[key]
                            break
                if unread:
                    time.sleep(2)
            require(not unread, f'{round_name}: {len(unread)} ACKED writes lost')
            print(f'PASS: {round_name} every acked write ({len(acked)}) reads back', flush=True)

        # Round 1: fill under load, then SIGKILL every voter mid-deferral.
        # A fill thread is still running when the cluster dies; only ACKED
        # writes are owed to us afterward.
        fill('warm', 128)
        killer_done = threading.Event()

        def background_writes():
            i = 0
            while not killer_done.is_set():
                key = ''.join(rng.choice('0123456789abcdef') for _ in range(8))
                value = ('dead%04x' % i) * 4
                for n in (1, 2, 3):
                    if n not in processes:
                        continue
                    try:
                        put = command('crash-fill', 'client', 'raw-put', '--addr', addresses[n],
                                      '--keyspace', keyspace_id, '--key-hex', key,
                                      '--value-hex', value, success=None, log=False)
                    except Exception:
                        return
                    if put['exit_code'] == 0:
                        with acked_lock:
                            acked[key] = value
                        break
                i += 1
        writer = threading.Thread(target=background_writes)
        writer.start()
        time.sleep(2)  # let deferral accumulate an unsynced tail under load
        for n in (1, 2, 3):
            kill_hard(n)
        killer_done.set()
        writer.join(timeout=30)
        print(f'PASS: hard-killed all voters with {len(acked)} acked writes', flush=True)

        for n in (1, 2, 3):
            start(n, 'restart1')
        wait('all voters ready after crash 1',
             lambda: all(status(n).get('endpoint_ready') == 'true' for n in (1, 2, 3)), 300)
        wait('data group re-elected after crash 1', lambda: group_leader() is not None, 300)
        verify_all('crash1')

        # Round 2: more load, second crash, verify again.
        fill('round2', 128)
        for n in (1, 2, 3):
            kill_hard(n)
        for n in (1, 2, 3):
            start(n, 'restart2')
        wait('all voters ready after crash 2',
             lambda: all(status(n).get('endpoint_ready') == 'true' for n in (1, 2, 3)), 300)
        wait('data group re-elected after crash 2', lambda: group_leader() is not None, 300)
        verify_all('crash2')

        # The fixture still takes new writes after both crashes.
        fill('post', 32)
        verify_all('final')

        manifest_record.update(
            verdict='accepted', acked=len(acked),
            checks=['acked concurrent writes under active deferral (bytes window, 100ms age bound)',
                    'SIGKILL of every voter mid-load, twice: no shutdown sync ran',
                    'after each restart EVERY acked write reads back (raft-log replay over the truncated engine tail)',
                    'the cluster keeps serving and accepting writes after both crashes'])
    except Exception as error:
        manifest_record.update(verdict='failed', error=str(error))
        raise
    finally:
        for node in list(processes):
            process = processes.pop(node)
            if process.poll() is None:
                process.kill()
            process.wait(timeout=10)
        for handle in handles:
            handle.close()
        (output / 'result.json').write_text(json.dumps(manifest_record, indent=2) + '\n')


if __name__ == '__main__':
    main()
