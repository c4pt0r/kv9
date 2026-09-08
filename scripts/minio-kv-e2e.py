#!/usr/bin/env python3
"""Three real nodes, real S3, killed leaders, reclaimed WAL and remote recovery."""
import json
import hashlib
import re
import os
from pathlib import Path
import secrets
import subprocess
import tempfile
import time
import urllib.request

repo = Path(__file__).resolve().parent.parent
binary = repo / 'target/debug/kv9'
artifacts = Path(tempfile.mkdtemp(prefix='kv9-minio-kv-'))
env = os.environ.copy()
processes = {}
logs = []
container = None
base = int(env.get('KV9_BASE_PORT', '22400'))
print(f'Artifacts: {artifacts}', flush=True)

def run(args, **kwargs):
    result = subprocess.run([str(x) for x in args], env=env, text=True, capture_output=True, timeout=30, **kwargs)
    if result.returncode:
        raise RuntimeError(f'{args[0:3]} failed: {result.stdout}{result.stderr}')
    return result.stdout.strip()

def wait(label, condition, seconds=45):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        answer = condition()
        if answer:
            return answer
        for node, process in processes.items():
            if process.poll() is not None:
                raise AssertionError(f'n{node} exited unexpectedly: {(artifacts / f"n{node}.log").read_text()[-4000:]}')
        time.sleep(0.05)
    raise AssertionError(f'timed out: {label}')

def status(node):
    try:
        fields = dict(line.split('=', 1) for line in (artifacts / f'n{node}/status').read_text().splitlines())
        if int(fields['pid']) != processes[node].pid:
            return {}
        return fields
    except (FileNotFoundError, ValueError, KeyError):
        return {}

def leader():
    states = {node: status(node) for node in processes}
    if not states or any(s.get('bootstrap_state') != 'Serving' or s.get('fatal') for s in states.values()):
        return None
    ids = {s.get('leader_id') for s in states.values()}
    if len(ids) != 1 or None in ids or '0' in ids:
        return None
    chosen = int(ids.pop())
    return chosen if chosen in states and states[chosen].get('role') == 'leader' else None

def start(node, flush='100'):
    log = open(artifacts / f'n{node}.log', 'a')
    logs.append(log)
    local_env = {**env, 'KV9_FLUSH_INTERVAL_MS': flush}
    processes[node] = subprocess.Popen([str(binary), 'start', '--node-id', str(node), '--addr', f'127.0.0.1:{base+node}', '--data-dir', str(artifacts / f'n{node}')], env=local_env, stdout=log, stderr=log)

def kill(node):
    process = processes.pop(node)
    process.kill()
    process.wait(timeout=10)

def client(command, *args):
    # Typed read refusals and pre-append NotLeader are retryable. An ordinary
    # transport timeout on a write remains ambiguous and fails this harness.
    deadline = time.monotonic() + 15
    while True:
        node = wait('agreed serving leader', leader)
        result = subprocess.run([str(binary), 'client', command, '--addr', f'127.0.0.1:{base+node}', *map(str,args)], env=env, text=True, capture_output=True, timeout=30)
        if result.returncode == 0:
            return result.stdout.strip()
        error = result.stdout + result.stderr
        retryable = 'not_leader=true' in error or (command in ('raw-get', 'raw-scan') and 'read_unconfirmed=true' in error)
        if not retryable or time.monotonic() >= deadline:
            raise RuntimeError(f'{command} failed: {error}')
        time.sleep(0.05)

def put(key, value, keyspace):
    output = client('raw-put', '--keyspace', keyspace, '--key-hex', key.hex(), '--value-hex', value.hex())
    return int(dict(line.split('=', 1) for line in output.splitlines())['applied_index'])

def check(key, value, keyspace):
    output = client('raw-get', '--keyspace', keyspace, '--key-hex', key.hex())
    expected = 'found=false' if value is None else f'value_hex={value.hex()}'
    assert output == expected, f'key {key!r}: wanted {expected}, got {output}'

def checkpoint(node):
    path = artifacts / f'n{node}/catalog.checkpoint'
    try:
        data = path.read_bytes()
        assert data.startswith(b'KV9CHECKPOINT\x01')
        return json.loads(data[len(b'KV9CHECKPOINT\x01'):])
    except FileNotFoundError:
        return None

