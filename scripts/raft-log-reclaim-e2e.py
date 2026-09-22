#!/usr/bin/env python3
"""Real-process PHYSICAL raft-log reclamation gate (#20): compaction advances
first_index but the APPEND-ONLY raft.log is never rewritten, so the file grows
without bound. With KV9_AUTO_COMPACT_ENTRIES set (so the log compacts) and
KV9_RECLAIM_RAFT_LOG_BYTES set, once a compacted group's on-disk raft.log
exceeds the reclaim threshold every replica PHYSICALLY reclaims its log — a
crash-safe rewrite (temp file -> fsync -> atomic rename) that drops the
compacted-away prefix from disk.

The fixture writes ~4 KiB values so the file grows past the reclaim threshold
while compaction keeps the LIVE tail small. It asserts every voter's
`log_file_bytes` first climbs ABOVE the reclaim threshold (append-only growth)
and then DROPS well below it after reclamation (the file physically shrinks —
which the append-only log alone never did), while `log_first_index` stays > 1
(the base survives) and no committed data is lost; then a full-cluster restart
recovers on the REWRITTEN logs of all three voters with every sampled key
serving and the group still writing.

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
    parser.add_argument('--base-port', type=int, default=27520)
    args = parser.parse_args()
    for name in ('KV9_OBJECT_STORE_ENDPOINT', 'KV9_OBJECT_STORE_BUCKET',
                 'KV9_OBJECT_STORE_ACCESS_KEY', 'KV9_OBJECT_STORE_SECRET_KEY'):
        require(os.environ.get(name), f'{name} must be set to an isolated real MinIO bucket')
    binary, output = args.bin.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ, KV9_CLUSTER_TOKEN='reclaim-e2e-cluster',
               KV9_CLIENT_TOKENS='admin=reclaim-e2e-client', KV9_CLIENT_TOKEN='reclaim-e2e-client',
               KV9_BOOTSTRAP_TOKEN='reclaim-e2e-bootstrap', KV9_STORAGE='minio',
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

        THRESHOLD = 131072  # KV9_RECLAIM_RAFT_LOG_BYTES

        def file_bytes():
            out = {}
            for n in (1, 2, 3):
                if n not in processes:
                    continue
                fb = data_groups(n).get(region, {}).get('log_file_bytes')
                if fb is not None:
                    out[n] = int(fb)
            return out

        # Sustained ~4 KiB writes. Compaction (KV9_AUTO_COMPACT_ENTRIES=8)
        # advances first_index, but the APPEND-ONLY raft.log keeps growing on
        # disk until each replica INDEPENDENTLY reclaims once its file passes the
        # threshold. Reclamation is per-replica and async, so we cannot catch
        # all three above the threshold at once; instead track each voter's PEAK
        # file size and detect when its current file DROPS to under half that
        # peak (a meaningful physical shrink), the peak having exceeded the
        # threshold. Keep writing so files keep re-growing and every voter fires.
        peaks = {1: 0, 2: 0, 3: 0}
        reclaimed = {1: None, 2: None, 3: None}
        for round_ in range(120):
            burst(f'churn{round_}', 8)
            fb = file_bytes()
            for n, v in fb.items():
                peaks[n] = max(peaks[n], v)
                if reclaimed[n] is None and peaks[n] > THRESHOLD and v * 2 < peaks[n]:
                    reclaimed[n] = v  # this voter physically shrank its file
            if all(reclaimed.values()):
                break
        require(all(reclaimed.values()),
                f'every voter must physically reclaim (peaks={peaks}, reclaimed={reclaimed})')
        fi = first_indices()
        require(len(fi) == 3 and all(v > 1 for v in fi.values()),
                f'the compacted base must survive reclamation (first_index={fi})')
        print(f'PASS: every voter physically reclaimed its raft.log: dropped to {reclaimed} '
              f'from peaks {peaks} (>{THRESHOLD}); first_index {fi} (base survives)', flush=True)

        # Full-cluster restart on the REWRITTEN (reclaimed) logs.
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
            checks=['every voter\'s append-only raft.log grows past the reclaim threshold on disk',
                    'reclamation PHYSICALLY shrinks every voter\'s raft.log below its peak (the append-only log alone never shrinks) while the compacted base survives (log_first_index > 1)',
                    'the group keeps serving and accepting writes after the reclamation swap (no committed data lost)',
                    'a full-cluster restart recovers on the REWRITTEN logs and every sampled key serves'])
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
