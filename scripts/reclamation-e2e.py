#!/usr/bin/env python3
"""Real-process physical-reclamation gate: cascades retire parents
aggressively; with KV9_RECLAIM_RETIRED=1 every retired local group's
payload (engine WAL, segment directory, raft log) is physically deleted
under re-verified committed authority, while the durable group record
survives as the permanent fence.

The gate: sustained cascade writes all land and read back (the
cascade-split gate), EVERY retired group on every node transitions to
'reclaimed' with its payload gone and its record retained on disk, the
leaves keep serving, and a full-cluster restart recovers the reclaimed
world whole (records fence; nothing reopens; keys still serve).

Verdicts: 'completed' (the gate), 'reproduced' (a cascade hang
signature — collected as evidence), 'failed' (anything else).

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
    parser.add_argument('--base-port', type=int, default=27080)
    args = parser.parse_args()
    for name in ('KV9_OBJECT_STORE_ENDPOINT', 'KV9_OBJECT_STORE_BUCKET',
                 'KV9_OBJECT_STORE_ACCESS_KEY', 'KV9_OBJECT_STORE_SECRET_KEY'):
        require(os.environ.get(name), f'{name} must be set to an isolated real MinIO bucket')
    binary, output = args.bin.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ, KV9_CLUSTER_TOKEN='reclaim-e2e-cluster',
               KV9_CLIENT_TOKENS='admin=reclaim-e2e-client', KV9_CLIENT_TOKEN='reclaim-e2e-client',
               KV9_BOOTSTRAP_TOKEN='reclaim-e2e-bootstrap', KV9_STORAGE='minio',
               KV9_AUTO_SPLIT_BYTES='4096', KV9_RECLAIM_RETIRED='1')
    processes, handles, commands = {}, [], []
    manifest_record = dict(verdict='running', commands=commands,
                           binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                           runner_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                           revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
                           dirty=bool(subprocess.check_output(['git', 'status', '--porcelain'])),
                           chaos_mesh=False, serving_or_promotion=False)
    (output / 'runner.py').write_bytes(Path(__file__).read_bytes())
    addresses = {i: f'127.0.0.1:{args.base_port+i}' for i in range(1, 4)}
    observations = (output / 'observations.jsonl').open('w')

    def command(label, *arguments, success=True, extra_env=None):
        started = time.time_ns()
        call_env = dict(env, **(extra_env or {}))
        result = subprocess.run([str(binary), *map(str, arguments)], env=call_env, text=True,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=300)
        if success is None:
            label = f'{label}-probe-{len(commands)}'
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

    def observe(tag):
        snap = dict(tag=tag, monotonic=time.monotonic())
        for n in (1, 2, 3):
            s = status(n)
            snap[f'n{n}'] = dict(
                role=s.get('role'), groups=s.get('data_groups'),
                control_error=s.get('data_group_control_error'))
        observations.write(json.dumps(snap) + '\n')
        observations.flush()
        return snap

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
                           '--root-digest', root, '--creation-task', task, '--name', 'cascade-src')
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

        # --- Sustained writes at a threshold LOW enough that children
        # re-trigger. Fill until a write refuses to land anywhere for the
        # hang window — that is the e2e-third signature — or all land.
        import random
        rng = random.Random(9)
        written, stuck_key = {}, None
        for i in range(800):
            key = ''.join(rng.choice('0123456789abcdef') for _ in range(8))
            value = ('%04x' % i) * 8
            landed = False
            for attempt in range(120):
                for n in (1, 2, 3):
                    if n not in processes:
                        continue
                    put = command('fill', 'client', 'raw-put', '--addr', addresses[n],
                                  '--keyspace', keyspace_id, '--key-hex', key,
                                  '--value-hex', value, success=None)
                    if put['exit_code'] == 0:
                        written[key] = value
                        landed = True
                        break
                if landed:
                    break
                if attempt % 20 == 19:
                    observe(f'fill-{i}-attempt-{attempt}')
                time.sleep(0.5)
            if not landed:
                stuck_key = key
                print(f'HANG: fill key {key} refused everywhere for 60s at write {i}', flush=True)
                break
            if i % 25 == 0:
                observe(f'fill-{i}')

        if stuck_key is None:
            # No mid-fill hang. The fence only bites writes/reads that hit a
            # sealed-unpublished slice — so KEEP writing (in-flight pipelines
            # finish or hang under load) and then read back EVERY written key.
            observe('fill-complete')
            for i in range(60):
                key = ''.join(rng.choice('0123456789abcdef') for _ in range(8))
                value = ('beef%04x' % i) * 4
                landed = False
                for attempt in range(120):
                    for n in (1, 2, 3):
                        if n not in processes:
                            continue
                        put = command('post-fill', 'client', 'raw-put', '--addr', addresses[n],
                                      '--keyspace', keyspace_id, '--key-hex', key,
                                      '--value-hex', value, success=None)
                        if put['exit_code'] == 0:
                            written[key] = value
                            landed = True
                            break
                    if landed:
                        break
                    if attempt % 20 == 19:
                        observe(f'post-fill-{i}-attempt-{attempt}')
                    time.sleep(0.5)
                if not landed:
                    stuck_key = key
                    print(f'HANG: post-fill key {key} refused everywhere for 60s', flush=True)
                    break
        if stuck_key is None:
            observe('post-fill-complete')
            # Every written key must come back through public routing: a
            # sealed-unpublished slice would fence its keys from serving.
            unread = dict(written)
            deadline = time.monotonic() + 300
            while unread and time.monotonic() < deadline:
                for key, value in list(unread.items()):
                    for n in (1, 2, 3):
                        if n not in processes:
                            continue
                        got = command('readback', 'client', 'raw-get', '--addr', addresses[n],
                                      '--keyspace', keyspace_id, '--key-hex', key, success=None)
                        if got.get('value_hex') == value:
                            del unread[key]
                            break
                if unread:
                    observe(f'readback-{len(unread)}-unread')
                    time.sleep(2)
            if unread:
                stuck_key = sorted(unread)[0]
                print(f'HANG: {len(unread)} written keys never served; first {stuck_key}', flush=True)
        if stuck_key is None:
            # EVERY retired group on every node must reclaim: state flips to
            # 'reclaimed', the payload disappears, the record fence stays.
            def group_states(n):
                return {g['region']: g.get('state') for g in
                        json.loads(status(n).get('data_groups', '[]'))}

            def all_reclaimed():
                saw_any = False
                for n in (1, 2, 3):
                    states = group_states(n)
                    if not states:
                        return False
                    if any(s == 'retired' for s in states.values()):
                        return False
                    saw_any = saw_any or any(s == 'reclaimed' for s in states.values())
                return saw_any
            wait('every retired group reclaims on every node', all_reclaimed, 300)
            reclaimed = {n: sorted(r for r, s in group_states(n).items() if s == 'reclaimed')
                         for n in (1, 2, 3)}
            payload_checked = 0
            for n in (1, 2, 3):
                require(len(reclaimed[n]) >= 2, f'n{n}: cascades must have reclaimed parents')
                for region in reclaimed[n]:
                    d = output / f'n{n}/data-groups/{region}'
                    require((d / 'group-record').exists(), f'{d}: the fence record must survive')
                    for gone in ('data.wal', 'data.segments', 'raft'):
                        require(not (d / gone).exists(), f'{d}/{gone}: payload must be deleted')
                    payload_checked += 1
            observe('reclaimed')
            # The reclaimed world survives a full-cluster restart whole.
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

            def still_reclaimed():
                return all(sorted(r for r, s in group_states(n).items() if s == 'reclaimed')
                           == reclaimed[n] for n in (1, 2, 3))
            wait('every reclaimed group stays fenced after the restart', still_reclaimed, 120)
            manifest_record.update(
                verdict='completed', written=len(written),
                reclaimed={str(n): reclaimed[n] for n in reclaimed},
                payload_directories_verified=payload_checked,
                checks=['sustained cascade writes all land and read back; no manual verb runs',
                        'every retired group on every node reclaims: payload deleted, record fence retained',
                        'active leaves keep serving throughout',
                        'a full-cluster restart recovers the reclaimed world: fences hold, nothing reopens, keys serve'])

        if stuck_key is not None and manifest_record['verdict'] == 'running':
            # Collect the surfaced evidence for two minutes.
            errors = {}
            for round_index in range(24):
                snap = observe(f'hang-{round_index}')
                for n in (1, 2, 3):
                    err = snap.get(f'n{n}', {}).get('control_error')
                    if err and err != 'null':
                        errors.setdefault(err, dict(node=n, first_round=round_index))
                time.sleep(5)
            # Final durable state per node for offline inspection.
            for n in (1, 2, 3):
                (output / f'final-status-{n}.txt').write_text(
                    (output / f'n{n}/status').read_text() if (output / f'n{n}/status').exists() else '')
            manifest_record.update(verdict='reproduced', stuck_key=stuck_key, written=len(written),
                                   surfaced_errors=sorted(errors),
                                   error_details={e: d for e, d in errors.items()})
    except Exception as error:
        manifest_record.update(verdict='failed', error=str(error))
        raise
    finally:
        for node in list(processes):
            stop(node)
        for handle in handles:
            handle.close()
        observations.close()
        (output / 'result.json').write_text(json.dumps(manifest_record, indent=2) + '\n')


if __name__ == '__main__':
    main()