def pending_record(node):
    path = artifacts/f'n{node}/catalog.pending'
    assert path.is_file(), 'prepared gate requires a durable pending journal before propose'
    data = path.read_bytes()
    magic = b'KV9PENDING\x01'
    assert data.startswith(magic), 'actual production pending record must exist'
    assert hashlib.sha256(data[:-32]).digest() == data[-32:], 'pending record must be intact'
    generation = int.from_bytes(data[len(magic):len(magic)+8], 'little')
    payload = data[len(magic)+8:-32]
    assert payload.startswith(b'KV9CHECKPOINT\x01')
    return generation, json.loads(payload[len(b'KV9CHECKPOINT\x01'):]), data

def pending_startup_refusals(node, original):
    path = artifacts/f'n{node}/catalog.pending'
    wal = artifacts/f'n{node}/catalog.wal'
    before = wal.read_bytes()
    bad_crc = bytearray(original)
    bad_crc[-1] ^= 1
    magic = b'KV9PENDING\x01'
    head = original[:len(magic)+8]
    checkpoint_magic = b'KV9CHECKPOINT\x01'
    manifest = json.loads(original[len(head)+len(checkpoint_magic):-32])
    manifest['term'] += 100
    body = head + checkpoint_magic + json.dumps(manifest, separators=(',',':')).encode()
    wrong_term = body + hashlib.sha256(body).digest()
    for damaged, expected in [(bytes(bad_crc), 'pending flush checksum mismatch'), (wrong_term, "pending flush does not belong to this cluster's committed Raft history")]:
        path.write_bytes(damaged)
        offset = len((artifacts/f'n{node}.log').read_text())
        start(node)
        rejected = processes.pop(node)
        try:
            code = rejected.wait(timeout=10)
        finally:
            if rejected.poll() is None:
                rejected.kill()
                rejected.wait(timeout=10)
        output = (artifacts/f'n{node}.log').read_text()[offset:]
        assert code != 0 and expected in output, f'pending preflight must refuse before serving: {output}'
        assert path.read_bytes() == damaged, 'invalid pending record must not be reset'
        assert wal.read_bytes() == before, 'invalid pending record must not edit state-machine WAL'
    path.write_bytes(original)

def latest_generation(node):
    values = re.findall(r'checkpoint generation=(\d+) through=\d+ adopted', (artifacts/f'n{node}.log').read_text())
    return max(map(int, values), default=0)

