#!/usr/bin/env python3
"""Real-process AUTOMATIC compaction gate (#20): with KV9_AUTO_COMPACT_ENTRIES
set and ZERO manual verbs, sustained writes grow the raft log past the
threshold and the metadata leader auto-proposes a committed compaction
floor; the existing gated reconcile executes it. Correctness is the
group-compaction model's — this fixture only proves the SELF-DRIVING
trigger. Placeholder header (original manual-gate doc follows):

Real-process healthy-group compaction gate: bounded raft-log growth
for ordinary data groups, no migration involved. A committed kind-110
floor (strictly increasing per region) authorizes compaction; v1
EXECUTES ONLY AT THE GROUP LEADER through the all-matched-gated seam —
committed means a QUORUM holds the entries, not every voter, so
compacting any replica whose prefix a minority voter still needs would
strand it. Follower log bounding is the documented open edge.

The gate: after a write burst and a committed floor, the group leader's
raft directory shrinks (all-matched verified); a SECOND higher floor
after more writes compacts again; a stale floor refuses; a full-cluster
restart recovers on the compacted log (the startup history gates accept
the durable compacted base) and the group keeps serving and writing.

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
    parser.add_argument('--base-port', type=int, default=27380)
    args = parser.parse_args()
    for name in ('KV9_OBJECT_STORE_ENDPOINT', 'KV9_OBJECT_STORE_BUCKET',
                 'KV9_OBJECT_STORE_ACCESS_KEY', 'KV9_OBJECT_STORE_SECRET_KEY'):
        require(os.environ.get(name), f'{name} must be set to an isolated real MinIO bucket')
    binary, output = args.bin.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ, KV9_CLUSTER_TOKEN='autocompact-e2e-cluster',
               KV9_CLIENT_TOKENS='admin=autocompact-e2e-client', KV9_CLIENT_TOKEN='autocompact-e2e-client',
               KV9_BOOTSTRAP_TOKEN='autocompact-e2e-bootstrap', KV9_STORAGE='minio', KV9_AUTO_COMPACT_ENTRIES='64')
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

    def start(node, label):
        log = (output / f'{label}-{node}.log').open('w')
        handles.append(log)
        processes[node] = subprocess.Popen(
            [str(binary), 'start', '--node-id', str(node), '--addr', addresses[node],
             '--data-dir', str(output / f'n{node}')], env=env, stdout=log, stderr=subprocess.STDOUT)

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
            node = wait(f'{name} leader', group_leader)
            last = None
            for i in range(count):
                key = ''.join(rng.choice('0123456789abcdef') for _ in range(8))
                value = ('%04x' % i) * 16
                last = command(f'{name}-put', 'client', 'raw-put', '--addr', addresses[node],
                               '--keyspace', keyspace_id, '--key-hex', key,
                               '--value-hex', value, success=None, log=(i == count - 1))
                if last['exit_code'] != 0:
                    node = wait(f'{name} leader refresh', group_leader)
                    last = command(f'{name}-put-retry', 'client', 'raw-put', '--addr', addresses[node],
                                   '--keyspace', keyspace_id, '--key-hex', key,
                                   '--value-hex', value)
                written[key] = value
            return int(last['applied_term']), int(last['applied_index'])

        def leader_first_index():
            lead = group_leader()
            if lead is None:
                return None
            g = data_groups(lead).get(region, {})
            return g.get('log_first_index')

        # Sustained writes with ZERO compaction verbs. The metadata leader
        # observes the growing log and auto-proposes a floor; the gated
        # reconcile executes it, advancing the leader's first index past 1.
        burst('fill', 400)

        def auto_compacted():
            first = leader_first_index()
            return first is not None and int(first) > 1
        first = wait('the log auto-compacts with no manual verb', auto_compacted, 300)
        print(f'PASS: auto compaction advanced log_first_index to {first}', flush=True)

        # A SECOND burst pushes the log past the threshold again; the floor
        # auto-advances further (strictly increasing).
        burst('fill2', 400)

        def advanced_further():
            f = leader_first_index()
            return f is not None and int(f) > int(first)
        second = wait('a second automatic floor advances further', advanced_further, 300)
        print(f'PASS: second automatic floor advanced to {second}', flush=True)

        # Full-cluster restart on the compacted logs.
        for n in (1, 2, 3):
            stop(n)
        for n in (1, 2, 3):
            start(n, f'restart-{n}')
        wait('all voters ready after the compacted restart',
             lambda: all(status(n).get('endpoint_ready') == 'true' for n in (1, 2, 3)), 300)
        wait('group re-elected after the compacted restart', lambda: group_leader() is not None, 300)
        sample = sorted(written.items())
        for key, value in [sample[0], sample[len(sample) // 2], sample[-1]]:
            def served():
                for n in (1, 2, 3):
                    got = command('readback', 'client', 'raw-get', '--addr', addresses[n],
                                  '--keyspace', keyspace_id, '--key-hex', key, success=None, log=False)
                    if got.get('value_hex') == value:
                        return True
                return False
            wait(f'key {key} serves after the compacted restart', served, 180)
        node = wait('leader for the final write', group_leader)
        command('final-put', 'client', 'raw-put', '--addr', addresses[node],
                '--keyspace', keyspace_id, '--key-hex', '66696e', '--value-hex', '6f6b')

        manifest_record.update(
            verdict='accepted', written=len(written),
            checks=['sustained writes auto-propose a committed compaction floor with NO manual verb',
                    'the gated reconcile executes it: the leader log first-index advances past 1',
                    'a second burst auto-advances the floor further (strictly increasing)',
                    'a full-cluster restart recovers on the auto-compacted logs and every sampled key serves'])
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
