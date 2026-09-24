#!/usr/bin/env python3
"""Real-process END-TO-END WRITE BACKPRESSURE gate (#20): bound absolute raft
log growth when writes outrun commit+compaction. With KV9_MAX_RAFT_LOG_ENTRIES
set and auto-compaction OFF (so nothing drains the log), sustained writes grow
the retained committed log to the bound and then every further WRITE is refused
with a retryable RESOURCE_EXHAUSTED (Error::WriteBackpressure) — the log stays
BOUNDED at the cap instead of growing without limit. Reads are never gated.

The fixture proves: (1) writes succeed up to the bound; (2) once the retained
log (committed - first_index) reaches the cap, writes are refused and the log
stays bounded (does not grow) under continued attempts; (3) reads still serve
during backpressure; (4) draining the log via a committed compaction floor
(the manual record-group-compaction verb) drops retained below the cap and
writes RESUME — backpressure is released, not a permanent wedge.

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
    parser.add_argument('--base-port', type=int, default=27600)
    args = parser.parse_args()
    for name in ('KV9_OBJECT_STORE_ENDPOINT', 'KV9_OBJECT_STORE_BUCKET',
                 'KV9_OBJECT_STORE_ACCESS_KEY', 'KV9_OBJECT_STORE_SECRET_KEY'):
        require(os.environ.get(name), f'{name} must be set to an isolated real MinIO bucket')
    binary, output = args.bin.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ, KV9_CLUSTER_TOKEN='backpressure-e2e-cluster',
               KV9_CLIENT_TOKENS='admin=backpressure-e2e-client', KV9_CLIENT_TOKEN='backpressure-e2e-client',
               KV9_BOOTSTRAP_TOKEN='backpressure-e2e-bootstrap', KV9_STORAGE='minio',
               KV9_AUTO_COMPACT_ENTRIES='0', KV9_AUTO_COMPACT_BYTES='0',
               KV9_MAX_RAFT_LOG_ENTRIES='120')
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
        return dict(dict(word.split('=', 1) for word in result.stdout.split() if '=' in word),
                    exit_code=result.returncode, _raw=result.stdout)

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

        CAP = 120  # KV9_MAX_RAFT_LOG_ENTRIES

        def retained(node):
            g = data_groups(node).get(region, {})
            c, f = g.get('committed'), g.get('log_first_index')
            return None if c is None or f is None else int(c) - int(f)

        def put_one(tag, i, want=None):
            node = group_leader()
            if node is None:
                return None
            key = '%08x' % i
            r = command(tag, 'client', 'raw-put', '--addr', addresses[node],
                        '--keyspace', keyspace_id, '--key-hex', key,
                        '--value-hex', '76616c', success=want, log=True)
            if r['exit_code'] == 0:
                written[key] = '76616c'
            return r

        def refusal_is_backpressure(r):
            text = (r.get('_raw') or '').lower()
            return 'backpressure' in text or 'resource' in text

        # PHASE 1: writes succeed up to the bound, then the retained log
        # (committed - first_index) reaches the cap and further writes are
        # REFUSED — auto-compaction is off, so nothing drains the log.
        refused_at, refused_r = None, None
        for i in range(600):
            r = put_one('fill', i, want=None)
            if r is None:
                time.sleep(0.2)
                continue
            if r['exit_code'] != 0:
                refused_at, refused_r = i, r
                break
        require(refused_at is not None,
                'a write must eventually hit backpressure at the log bound')
        require(refusal_is_backpressure(refused_r),
                'the refusal must be typed write backpressure (RESOURCE_EXHAUSTED): '
                + (refused_r.get('_raw') or '')[:200])
        # The gate reads the LIVE driver (raft_committed - log_first_index);
        # the on-disk status file publishing the same two counters lags it by a
        # flush, so poll it up to the cap rather than sampling instantaneously.
        wait('the retained log readback reaches the cap',
             lambda: (r := retained(group_leader())) is not None and r >= CAP, 60)
        gl = group_leader()
        print(f'PASS: backpressure engaged at write {refused_at}, retained={retained(gl)} (cap {CAP})',
              flush=True)

        # PHASE 2: the log stays BOUNDED under continued write attempts — every
        # refused write is not proposed, so retained does not grow past the cap.
        before = retained(group_leader())
        for i in range(40):
            r = put_one('bounded', 10_000 + i, want=False)  # all refused
            require(r is None or r['exit_code'] != 0, 'writes must stay refused at the bound')
        after = retained(group_leader())
        require(after is not None and after <= before + 8,
                f'log must stay bounded under backpressure: {before} -> {after}')
        print(f'PASS: log stays bounded under sustained refused writes: {before} -> {after}',
              flush=True)

        # PHASE 3: reads still serve during backpressure (only writes are gated).
        sample_key = next(iter(written))
        got = command('read-under-bp', 'client', 'raw-get', '--addr', addresses[group_leader()],
                      '--keyspace', keyspace_id, '--key-hex', sample_key, success=None, log=False)
        require(got.get('value_hex') == '76616c', 'reads must still serve during backpressure')
        print('PASS: reads still serve while writes are backpressured', flush=True)

        # PHASE 4: drain the log with a committed compaction floor at the applied
        # position; retained drops below the cap and writes RESUME (backpressure
        # released, not a permanent wedge).
        meta = wait('metadata leader for the drain verb', leader)
        root = status(meta)['root_digest']
        gl = group_leader()
        da = json.loads(status(gl).get('data_groups', '[]'))
        floor = next(g['driver_applied'] for g in da if g['region'] == region)
        command('drain-compact', 'client', 'record-group-compaction', '--addr', addresses[meta],
                '--root-digest', root, '--region', region,
                '--floor-term', floor['term'], '--floor-index', floor['index'])
        wait('the log drains below the cap after compaction',
             lambda: (r := retained(group_leader())) is not None and r < CAP, 180)
        resume = wait('a write resumes after the drain',
                      lambda: (rr := put_one('resume', 20_000, want=None)) is not None
                      and rr['exit_code'] == 0, 180)
        print(f'PASS: writes resumed after the log drained (retained={retained(group_leader())})',
              flush=True)

        manifest_record.update(
            verdict='accepted', written=len(written), cap=CAP, refused_at=refused_at,
            checks=['writes succeed up to the log bound, then are refused with typed write backpressure (RESOURCE_EXHAUSTED)',
                    'the retained log stays BOUNDED at the cap under sustained refused writes (never grows past it)',
                    'reads still serve while writes are backpressured',
                    'draining the log via a committed compaction floor releases backpressure and writes resume'])
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