def pending_crash_cases(keyspace):
    gates = artifacts / 'flush-gates'
    for phase in ('prepared', 'applied'):
        wait('previous pending slots settled', lambda: all(not (artifacts/f'n{n}/catalog.pending').exists() for n in processes))
        for n in processes:
            (gates/f'{n}-{phase}').write_text('pause')
        put(('trigger-'+phase).encode(), b'confirmed-before-crash', keyspace)
        def arrived():
            return next((n for n in processes if (gates/f'{n}-{phase}.arrived').exists()), None)
        stopped_node = wait('durable '+phase+' crash gate', arrived)
        generation, manifest, journal_bytes = pending_record(stopped_node)
        if phase == 'prepared':
            assert (checkpoint(stopped_node) or {}).get('index', 0) < manifest['index'], 'pending upload must not authorize reclamation'
        # Change the full snapshot AFTER the controlled cut and confirm it on
        # every replica before killing the leader. Otherwise the new leader can
        # legitimately upload the same SST set and EffectSettled would mask the
        # historical-winner branch this prepared case is intended to exercise.
        after_cut = put(('after-cut-'+phase).encode(), b'distinct-future-snapshot', keyspace)
        wait('post-cut write applied on every replica', lambda: all(int(status(n).get('applied_index', 0)) >= after_cut for n in processes))
        (artifacts/f'{phase}-captured.pending').write_bytes(journal_bytes)
        log_offset = len((artifacts/f'n{stopped_node}.log').read_text())
        kill(stopped_node)
        assert (artifacts/f'n{stopped_node}/catalog.pending').read_bytes() == journal_bytes
        if phase == 'prepared':
            pending_startup_refusals(stopped_node, journal_bytes)
        for n in (1, 2, 3):
            (gates/f'{n}-{phase}').unlink(missing_ok=True)
        wait('new leader after '+phase+' crash', leader)
        # Move beyond expected+1 deliberately. Latest-pair-only reconciliation
        # cannot decide the prepared-but-never-submitted attempt at this point.
        for turn in range(3):
            index = put(f'{phase}-advance-{turn}'.encode(), b'majority-progress', keyspace)
            wait('new remote generation', lambda: all((checkpoint(n) or {}).get('index', 0) >= index for n in processes))
        wait('historical CAS window retained', lambda: all(latest_generation(n) > generation+1 for n in processes))
        # Hold the background reconciler until this node's own driver has
        # applied the majority's later generations. Without this barrier it
        # could observe expected+1 during catch-up, bypassing historical lookup.
        applied_after_window = max(int(status(n)['applied_index']) for n in processes)
        recovery_gate = gates/f'{stopped_node}-recovering'
        recovery_gate.with_suffix('.arrived').unlink(missing_ok=True)
        recovery_gate.write_text('pause')
        start(stopped_node)
        wait('reconciler reached recovery gate', lambda: recovery_gate.with_suffix('.arrived').exists())
        wait('local apply passed the original CAS window', lambda: int(status(stopped_node).get('applied_index', 0)) >= applied_after_window)
        recovery_gate.unlink()
        wait('restart with real pending state', leader)
        expected_resume = f'checkpoint pending generation={generation} through={manifest["index"]} resuming'
        def settled():
            new_log = (artifacts/f'n{stopped_node}.log').read_text()[log_offset:]
            if expected_resume not in new_log or (artifacts/f'n{stopped_node}/catalog.pending').exists():
                return False
            outcome = 'WindowRefused' if phase == 'prepared' else '(AlreadyApplied|EffectSettled)'
            return bool(re.search(r'checkpoint pending settled '+outcome, new_log))
        wait('original identity settles from retained history', settled)
        check(('trigger-'+phase).encode(), b'confirmed-before-crash', keyspace)
        index = put(('after-'+phase).encode(), b'flush-progress', keyspace)
        wait('flush pipeline progresses after reconciliation', lambda: all((checkpoint(n) or {}).get('index', 0) >= index for n in processes))
        print(f'Pending crash cut {phase}: original identity recovered after generation window, journal cleared.', flush=True)

