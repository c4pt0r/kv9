#!/usr/bin/env python3
"""Real-process metadata-admission-floor gate (#20 item 4): a sustained
raw-write flood beyond the shared capacity must never starve control
traffic. KV9_PUBLIC_METADATA_RESERVED (default 8) keeps the last slots
of KV9_PUBLIC_MAX_REQUESTS admissible only by the metadata classes; the
flood receives the TYPED pre-append refusal (metadata_floor) instead of
consuming them.

The gate: with a 40-writer flood running for the whole window against a
small shared pool, an idempotent metadata write (create-data-group
confirm) succeeds on EVERY probe throughout; afterwards the status
counters show floor refusals for raw writes and ZERO floor refusals for
the metadata classes; the flood's acked writes read back.

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
    parser.add_argument('--base-port', type=int, default=27320)
    args = parser.parse_args()
    for name in ('KV9_OBJECT_STORE_ENDPOINT', 'KV9_OBJECT_STORE_BUCKET',
                 'KV9_OBJECT_STORE_ACCESS_KEY', 'KV9_OBJECT_STORE_SECRET_KEY'):
        require(os.environ.get(name), f'{name} must be set to an isolated real MinIO bucket')
    binary, output = args.bin.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    # A SMALL shared pool so a 40-subprocess flood genuinely saturates it
    # (CLI process spawn dilutes concurrency): 12 total, 8 reserved -> 4
    # shared slots.
    env = dict(os.environ, KV9_CLUSTER_TOKEN='floor-e2e-cluster',
               KV9_CLIENT_TOKENS='admin=floor-e2e-client', KV9_CLIENT_TOKEN='floor-e2e-client',
               KV9_BOOTSTRAP_TOKEN='floor-e2e-bootstrap', KV9_STORAGE='minio',
               KV9_PUBLIC_MAX_REQUESTS='12', KV9_PUBLIC_METADATA_RESERVED='8')
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
                           '--root-digest', root, '--creation-task', task, '--name', 'floor-src')
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
        data_node = group_leader()

        # --- THE FLOOD: 40 writers hammer raw-puts at the DATA-GROUP
        # leader for the whole window (shared pool = 16). Refusals are
        # EXPECTED (typed); acked writes are recorded for readback.
        import random
        rng = random.Random(17)
        acked = {}
        acked_lock = threading.Lock()
        flood_done = threading.Event()
        floor_seen = threading.Event()

        def flooder(worker):
            local = random.Random(1000 + worker)
            while not flood_done.is_set():
                key = ''.join(local.choice('0123456789abcdef') for _ in range(8))
                value = ('%02x' % worker) * 16
                try:
                    put = command('flood', 'client', 'raw-put', '--addr', addresses[data_node],
                                  '--keyspace', keyspace_id, '--key-hex', key,
                                  '--value-hex', value, success=None, log=False)
                except Exception:
                    return
                if put['exit_code'] == 0:
                    with acked_lock:
                        acked[key] = value
                elif put.get('reason') == 'metadata_floor':
                    floor_seen.set()
        flood = [threading.Thread(target=flooder, args=(w,)) for w in range(40)]
        for thread in flood:
            thread.start()

        # Metadata probes MUST keep succeeding through the whole flood:
        # the idempotent create-data-group confirm is a MetadataWrite.
        # The gate is ADMISSION liveness: the probe must never be refused
        # by admission. A raft-apply timeout under the flood's disk load is
        # not a floor failure — the idempotent confirm retries it (bounded),
        # and the retried probe must succeed.
        probe_failures = 0
        admission_refusals = 0
        probes = 0
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            probes += 1
            succeeded = False
            for attempt in range(4):
                probe = command('meta-probe', 'client', 'create-data-group', '--addr', addresses[owner_node],
                                '--root-digest', root, '--operation-id', f'{1:032x}',
                                '--voters', '1,2,3', success=None, log=False)
                if probe['exit_code'] == 0:
                    succeeded = True
                    break
                if probe.get('admission_refused') is not None or probe.get('reason'):
                    admission_refusals += 1
                    (output / f'meta-probe-admission-refusal-{admission_refusals}.log').write_text(json.dumps(probe))
                    break
                time.sleep(0.5)
            if not succeeded:
                probe_failures += 1
                (output / f'meta-probe-failure-{probe_failures}.log').write_text(json.dumps(probe))
            time.sleep(1)
        flood_done.set()
        for thread in flood:
            thread.join(timeout=30)
        require(probes >= 10, 'the probe loop must have run through the flood')
        require(admission_refusals == 0,
                f'{admission_refusals} metadata probes were REFUSED BY ADMISSION under the flood')
        require(probe_failures == 0,
                f'{probe_failures}/{probes} metadata probes failed even after idempotent retries')
        print(f'PASS: {probes} metadata probes all succeeded under a 40-writer flood '
              f'({len(acked)} raw writes acked)', flush=True)

        # The typed floor refusal was actually exercised, and the status
        # counters agree: raw_write saw floor refusals, metadata saw none.
        require(floor_seen.is_set(), 'the flood never hit the typed floor refusal')
        raw_floor = 0
        for n in (1, 2, 3):
            s = status(n)
            for line_key, value in s.items():
                if line_key == 'public_rpc_raw_write':
                    raw_floor += int(dict(p.split('=') for p in value.split(','))['refused_floor'])
                if line_key in ('public_rpc_metadata_read', 'public_rpc_metadata_write'):
                    meta_floor = int(dict(p.split('=') for p in value.split(','))['refused_floor'])
                    require(meta_floor == 0, f'{line_key} hit the floor: {meta_floor}')
        require(raw_floor > 0, 'status must count raw floor refusals')
        print(f'PASS: status counters: raw_write refused_floor={raw_floor}, metadata floors 0', flush=True)

        # Acked flood writes read back.
        sample = sorted(acked.items())
        checks = [sample[0], sample[len(sample) // 2], sample[-1]] if sample else []
        for key, value in checks:
            def served():
                for n in (1, 2, 3):
                    got = command('readback', 'client', 'raw-get', '--addr', addresses[n],
                                  '--keyspace', keyspace_id, '--key-hex', key, success=None, log=False)
                    if got.get('value_hex') == value:
                        return True
                return False
            wait(f'acked key {key} serves', served, 120)

        manifest_record.update(
            verdict='accepted', acked=len(acked), probes=probes, raw_floor_refusals=raw_floor,
            checks=['a 40-writer raw flood saturates the small shared pool for the whole window',
                    'every metadata probe succeeds throughout: the floor keeps control traffic admissible',
                    'the flood receives the TYPED metadata_floor refusal; metadata classes count zero floor refusals',
                    'acked flood writes read back'])
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
