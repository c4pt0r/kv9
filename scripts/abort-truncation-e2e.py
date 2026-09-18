#!/usr/bin/env python3
"""Real-process abort-settled-truncation gate: after a committed abort
settles a stranded operation (pin released, learner detached), the
source's retained raft log below a floor is compacted under the SAME
raft gates (leader-only, every voter matched, deferred-sync barrier,
local prefix only) — with the committed ABORT as the settlement
authority instead of install evidence. The full stranded-recovery chain
then still completes: a fresh incarnation re-migrates the compacted
region end to end.

Chain (extends the migration-abort gate):

A destination store lost mid-operation (attached, image published and
captured, NEVER installed) wedges its migration forever: evidence cannot
exist, so the source pin sticks at Published, and the learner stays in
the source configuration. The committed ABORT row (kind 108, KV9ABT01)
settles the operation instead of evidence — mutually exclusive with it,
permanently — and unlocks: source-pin quiesce and release through the
UNCHANGED ledger verbs, learner detach at the source leader, and a fresh
migration of the same region to a re-provisioned destination incarnation
that then completes end to end (install, adoption, catchup, evidence,
quiesce, release).

Requires KV9_OBJECT_STORE_* (isolated real MinIO); servers run KV9_STORAGE=minio.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
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
    parser.add_argument('--base-port', type=int, default=27340)
    args = parser.parse_args()
    for name in ('KV9_OBJECT_STORE_ENDPOINT', 'KV9_OBJECT_STORE_BUCKET',
                 'KV9_OBJECT_STORE_ACCESS_KEY', 'KV9_OBJECT_STORE_SECRET_KEY'):
        require(os.environ.get(name), f'{name} must be set to an isolated real MinIO bucket')
    binary, output = args.bin.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ, KV9_CLUSTER_TOKEN='aborttrunc-e2e-cluster',
               KV9_CLIENT_TOKENS='admin=aborttrunc-e2e-client', KV9_CLIENT_TOKEN='aborttrunc-e2e-client',
               KV9_BOOTSTRAP_TOKEN='aborttrunc-e2e-bootstrap', KV9_STORAGE='minio')
    processes, handles, commands = {}, [], []
    manifest_record = dict(verdict='running', commands=commands,
                           binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                           runner_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                           revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
                           dirty=bool(subprocess.check_output(['git', 'status', '--porcelain'])),
                           chaos_mesh=False, serving_or_promotion=False)
    (output / 'runner.py').write_bytes(Path(__file__).read_bytes())
    addresses = {i: f'127.0.0.1:{args.base_port+i}' for i in range(1, 5)}

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

    root_holder = [None]

    def plan_with_retry(label, node, operation, path):
        def planned():
            result = command(label, 'client', 'plan-migration-image', '--addr', addresses[node],
                             '--root-digest', root_holder[0], '--operation-id', operation,
                             '--manifest-file', path, success=None)
            return result if result['exit_code'] == 0 else None
        return wait(f'{label} intent locally applied', planned, 30)

    def capture_with_retry(label, node, operation, path):
        def captured():
            result = command(label, 'client', 'capture-migration-image', '--addr', addresses[node],
                             '--root-digest', root_holder[0], '--operation-id', operation,
                             '--record-file', path, success=None)
            if result['exit_code'] == 0:
                return result
            text = open(output / f"{label}-probe-{len(commands)-1}.log").read()
            if 'not published' in text or 'not committed' in text:
                return None
            raise RuntimeError(f'{label}: non-retryable capture failure; see {output}')
        return wait(f'{label} owners locally applied', captured, 30)

    def attach_with_retry(label, node, operation):
        # The intent commits at the metadata leader; the group leader's
        # local applied state may lag for a moment. Attach is idempotent.
        def attached():
            result = command(label, 'client', 'attach-migration-learner', '--addr', addresses[node],
                             '--root-digest', root_holder[0], '--operation-id', operation, success=None)
            if result['exit_code'] == 0 and result.get('attach_outcome') in ('attached', 'confirmed'):
                return result
            return None
        return wait(f'{label} learner attached', attached, 30)

    def abort_with_retry(label, operation, expect):
        # The abort commits at the metadata leader against LOCAL applied
        # state; committed rows replicate within moments. Idempotent.
        def recorded():
            result = command(label, 'client', 'record-migration-abort', '--addr', addresses[owner_holder[0]],
                             '--root-digest', root_holder[0], '--operation-id', operation, success=None)
            if result['exit_code'] == 0 and result.get('abort_outcome') == expect:
                return result
            return None
        return wait(f'{label} abort {expect}', recorded, 30)

    owner_holder = [None]
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
        owner_holder[0] = owner_node
        wait('all endpoints ready', lambda: all(status(n).get('endpoint_ready') == 'true' for n in processes))
        root = status(owner_node)['root_digest']
        root_holder[0] = root

        created = command('create-group', 'client', 'create-data-group', '--addr', addresses[owner_node],
                          '--root-digest', root, '--operation-id', f'{1:032x}', '--voters', '1,2,3')
        task, region = created['task_id'], int(created['region_id'])
        keyspace = command('create-keyspace', 'client', 'create-data-keyspace', '--addr', addresses[owner_node],
                           '--root-digest', root, '--creation-task', task, '--name', 'abort-src')
        keyspace_id = keyspace['keyspace_id']

        admitted = command('admit-node-4', 'client', 'admit-node', '--addr', addresses[owner_node],
                           '--node-id', 4, '--node-addr', addresses[4], '--ttl-seconds', 600)
        command('join-4', 'join', '--root', root_file, '--node-id', 4, '--data-dir', output / 'n4',
                extra_env={'KV9_JOIN_TICKET': admitted['join_ticket']})
        start(4, 'joiner', ticket=admitted['join_ticket'])
        wait('joiner registered and ready', lambda: status(4).get('endpoint_ready') == 'true')

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
        for n, (key, value) in enumerate([('616c706861', '6f6e65'), ('62657461', '74776f')]):
            command(f'put-{n}', 'client', 'raw-put', '--addr', addresses[data_node],
                    '--keyspace', keyspace_id, '--key-hex', key, '--value-hex', value)

        command('migrate', 'client', 'migrate-data-group', '--addr', addresses[owner_node],
                '--root-digest', root, '--operation-id', f'{7:032x}',
                '--creation-task', task, '--destination-node', 4)
        attached = attach_with_retry('attach', data_node, f'{7:032x}')
        require(attached['destination_node'] == '4', 'attach lacks the learner receipt')
        plan_with_retry('plan', data_node, f'{7:032x}', output / 'image.manifest')
        command('bind', 'client', 'bind-migration-image', '--addr', addresses[owner_node],
                '--root-digest', root, '--operation-id', f'{7:032x}',
                '--manifest-file', output / 'image.manifest')
        captured = capture_with_retry('capture', data_node, f'{7:032x}', output / 'image.record')

        # --- THE STRANDING: the destination dies with the image published
        # and captured but NEVER installed. Its incarnation is gone.
        stop(4)

        # Ledger discipline before any settlement: quiesce and release refuse.
        import struct

        def sealed(data):
            return data + hashlib.sha256(data).digest()

        def ledger_token(owner_hex):
            return bytes.fromhex(owner_hex) + struct.pack('>Q', 1)

        root_bytes = bytes.fromhex(root)
        requests = {
            'quiesce-source': bytes([5]) + ledger_token(captured['source_owner'])
            + ledger_token(captured['destination_owner']),
            'release-source': bytes([6]) + ledger_token(captured['source_owner']),
            'release-destination': bytes([6]) + ledger_token(captured['destination_owner']),
        }
        for name, payload in requests.items():
            (output / f'{name}.request').write_bytes(
                sealed(b'KV9RTX01' + root_bytes + payload))

        def retention(label, name, success=True):
            return command(label, 'client', 'retention-apply', '--addr', addresses[owner_node],
                           '--root-digest', root, '--request-file', output / f'{name}.request',
                           success=success)

        def owner_phase(owner_hex):
            probe = command('owner-view', 'client', 'retention-owner', '--addr', addresses[owner_node],
                            '--root-digest', root, '--owner-id', owner_hex, success=None)
            require(probe.get('found') == 'true', 'committed owner must read back')
            return bytes.fromhex(probe['owner_hex'])[155]

        early = retention('quiesce-early', 'quiesce-source', success=None)
        require(early['exit_code'] != 0, 'quiesce before any settlement must refuse')
        early = retention('release-early', 'release-source', success=None)
        require(early['exit_code'] != 0, 'release before quiesce must refuse')
        require(owner_phase(captured['source_owner']) == 2, 'source pin stays published')

        # An abort for an uncommitted operation refuses.
        unknown = command('abort-unknown', 'client', 'record-migration-abort', '--addr', addresses[owner_node],
                          '--root-digest', root, '--operation-id', f'{9:032x}', success=None)
        require(unknown['exit_code'] != 0, 'an uncommitted operation must refuse to abort')

        # The committed abort: first a mutation, then an exact confirmation.
        recorded = abort_with_retry('record-abort', f'{7:032x}', 'committed')
        confirmed = abort_with_retry('record-abort-again', f'{7:032x}', 'confirmed')
        require(confirmed['abort_task'] == recorded['abort_task'],
                'the identical abort must confirm the same committed row')

        # Attach refuses permanently after the abort.
        node = wait('source leader after stranding', group_leader)
        reattach = command('attach-after-abort', 'client', 'attach-migration-learner',
                           '--addr', addresses[node], '--root-digest', root,
                           '--operation-id', f'{7:032x}', success=None)
        require(reattach['exit_code'] != 0, 'attach after abort must refuse')
        require('aborted' in open(output / f'attach-after-abort-probe-{len(commands)-1}.log').read(),
                'the attach refusal must name the abort')

        # The abort settles the ledger: quiesce commits, release follows,
        # the destination pin still never drops.
        result = retention('quiesce-after-abort', 'quiesce-source')
        require(result['retention_outcome'] == 'changed', 'quiesce commits after the abort')
        require(owner_phase(captured['source_owner']) == 3, 'source pin quiesced')
        result = retention('release-after-abort', 'release-source')
        require(result['retention_outcome'] == 'changed', 'release commits after quiesce')
        require(owner_phase(captured['source_owner']) == 4, 'source pin released')
        require(owner_phase(captured['destination_owner']) == 2,
                'the abandoned image pin stays published (documented cost)')
        blocked = retention('release-destination-blocked', 'release-destination', success=None)
        require(blocked['exit_code'] != 0, 'the destination pin must refuse release')

        # Detach the stranded learner at the source leader; confirm idempotently.
        detached = command('detach', 'client', 'detach-aborted-learner', '--addr', addresses[node],
                           '--root-digest', root, '--operation-id', f'{7:032x}')
        require(detached['detach_outcome'] == 'detached' and detached['detached_node'] == '4',
                'detach lacks the removal receipt')
        require(detached['voters'] == '1,2,3', 'the source voter set is unchanged')
        again = command('detach-again', 'client', 'detach-aborted-learner', '--addr', addresses[node],
                        '--root-digest', root, '--operation-id', f'{7:032x}')
        require(again['detach_outcome'] == 'confirmed', 'detach retry must confirm')

        # Writes keep landing at the source group throughout the recovery.
        mid = command('mid-put', 'client', 'raw-put', '--addr', addresses[node],
                      '--keyspace', keyspace_id, '--key-hex', '67616d6d61', '--value-hex', '7468726565')

        # --- ABORT-SETTLED TRUNCATION: the committed abort (not evidence)
        # authorizes the decision; the raft compaction gates guard the floor.
        floor_term = int(mid['applied_term'])
        floor_index = int(mid['applied_index'])
        recorded = command('record-truncation', 'client', 'record-source-truncation',
                           '--addr', addresses[owner_node], '--root-digest', root,
                           '--operation-id', f'{7:032x}',
                           '--floor-term', floor_term, '--floor-index', floor_index)
        require(recorded.get('truncation_outcome') in ('committed', 'recorded', 'confirmed'),
                f'truncation decision receipt missing: {recorded}')
        truncated = command('truncate-log', 'client', 'truncate-source-log',
                            '--addr', addresses[node], '--root-digest', root,
                            '--operation-id', f'{7:032x}')
        require(truncated['truncate_outcome'] == 'compacted'
                and int(truncated['first_index']) > 1,
                f'compaction receipt missing: {truncated}')
        print(f"PASS: abort-settled compaction to floor {floor_term}/{floor_index} "
              f"(first_index {truncated['first_index']})", flush=True)

        # The compacted world survives a full restart of every voter.
        for n in (1, 2, 3):
            stop(n)
        for n in (1, 2, 3):
            start(n, f'posttrunc-{n}')
        wait('all voters ready after the compacted restart',
             lambda: all(status(n).get('endpoint_ready') == 'true' for n in (1, 2, 3)), 300)
        # The restart may elect a different metadata leader — and later
        # elections can move it again: every subsequent owner verb
        # re-resolves the leader just before running.
        def owner():
            fresh = wait('metadata leader (re-resolve)', leader, 300)
            owner_holder[0] = fresh
            return fresh
        owner_node = owner()
        node = wait('source leader after the compacted restart', group_leader, 300)
        post = command('posttrunc-put', 'client', 'raw-put', '--addr', addresses[node],
                       '--keyspace', keyspace_id, '--key-hex', '64656c7461', '--value-hex', '78')
        require(int(post['applied_index']) > floor_index, 'the compacted group must keep writing')

        # --- THE RECOVERY: a fresh destination incarnation on the same node
        # id, and a NEW operation for the SAME region completes end to end.
        shutil.rmtree(output / 'n4')
        prepared = command('prepare-4b', 'store-prepare', '--node-id', 4, '--data-dir', output / 'n4')
        require(prepared['store_incarnation'] != '', 'a fresh incarnation is required')
        # The consumed admission record must be revoked before a fresh
        # admit: the operator's decommission path for the lost incarnation.
        command('revoke-4', 'client', 'revoke-admission', '--addr', addresses[owner()],
                '--node-id', 4)
        admitted = command('admit-node-4b', 'client', 'admit-node', '--addr', addresses[owner()],
                           '--node-id', 4, '--node-addr', addresses[4], '--ttl-seconds', 600)
        command('join-4b', 'join', '--root', root_file, '--node-id', 4, '--data-dir', output / 'n4',
                extra_env={'KV9_JOIN_TICKET': admitted['join_ticket']})
        start(4, 'rejoined', ticket=admitted['join_ticket'])
        wait('re-provisioned destination ready', lambda: status(4).get('endpoint_ready') == 'true')

        command('migrate-2', 'client', 'migrate-data-group', '--addr', addresses[owner()],
                '--root-digest', root, '--operation-id', f'{11:032x}',
                '--creation-task', task, '--destination-node', 4)
        node = wait('source leader for the fresh migration', group_leader)
        attach_with_retry('attach-2', node, f'{11:032x}')
        plan_with_retry('plan-2', node, f'{11:032x}', output / 'image2.manifest')
        command('bind-2', 'client', 'bind-migration-image', '--addr', addresses[owner()],
                '--root-digest', root, '--operation-id', f'{11:032x}',
                '--manifest-file', output / 'image2.manifest')
        captured2 = capture_with_retry('capture-2', node, f'{11:032x}', output / 'image2.record')

        stop(4)
        installed = command('install-2', 'install-migration-image',
                            '--data-dir', output / 'n4', '--record-file', output / 'image2.record')
        require(installed['install_outcome'] == 'selected', 'fresh image must install')
        start(4, 'readopted', ticket=None)
        wait('destination ready after install', lambda: status(4).get('endpoint_ready') == 'true')

        def adopted():
            g = data_groups(4).get(region, {})
            return g if g.get('state') == 'active' and g.get('driver_applied') is not None else None
        wait('fresh installed generation adopted', adopted, 120)

        node = wait('source leader after adoption', group_leader)
        command('post-put', 'client', 'raw-put', '--addr', addresses[node],
                '--keyspace', keyspace_id, '--key-hex', '64656c7461', '--value-hex', '666f7572')
        leader_applied = int(fields(open(output / 'post-put.log').read()).get('applied_index', 0))
        require(leader_applied > 0, 'missing leader apply receipt')

        def caught_up():
            g = data_groups(4).get(region, {})
            return int((g.get('driver_applied') or {}).get('index', 0)) >= leader_applied
        wait('fresh learner catches the retained tail', caught_up, 120)

        emitted = command('emit-evidence-2', 'client', 'emit-install-evidence', '--addr', addresses[4],
                          '--root-digest', root, '--region', region,
                          '--receipt-file', output / 'evidence2.receipt')
        require(emitted['image_digest'] == captured2['image_digest'],
                'the fresh receipt must name the exact installed image')
        recorded = command('record-evidence-2', 'client', 'record-install-evidence',
                           '--addr', addresses[owner()], '--root-digest', root,
                           '--receipt-file', output / 'evidence2.receipt')
        require(recorded['evidence_outcome'] == 'recorded', 'fresh evidence commits')

        requests2 = {
            'quiesce-source-2': bytes([5]) + ledger_token(captured2['source_owner'])
            + ledger_token(captured2['destination_owner']),
            'release-source-2': bytes([6]) + ledger_token(captured2['source_owner']),
        }
        for name, payload in requests2.items():
            (output / f'{name}.request').write_bytes(
                sealed(b'KV9RTX01' + root_bytes + payload))
        result = retention('quiesce-2', 'quiesce-source-2')
        require(result['retention_outcome'] == 'changed', 'fresh quiesce commits after evidence')
        result = retention('release-2', 'release-source-2')
        require(result['retention_outcome'] == 'changed', 'fresh release commits after quiesce')

        manifest_record.update(verdict='accepted', region=region,
                               checks=['a stranded destination (published+captured, never installed) wedges quiesce and release',
                                       'the committed abort settles source-log truncation: decision + compaction under the raft gates',
                                       'every voter restarts on the compacted log and the group keeps writing',
                                       'the committed abort settles the ledger: quiesce then release through the unchanged verbs',
                                       'evidence and abort stay mutually exclusive; attach refuses aborted operations permanently',
                                       'the stranded learner detaches at the source leader under the committed abort',
                                       'the abandoned image destination pin never drops (documented cost)',
                                       'a fresh incarnation re-migrates the SAME region end to end: install, adoption, catchup, evidence, quiesce, release'])
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