try:
    if env.get('KV9_MINIO_EXTERNAL') != '1':
        container = 'kv9-minio-e2e-' + secrets.token_hex(4)
        env.update(KV9_OBJECT_STORE_ENDPOINT='http://127.0.0.1:19450', KV9_OBJECT_STORE_BUCKET='kv9-e2e', KV9_OBJECT_STORE_ACCESS_KEY='kv9'+secrets.token_hex(8), KV9_OBJECT_STORE_SECRET_KEY=secrets.token_hex(24))
        secret_file = artifacts / 'minio.env'
        secret_file.write_text('MINIO_ROOT_USER='+env['KV9_OBJECT_STORE_ACCESS_KEY']+'\nMINIO_ROOT_PASSWORD='+env['KV9_OBJECT_STORE_SECRET_KEY']+'\n')
        secret_file.chmod(0o600)
        run(['docker', 'run', '-d', '--name', container, '-p', '127.0.0.1:19450:9000', '--env-file', secret_file, 'quay.io/minio/minio@sha256:14cea493d9a34af32f524e538b8346cf79f3321eff8e708c1e2960462bd8936e', 'server', '/data'])
        def healthy():
            try:
                with urllib.request.urlopen(env['KV9_OBJECT_STORE_ENDPOINT']+'/minio/health/live', timeout=2) as response:
                    return response.status == 200
            except OSError:
                return False
        wait('MinIO health', healthy)
        host = 'http://'+env['KV9_OBJECT_STORE_ACCESS_KEY']+':'+env['KV9_OBJECT_STORE_SECRET_KEY']+'@127.0.0.1:9000'
        run(['docker', 'exec', '-e', 'MC_HOST_e2e='+host, container, 'mc', 'mb', '--ignore-existing', 'e2e/'+env['KV9_OBJECT_STORE_BUCKET']])
        secret_file.unlink()
    for name in ['ENDPOINT', 'BUCKET', 'ACCESS_KEY', 'SECRET_KEY']:
        assert env.get('KV9_OBJECT_STORE_'+name), f'missing KV9_OBJECT_STORE_{name}'
    env.update(KV9_STORAGE='minio', KV9_BOOTSTRAP_TOKEN=secrets.token_hex(24), KV9_CLUSTER_TOKEN=secrets.token_hex(24), KV9_CLIENT_TOKEN=secrets.token_hex(24))
    env['KV9_CLIENT_TOKENS'] = 'acceptance='+env['KV9_CLIENT_TOKEN']
    if env.get('KV9_TEST_PENDING_CRASHES') == '1':
        gates = artifacts/'flush-gates'
        gates.mkdir()
        env['KV9_TESTING_FLUSH_PAUSE_DIR'] = str(gates)
    root = artifacts / 'root.bin'
    run([binary, 'root-create', '--output', root, '--voters', ','.join(f'{n}@127.0.0.1:{base+n}' for n in (1,2,3))])
    for node in (1,2,3):
        run([binary, 'init', '--root', root, '--node-id', node, '--data-dir', artifacts/f'n{node}'])
        start(node)
    wait('three serving replicas', leader)
    created = client('create-keyspace', '--name', 'remote-kv', '--api-type', 'raw')
    keyspace = dict(line.split('=',1) for line in created.splitlines())['keyspace_id']
    put(b'keep', b'remote-value', keyspace)
    put(b'deleted', b'old-value', keyspace)
    client('raw-delete', '--keyspace', keyspace, '--key-hex', b'deleted'.hex())
    boundary = put(b'overwrite', b'new-value', keyspace)
    wait('all replicas install remote checkpoint', lambda: all((checkpoint(n) or {}).get('index', 0) >= boundary for n in processes))
    # Wait for the actual tail replacement, not merely for the sidecar rename.
    def reclaimed():
        for n in processes:
            data = (artifacts/f'n{n}/catalog.wal').read_bytes()
            if b'remote-value' in data or b'old-value' in data or b'new-value' in data:
                return False
        return True
    wait('absorbed values absent from reclaimed WALs', reclaimed)
    print('Remote checkpoint committed on all replicas; absorbed data reclaimed.', flush=True)
    if env.get('KV9_TEST_PENDING_CRASHES') == '1':
        pending_crash_cases(keyspace)
    old_leader = leader()
    kill(old_leader)
    wait('majority elects a new leader', leader)
    check(b'keep', b'remote-value', keyspace)
    check(b'deleted', None, keyspace)
    check(b'overwrite', b'new-value', keyspace)
    start(old_leader)
    wait('killed leader rejoins', leader)
    # Kill EVERY node, retaining only manifest metadata and protocol history;
    # catalog WALs already contain no absorbed values. Restart requires real GETs.
    for n in list(processes):
        kill(n)
    for n in (1,2,3):
        start(n, flush='600000')
    wait('fresh processes recover from MinIO and WAL tail', leader)
    check(b'keep', b'remote-value', keyspace)
    check(b'deleted', None, keyspace)
    check(b'overwrite', b'new-value', keyspace)
    tail_index = put(b'tail', b'after-checkpoint', keyspace)
    wait('tail replicated everywhere', lambda: all(int(status(n).get('applied_index', 0)) >= tail_index for n in processes))
    for n in list(processes):
        kill(n)
    for n in (1,2,3):
        start(n, flush='600000')
    wait('remote checkpoint plus unflushed tail restart', leader)
    check(b'tail', b'after-checkpoint', keyspace)
    check(b'deleted', None, keyspace)
    put(b'post-restart', b'writable', keyspace)
    check(b'post-restart', b'writable', keyspace)
    print('PASS: minio-kv-e2e (3 replicas, failover, remote checkpoint, reclaimed WAL, live tail, deletes)', flush=True)
    if env.get('KV9_TEST_PENDING_CRASHES') == '1':
        print('PASS: minio-pending-e2e (durable prepare, applied-before-clear, historical settlement, resumed flush)', flush=True)
finally:
    for n in list(processes):
        kill(n)
    for log in logs:
        log.close()
    if container:
        subprocess.run(['docker', 'rm', '-f', container], capture_output=True, timeout=30)
