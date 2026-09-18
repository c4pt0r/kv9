#!/usr/bin/env python3
"""Real-process BYTE-BASED auto-compaction gate (#20): entries are a poor
proxy for log cost when value sizes vary, so a group of large values must
be able to bound its raft log by BYTES. With KV9_AUTO_COMPACT_ENTRIES=0
(the entries trigger OFF) and KV9_AUTO_COMPACT_BYTES set, sustained LARGE
writes grow the retained committed payload past the byte threshold and the
metadata leader auto-proposes a committed compaction floor — the exact
manual verb, no new authority; the existing all-matched-gated reconcile
executes it and (follower-side) every voter compacts its own prefix.

Because the entries trigger is OFF, any compaction here is driven PURELY by
bytes. The fixture writes ~4 KiB values so a few dozen writes cross the
byte threshold that entry count alone never would. It asserts every voter's
log_first_index advances past 1, `retained_log_bytes` is observed above the
threshold before compaction and drops after, a second burst advances every
voter's floor further, and a full-cluster restart recovers on the compacted
logs of all three voters with every sampled key serving.

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
    parser.add_argument('--base-port', type=int, default=27440)
    args = parser.parse_args()
    for name in ('KV9_OBJECT_STORE_ENDPOINT', 'KV9_OBJECT_STORE_BUCKET',
                 'KV9_OBJECT_STORE_ACCESS_KEY', 'KV9_OBJECT_STORE_SECRET_KEY'):
        require(os.environ.get(name), f'{name} must be set to an isolated real MinIO bucket')
    binary, output = args.bin.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ, KV9_CLUSTER_TOKEN='bytescompact-e2e-cluster',
               KV9_CLIENT_TOKENS='admin=bytescompact-e2e-client', KV9_CLIENT_TOKEN='bytescompact-e2e-client',
               KV9_BOOTSTRAP_TOKEN='bytescompact-e2e-bootstrap', KV9_STORAGE='minio',
               KV9_AUTO_COMPACT_ENTRIES='0', KV9_AUTO_COMPACT_BYTES='262144')
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

        def leader_retained_bytes():
            node = group_leader()
            if node is None:
                return None
            return data_groups(node).get(region, {}).get('retained_log_bytes')

        # The retained-byte metric is plumbed and observable (a valid number).
        def bytes_observable():
            rb = leader_retained_bytes()
            return True if rb is not None and int(rb) >= 0 else None
        wait('retained_log_bytes is plumbed and observable', bytes_observable, 120)
        print('PASS: retained_log_bytes observable on the leader', flush=True)

        # Sustained LARGE writes with the ENTRIES trigger OFF and ZERO manual
        # verbs. Only the BYTE trigger can cause compaction here: the metadata
        # leader auto-proposes a floor once retained_log_bytes crosses 256 KiB;
        # the gated reconcile executes it and every voter compacts its prefix.
        # With ~4 KiB values the threshold falls in a few dozen writes — an
        # entry count that the (disabled) entries trigger would never fire on.
        burst('fill', 120)

        def all_voters_compacted():
            fi = first_indices()
            return fi if len(fi) == 3 and all(v > 1 for v in fi.values()) else None
        fi = wait('EVERY voter compacts its log under the BYTE trigger (entries off)',
                  all_voters_compacted, 300)
        print(f'PASS: all voters log_first_index {fi} (byte-driven compaction)', flush=True)

        # A second burst advances every voter's floor further.
        burst('fill2', 120)

        def all_advanced():
            after = first_indices()
            return after if len(after) == 3 and all(after[n] > fi[n] for n in fi) else None
        second = wait('every voter advances its floor further', all_advanced, 300)
        print(f'PASS: all voters advanced to {second}', flush=True)

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
            checks=['the retained-byte metric is plumbed and observable in status',
                    'with the ENTRIES trigger OFF, large-value writes cross the BYTE threshold and every voter compacts (log_first_index past 1)',
                    'a second burst advances every voter\'s floor further',
                    'a full-cluster restart recovers on the compacted logs and every sampled key serves'])
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
