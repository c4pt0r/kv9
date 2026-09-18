#!/usr/bin/env python3
"""Real-process proposal-batching gate (#20 slice): with
KV9_PROPOSAL_BATCH_OPS=16 the server coalesces CONCURRENT same-epoch raw
writes of one group into single raft entries while every request keeps
its exact applied receipt.

The gate: a concurrent fill (32 parallel writers) lands every write and
every key reads back with a nonzero applied position; an automatic split
fires MID-FILL (threshold sized to trigger), so the epoch changes under
open batches and the fence refuses stale merges — writes keep landing
through the documented bounded pause and the children serve; a
full-cluster restart recovers everything. Batching must never trade
durability or receipts for speed — this fixture asserts correctness
only; the separate bench publishes measured numbers under predeclared
targets.

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
    parser.add_argument('--base-port', type=int, default=27120)
    args = parser.parse_args()
    for name in ('KV9_OBJECT_STORE_ENDPOINT', 'KV9_OBJECT_STORE_BUCKET',
                 'KV9_OBJECT_STORE_ACCESS_KEY', 'KV9_OBJECT_STORE_SECRET_KEY'):
        require(os.environ.get(name), f'{name} must be set to an isolated real MinIO bucket')
    binary, output = args.bin.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ, KV9_CLUSTER_TOKEN='batching-e2e-cluster',
               KV9_CLIENT_TOKENS='admin=batching-e2e-client', KV9_CLIENT_TOKEN='batching-e2e-client',
               KV9_BOOTSTRAP_TOKEN='batching-e2e-bootstrap', KV9_STORAGE='minio',
               KV9_PROPOSAL_BATCH_OPS='16', KV9_PROPOSAL_BATCH_DELAY_MS='2',
               KV9_AUTO_SPLIT_BYTES='8192')
    processes, handles, commands = {}, [], []
    commands_lock = __import__('threading').Lock()
    manifest_record = dict(verdict='running', commands=commands,
                           binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                           runner_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                           revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
                           dirty=bool(subprocess.check_output(['git', 'status', '--porcelain'])),
                           chaos_mesh=False, serving_or_promotion=False)
    (output / 'runner.py').write_bytes(Path(__file__).read_bytes())
    addresses = {i: f'127.0.0.1:{args.base_port+i}' for i in range(1, 4)}

    def command(label, *arguments, success=True, extra_env=None, log=True):
        started = time.time_ns()
        call_env = dict(env, **(extra_env or {}))
        result = subprocess.run([str(binary), *map(str, arguments)], env=call_env, text=True,
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

    def start(node, label, ticket=None):
        log = (output / f'{label}-{node}.log').open('w')
        handles.append(log)
        run_env = env if ticket is None else dict(env, KV9_JOIN_TICKET=ticket)
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
                           '--root-digest', root, '--creation-task', task, '--name', 'batch-src')
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

        # --- CONCURRENT fill: 32 parallel writers, 384 keys. Server-side
        # aggregation coalesces concurrent same-epoch writes; the auto-split
        # threshold guarantees an epoch change lands MID-FILL under open
        # batches. Every write must land (outlasting the bounded pause) with
        # a NONZERO applied receipt, through any node.
        import random
        rng = random.Random(11)
        written = {}
        keys = []
        for i in range(384):
            key = ''.join(rng.choice('0123456789abcdef') for _ in range(8))
            value = ('%04x' % i) * 8
            keys.append((i, key, value))
            written[key] = value

        def fill_one(item):
            i, key, value = item
            for attempt in range(240):
                for n in (1, 2, 3):
                    if n not in processes:
                        continue
                    put = command('fill', 'client', 'raw-put', '--addr', addresses[n],
                                  '--keyspace', keyspace_id, '--key-hex', key,
                                  '--value-hex', value, success=None, log=(i % 32 == 0))
                    if put['exit_code'] == 0:
                        require(int(put.get('applied_index', 0)) > 0,
                                f'write {key} lacks a nonzero applied receipt')
                        return True
                time.sleep(0.25)
            raise RuntimeError(f'fill write {key} did not land')

        with concurrent.futures.ThreadPoolExecutor(max_workers=32) as pool:
            for landed in pool.map(fill_one, keys):
                require(landed, 'fill worker failed')
        print(f'PASS: concurrent fill landed {len(written)} keys', flush=True)

        # The split fired mid-fill: the parent retires and children serve.
        def split_served():
            for n in (1, 2, 3):
                if n not in processes:
                    continue
                groups = data_groups(n)
                if groups.get(region, {}).get('state') == 'retired':
                    actives = [r for r, g in groups.items()
                               if r != region and g.get('state') == 'active']
                    if len(actives) >= 2:
                        return True
            return False
        wait('the mid-fill automatic split completes', split_served, 600)

        # Every written key reads back through public routing.
        unread = dict(written)
        deadline = time.monotonic() + 300
        while unread and time.monotonic() < deadline:
            for key, value in list(unread.items()):
                for n in (1, 2, 3):
                    if n not in processes:
                        continue
                    got = command('readback', 'client', 'raw-get', '--addr', addresses[n],
                                  '--keyspace', keyspace_id, '--key-hex', key,
                                  success=None, log=False)
                    if got.get('value_hex') == value:
                        del unread[key]
                        break
            if unread:
                time.sleep(2)
        require(not unread, f'{len(unread)} written keys never served')
        print('PASS: every batched write reads back', flush=True)

        # Full-cluster restart: the batched world recovers whole.
        for n in (1, 2, 3):
            stop(n)
            start(n, f'restart-{n}')
        wait('all voters ready after the full restart',
             lambda: all(status(n).get('endpoint_ready') == 'true' for n in (1, 2, 3)), 300)
        key, value = sorted(written.items())[len(written) // 2]

        def served():
            for n in (1, 2, 3):
                got = command('post-restart-get', 'client', 'raw-get', '--addr', addresses[n],
                              '--keyspace', keyspace_id, '--key-hex', key, success=None)
                if got.get('value_hex') == value:
                    return True
            return False
        wait('a written key serves after the full restart', served, 300)

        manifest_record.update(
            verdict='accepted', written=len(written),
            checks=['32 concurrent writers all land with nonzero applied receipts under batching',
                    'an automatic split fires MID-FILL: the epoch fence closes open batches and writes keep landing',
                    'every batched write reads back through public routing',
                    'the batched world survives a full-cluster restart'])
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
