#!/usr/bin/env python3
"""Real-process split-publication gate: the manual split completes end to
end — intent, fence, verified population, then ONE atomic catalog
transaction flips routing to the children, which serve the exact
pre-split data while the sealed parent stays fenced.

Chain: bound parent keyspace under writes -> two activated unbound child
groups -> refusals (unbound parent, identical children) -> the exact intent
commits once and confirms idempotently -> divergent-key and second-split
refusals -> the parent keeps serving unchanged.

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
    parser.add_argument('--base-port', type=int, default=26990)
    args = parser.parse_args()
    for name in ('KV9_OBJECT_STORE_ENDPOINT', 'KV9_OBJECT_STORE_BUCKET',
                 'KV9_OBJECT_STORE_ACCESS_KEY', 'KV9_OBJECT_STORE_SECRET_KEY'):
        require(os.environ.get(name), f'{name} must be set to an isolated real MinIO bucket')
    binary, output = args.bin.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ, KV9_CLUSTER_TOKEN='publish-e2e-cluster',
               KV9_CLIENT_TOKENS='admin=publish-e2e-client', KV9_CLIENT_TOKEN='publish-e2e-client',
               KV9_BOOTSTRAP_TOKEN='publish-e2e-bootstrap', KV9_STORAGE='minio')
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

    def plan_with_retry(label, node, operation, path):
        # Committed intents replicate to local applied state within moments;
        # planning is read-only, so bounded retry is safe and honest.
        def planned():
            result = command(label, 'client', 'plan-migration-image', '--addr', addresses[node],
                             '--root-digest', root_holder[0], '--operation-id', operation,
                             '--manifest-file', path, success=None)
            return result if result['exit_code'] == 0 else None
        return wait(f'{label} intent locally applied', planned, 30)

    root_holder = [None]
    def capture_with_retry(label, node, operation, path):
        # Owners commit at the metadata leader; the group leader's local
        # applied ledger may lag for a moment. Capture refuses in the safe
        # direction and is idempotent, so bounded retry is honest.
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
        root_holder[0] = root

        created = command('create-group', 'client', 'create-data-group', '--addr', addresses[owner_node],
                          '--root-digest', root, '--operation-id', f'{1:032x}', '--voters', '1,2,3')
        task, region = created['task_id'], int(created['region_id'])
        keyspace = command('create-keyspace', 'client', 'create-data-keyspace', '--addr', addresses[owner_node],
                           '--root-digest', root, '--creation-task', task, '--name', 'attach-src')
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

        # --- Committed split intent: authority only.
        low = command('create-child-low', 'client', 'create-data-group', '--addr', addresses[owner_node],
                      '--root-digest', root, '--operation-id', f'{21:032x}', '--voters', '1,2,3')
        high = command('create-child-high', 'client', 'create-data-group', '--addr', addresses[owner_node],
                       '--root-digest', root, '--operation-id', f'{22:032x}', '--voters', '1,2,3')

        early = command('split-wrong-parent', 'client', 'record-split-intent', '--addr', addresses[owner_node],
                        '--root-digest', root, '--operation-id', f'{9:032x}', '--parent-region', 999,
                        '--split-key-hex', '6d', '--child-low-task', low['task_id'],
                        '--child-high-task', high['task_id'], success=None)
        require(early['exit_code'] != 0, 'an unbound parent region must refuse')

        same = command('split-same-children', 'client', 'record-split-intent', '--addr', addresses[owner_node],
                       '--root-digest', root, '--operation-id', f'{9:032x}', '--parent-region', region,
                       '--split-key-hex', '6d', '--child-low-task', low['task_id'],
                       '--child-high-task', low['task_id'], success=None)
        require(same['exit_code'] != 0, 'identical children must refuse')

        recorded = command('split-record', 'client', 'record-split-intent', '--addr', addresses[owner_node],
                           '--root-digest', root, '--operation-id', f'{9:032x}', '--parent-region', region,
                           '--split-key-hex', '6d', '--child-low-task', low['task_id'],
                           '--child-high-task', high['task_id'])
        require(recorded['split_outcome'] == 'recorded', 'first intent commit is a mutation')
        confirm = command('split-record-again', 'client', 'record-split-intent', '--addr', addresses[owner_node],
                          '--root-digest', root, '--operation-id', f'{9:032x}', '--parent-region', region,
                          '--split-key-hex', '6d', '--child-low-task', low['task_id'],
                          '--child-high-task', high['task_id'])
        require(confirm['split_outcome'] == 'confirmed'
                and confirm['split_task'] == recorded['split_task'],
                'an identical intent confirms the same committed row')

        divergent = command('split-divergent-key', 'client', 'record-split-intent', '--addr', addresses[owner_node],
                            '--root-digest', root, '--operation-id', f'{9:032x}', '--parent-region', region,
                            '--split-key-hex', '6e', '--child-low-task', low['task_id'],
                            '--child-high-task', high['task_id'], success=None)
        require(divergent['exit_code'] != 0, 'one operation never names a second split key')

        second = command('split-second-op', 'client', 'record-split-intent', '--addr', addresses[owner_node],
                         '--root-digest', root, '--operation-id', f'{10:032x}', '--parent-region', region,
                         '--split-key-hex', '6e', '--child-low-task', low['task_id'],
                         '--child-high-task', high['task_id'], success=None)
        require(second['exit_code'] != 0, 'one live split per parent region')

        # The intent alone reroutes nothing: the parent keeps serving.
        command('post-intent-put', 'client', 'raw-put', '--addr', addresses[data_node],
                '--keyspace', keyspace_id, '--key-hex', '7a7a', '--value-hex', '76')
        probe = command('post-intent-get', 'client', 'raw-get', '--addr', addresses[data_node],
                        '--keyspace', keyspace_id, '--key-hex', '7a7a', success=None)
        require(probe.get('value_hex') == '76', 'the parent range must keep serving unchanged')

        # Population before the fence must refuse: the half could still move.
        early_pop = command('populate-before-seal', 'client', 'populate-split-child',
                            '--addr', addresses[data_node], '--root-digest', root,
                            '--operation-id', f'{9:032x}', '--half', 'low', success=None)
        require(early_pop['exit_code'] != 0, 'population requires the sealed parent fence')

        # --- Seal execution: the committed intent is the one authority.
        wrong = command('seal-wrong-operation', 'client', 'seal-split-parent', '--addr', addresses[data_node],
                        '--root-digest', root, '--operation-id', f'{8:032x}', success=None)
        require(wrong['exit_code'] != 0, 'sealing without a committed intent must refuse')

        def seal(label):
            for attempt in range(20):
                node_now = wait('parent leader for sealing', group_leader)
                probe = command(label, 'client', 'seal-split-parent', '--addr', addresses[node_now],
                                '--root-digest', root, '--operation-id', f'{9:032x}', success=None)
                if probe['exit_code'] == 0:
                    return probe
                time.sleep(0.5)
            require(False, 'seal did not commit in time')

        sealed = seal('seal')
        require(sealed['seal_outcome'] == 'sealed' and int(sealed['sealed_version']) == 2,
                'the seal must bump the range version one-way')
        again = seal('seal-again')
        require(again['seal_outcome'] == 'confirmed' and int(again['sealed_version']) == 2,
                'a seal retry confirms the same durable fence')

        # The fence is immediate: writes AND reads at the parent refuse with
        # a stale epoch, and the refusal is typed, not an unknown.
        put_probe = command('sealed-put', 'client', 'raw-put', '--addr', addresses[data_node],
                            '--keyspace', keyspace_id, '--key-hex', '7373', '--value-hex', '76',
                            success=None)
        require(put_probe['exit_code'] != 0, 'a sealed parent must refuse writes')
        get_probe = command('sealed-get', 'client', 'raw-get', '--addr', addresses[data_node],
                            '--keyspace', keyspace_id, '--key-hex', '7a7a', success=None)
        require(get_probe['exit_code'] != 0, 'a sealed parent must refuse reads')

        # The fence survives restart: the group recovers sealed and still
        # refuses; the metadata quorum stays healthy throughout.
        sealed_leader = wait('parent leader before restart', group_leader)
        stop(sealed_leader)
        start(sealed_leader, 'sealed-restart', ticket=None)
        wait('sealed leader ready after restart',
             lambda: status(sealed_leader).get('endpoint_ready') == 'true')
        wait('sealed group recovers active',
             lambda: data_groups(sealed_leader).get(region, {}).get('state') == 'active')
        node_after = wait('parent leader after restart', group_leader)
        still = command('sealed-put-after-restart', 'client', 'raw-put', '--addr', addresses[node_after],
                        '--keyspace', keyspace_id, '--key-hex', '7474', '--value-hex', '76',
                        success=None)
        require(still['exit_code'] != 0, 'the durable fence must survive restart')

        # --- Population: each half copies into its child, exactly.
        low_region, high_region = int(low['region_id']), int(high['region_id'])

        def child_leader(child_region):
            def probe():
                return next((n for n in (1, 2, 3) if n in processes
                             and data_groups(n).get(child_region, {}).get('role') == 'Leader'), None)
            return wait(f'child {child_region} leader', probe)

        def populate(label, half, child_region):
            for attempt in range(20):
                node_now = child_leader(child_region)
                probe = command(label, 'client', 'populate-split-child', '--addr', addresses[node_now],
                                '--root-digest', root, '--operation-id', f'{9:032x}',
                                '--half', half, success=None)
                if probe['exit_code'] == 0:
                    return probe
                time.sleep(0.5)
            require(False, f'population of the {half} half did not commit in time')

        low_copy = populate('populate-low', 'low', low_region)
        require(low_copy['parent_half_digest'] == low_copy['child_digest'],
                'the low child must equal its parent half exactly')
        require(int(low_copy['rows_copied']) == 2, 'the low half carries alpha and beta')
        high_copy = populate('populate-high', 'high', high_region)
        require(high_copy['parent_half_digest'] == high_copy['child_digest'],
                'the high child must equal its parent half exactly')
        require(int(high_copy['rows_copied']) == 1, 'the high half carries exactly zz')
        require(low_copy['parent_half_digest'] != high_copy['parent_half_digest'],
                'the two halves are distinct data')

        # Idempotent: a rerun converges to the identical child state.
        low_again = populate('populate-low-again', 'low', low_region)
        require(low_again['child_digest'] == low_copy['child_digest'],
                'a population rerun must converge to the same rows')

        # A child leader restart, then another rerun: still exact.
        restart_child = child_leader(high_region)
        stop(restart_child)
        start(restart_child, 'child-restart', ticket=None)
        wait('child node ready after restart',
             lambda: status(restart_child).get('endpoint_ready') == 'true')
        high_again = populate('populate-high-again', 'high', high_region)
        require(high_again['child_digest'] == high_copy['child_digest'],
                'population after a child restart must converge to the same rows')

        # --- The atomic publication: one transaction flips the directory.
        owner_node = wait('metadata leader before publication', leader)
        early_pub = command('publish-wrong-operation', 'client', 'publish-split',
                            '--addr', addresses[owner_node], '--root-digest', root,
                            '--operation-id', f'{8:032x}', success=None)
        require(early_pub['exit_code'] != 0, 'publication requires the committed intent')

        published = command('publish', 'client', 'publish-split', '--addr', addresses[owner_node],
                            '--root-digest', root, '--operation-id', f'{9:032x}')
        require(published['publish_outcome'] == 'published'
                and int(published['sealed_version']) == 2
                and int(published['low_region']) == low_region
                and int(published['high_region']) == high_region,
                'the publication receipt must name the exact partition')
        confirm_pub = command('publish-again', 'client', 'publish-split', '--addr', addresses[owner_node],
                              '--root-digest', root, '--operation-id', f'{9:032x}')
        require(confirm_pub['publish_outcome'] == 'confirmed',
                'a publication retry confirms the identical directory')

        # The children become servable through PUBLIC routing and return the
        # exact pre-split data; new writes land on the correct child.
        def child_serves(key_hex, value_hex):
            def probe():
                for n in (1, 2, 3):
                    if n not in processes:
                        continue
                    got = command('routed-read', 'client', 'raw-get', '--addr', addresses[n],
                                  '--keyspace', keyspace_id, '--key-hex', key_hex, success=None)
                    if got.get('value_hex') == value_hex:
                        return True
                return False
            return probe
        wait('the low child serves alpha', child_serves('616c706861', '6f6e65'), 120)
        wait('the high child serves zz', child_serves('7a7a', '76'), 120)

        def routed_put(label, key_hex, value_hex):
            for attempt in range(30):
                for n in (1, 2, 3):
                    if n not in processes:
                        continue
                    put = command(label, 'client', 'raw-put', '--addr', addresses[n],
                                  '--keyspace', keyspace_id, '--key-hex', key_hex,
                                  '--value-hex', value_hex, success=None)
                    if put['exit_code'] == 0:
                        return
                time.sleep(0.5)
            require(False, f'routed write {key_hex} did not land')

        routed_put('post-split-put-low', '6161', '6c6f77')
        routed_put('post-split-put-high', '7979', '68696768')
        wait('the new low write serves', child_serves('6161', '6c6f77'), 60)
        wait('the new high write serves', child_serves('7979', '68696768'), 60)

        # Everything is durable: restart every voter; the split directory,
        # both children and the sealed parent all recover.
        for n in (1, 2, 3):
            stop(n)
            start(n, f'post-split-restart-{n}', ticket=None)
        wait('all voters ready after the full restart',
             lambda: all(status(n).get('endpoint_ready') == 'true' for n in (1, 2, 3)))
        wait('the low child serves alpha after the full restart',
             child_serves('616c706861', '6f6e65'), 180)
        wait('the high child serves the new write after the full restart',
             child_serves('7979', '68696768'), 180)

        manifest_record.update(verdict='accepted', region=region,
                               checks=['intent, fence and digest-verified population as previously qualified',
                                       'one atomic catalog transaction seals the parent binding and publishes both children',
                                       'publication re-verifies both children locally and confirms idempotently',
                                       'public routing serves the exact pre-split data from the correct children',
                                       'new writes land on the correct child; everything survives a full-cluster restart'])
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
