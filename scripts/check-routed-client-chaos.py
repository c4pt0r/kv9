#!/usr/bin/env python3
"""Independent bounded reader for the routed-client single-host Chaos gate."""
import argparse
import hashlib
import json
from pathlib import Path
import stat
import re
import tarfile

MAX_JSON = 16 * 1024**2
MAX_HISTORY = 32 * 1024**2
MAX_OPERATIONS = 256
PHASES = ('baseline', 'discovery-seed-partition', 'container-kill', 'leader-partition', 'client-endpoint-partition', 'healed')


def require(ok, message):
    if not ok:
        raise ValueError(message)


def integer(x, low=0):
    return type(x) is int and low <= x < 2**64


def unique(pairs):
    d = {}
    for k, v in pairs:
        require(k not in d, 'duplicate JSON key')
        d[k] = v
    return d


def read(path, cap=MAX_JSON):
    p = Path(path)
    require(p.is_file() and not p.is_symlink() and p.stat().st_size <= cap, 'unsafe/large JSON input')
    return json.loads(p.read_bytes(), object_pairs_hook=unique)


def pin(path):
    p = Path(path)
    with p.open('rb') as f:
        h = hashlib.file_digest(f, 'sha256').hexdigest()
    return {'path': str(p), 'bytes': p.stat().st_size, 'sha256': h}


def lines(path):
    p = Path(path)
    require(p.stat().st_size <= MAX_HISTORY, 'history byte limit')
    return [json.loads(x, object_pairs_hook=unique) for x in p.read_bytes().splitlines()]


def octets(value):
    require(type(value) is list and len(value) <= 65536 and
            all(type(x) is int and 0 <= x <= 255 for x in value), 'invalid byte vector')
    return bytes(value)


def retry_safe(failure, phase, read=False):
    require(type(failure) is dict, 'nonterminal successful/absent failure')
    if phase == 'lookup' or read:
        return True  # Read-only lookup can advance seeds; data uncertainty cannot.
    return failure.get('kind') == 'scope_refused' or (failure.get('kind') == 'rpc' and failure.get('reason', {}).get('kind') == 'not_leader')


def report_check(row, expected_region):
    r = row['report']; a = r['attempts']; outcome = r['outcome']['kind']
    require(r['operation'] == row['operation']['kind'], 'reported operation differs')
    require(integer(r['elapsed_ns']) and r['elapsed_ns'] <= 6_000_000_000 and integer(row['invoked_unix_ns'], 1) and
            integer(row['finished_unix_ns'], 1) and row['finished_unix_ns'] >= row['invoked_unix_ns'], 'operation clock')
    require(type(a) is list and 0 < len(a) <= 16, 'missing/excess attempts')
    require(r['stop'] in ('terminal', 'attempt_limit', 'deadline'), 'stop vocabulary')
    for number, attempt in enumerate(a, 1):
        require(attempt['ordinal'] == number and type(attempt['ordinal']) is int and
                attempt['phase'] in ('lookup', 'data') and attempt['node_id'] in (1, 2, 3) and
                integer(attempt['node_id'], 1) and integer(attempt['elapsed_ns']), 'attempt schema/order')
        route = attempt.get('route')
        if route is not None:
            require(route['region_id'] == expected_region and integer(route['region_id'], 1) and
                    integer(route['epoch_conf_ver'], 1) and integer(route['epoch_version'], 1), 'wrong route identity')
            require(route['epoch_conf_ver'] == route['epoch_version'] == 1, 'unexpected initial-route epoch')
            digest = route['binding_digest']
            require((type(digest) is str and len(digest) == 64 and all(c in '0123456789abcdef' for c in digest)) or
                    (type(digest) is list and len(octets(digest)) == 32), 'binding digest')
        if number < len(a):
            # Successful lookup installs a route and is allowed to precede data.
            require((attempt['phase'] == 'lookup' and attempt.get('failure') is None) or
                    retry_safe(attempt.get('failure'), attempt['phase'], row['operation']['kind'] in ('get', 'batch_get')), 'uncertain data attempt replayed')
    require(outcome in ('success', 'refused', 'unknown_write', 'read_failure', 'lookup_failure', 'client_rejected'), 'outcome vocabulary')
    write = row['operation']['kind'] in ('put', 'delete', 'batch_put')
    require(outcome != 'unknown_write' or write, 'unknown read')
    require(outcome != 'read_failure' or not write, 'write labeled failed read')
    require(outcome != 'client_rejected', 'fixture command was rejected by client')
    if outcome == 'success':
        require(a[-1]['phase'] == 'data' and a[-1].get('failure') is None and
                a[-1].get('route') is not None, 'success lacks exact data route')
        v = r['outcome']['value']
        if write:
            require(v['kind'] == 'applied' and integer(v['term'], 1) and integer(v['index'], 1), 'missing write fence')
        else:
            require(v['kind'] == row['operation']['kind'], 'wrong successful read type')
    if outcome == 'unknown_write':
        require(a[-1]['phase'] == 'data' and a[-1].get('failure') is not None, 'unknown write without data attempt')
    return outcome


def history_check(commands, output, config, region):
    require(output and output[0]['kind'] == 'start' and output[0]['config'] == config and
            integer(output[0]['pid'], 1), 'client header/config')
    require(0 < len(commands) <= MAX_OPERATIONS and len(output) == len(commands) + 1, 'incomplete/extra history')
    states = {}; outcomes = {}; phases = {}; fences = []; previous = 0; ids = set(); written = set(); previous_id = 0; bindings = set()
    for command, row in zip(commands, output[1:]):
        require(command['id'] == row['id'] and integer(row['id'], 1) and row['id'] not in ids and
                row['kind'] == 'operation' and row['operation'] == command['operation'], 'history invocation/return pairing')
        require(row['id'] > previous_id, 'operation ID reset'); previous_id = row['id']
        ids.add(row['id'])
        require(row['invoked_unix_ns'] >= previous, 'sequential client calls overlap/reset')
        previous = row['finished_unix_ns']
        outcome = report_check(row, region)
        for attempt in row['report']['attempts']:
            if attempt.get('route') is not None: bindings.add(json.dumps(attempt['route'], sort_keys=True))
        require(len(bindings) <= 1, 'route binding changed during fixed-group history')
        outcomes[outcome] = outcomes.get(outcome, 0) + 1
        phase = command['phase']; require(phase in PHASES, 'unknown phase')
        phase_count = phases.setdefault(phase, {'success_reads': 0, 'success_writes': 0, 'operations': 0})
        phase_count['operations'] += 1
        op = row['operation']; kind = op['kind']
        # This gate uses unique-key point operations. General unknown atomic
        # batches require correlated alternatives and a different history model.
        require(kind in ('put', 'get'), 'this gate requires single-key put/get history')
        if kind in ('put', 'delete', 'batch_put'):
            updates = [(octets(op['key']), None if kind == 'delete' else octets(op['value']))] if kind != 'batch_put' else [(octets(k), octets(v)) for k, v in op['pairs']]
            require(len({k for k, _ in updates}) == len(updates), 'duplicate batch keys')
            # Every fixture mutation has fresh keys, so it cannot replay an unknown write.
            require(all(k not in written for k, _ in updates), 'logical write key reused')
            written.update(k for k, _ in updates)
            for k, v in updates:
                before = states.get(k, {None})
                states[k] = {v} if outcome == 'success' else before | {v} if outcome == 'unknown_write' else before
            if outcome == 'success':
                phase_count['success_writes'] += 1
                fences.append(row['report']['outcome']['value'])
        elif kind in ('get', 'batch_get'):
            keys = [octets(op['key'])] if kind == 'get' else [octets(k) for k in op['keys']]
            if outcome == 'success':
                result = row['report']['outcome']['value']
                values = [result['value']] if kind == 'get' else result['values']
                require(len(keys) == len(values), 'read cardinality')
                for k, value in zip(keys, values):
                    v = None if value is None else octets(value)
                    require(v in states.get(k, {None}), 'read violates complete sequential history')
                    states[k] = {v}
                phase_count['success_reads'] += 1
        else:
            raise ValueError('unsupported operation')
    return {'calls': len(commands), 'outcomes': outcomes, 'phases': phases, 'acknowledged_fences': fences,
            'bindings': list(bindings), 'pid': output[0]['pid'], 'complete_history_checked': True}


def fault_check(row, namespace, namespace_uid):
    f = row['fault']; spec = f['spec']; meta = f['metadata']
    require(meta['namespace'] == namespace and integer(row['injected_observed_ns'], 1) and
            row['namespace_uid'] == namespace_uid and not meta.get('deletionTimestamp'), 'fault ownership')
    require(spec['selector']['namespaces'] == [namespace], 'unscoped fault selector')
    require(any(c['type'] == 'AllInjected' and c['status'] == 'True' for c in f['status']['conditions']), 'no injection')
    require(row['phase'] in PHASES[1:5], 'fault phase')
    after = row['fault_after_progress']
    require(after['metadata']['uid'] == meta['uid'] and not after['metadata'].get('deletionTimestamp') and
            any(c['type'] == 'AllInjected' and c['status'] == 'True' for c in after['status']['conditions']) and
            row['progress_finished_ns'] >= row['injected_observed_ns'], 'fault did not remain installed')
    selector = spec['selector']
    require(set(selector['pods']) == {namespace} and selector['pods'][namespace], 'exact Pod selector missing')
    if row['phase'] == 'container-kill':
        require(f['kind'] == 'PodChaos' and spec['action'] == 'container-kill', 'wrong kill action')
        before, after = row['before'], row['after']
        require(before['pod_uid'] == after['pod_uid'] and before['pvc_uid'] == after['pvc_uid'] and
                before['store_incarnation'] == after['store_incarnation'] and
                before['container_id'] != after['container_id'] and before['process'] != after['process'] and
                after['last_exit_code'] == 137 and after['old_container_id'] == before['container_id'], 'kill did not replace exact owner')
    else:
        require(f['kind'] == 'NetworkChaos' and spec['action'] == 'partition' and spec['direction'] == 'both', 'wrong network fault')
        require(spec['target']['selector']['namespaces'] == [namespace], 'unscoped network target')
        blocked = row['effect']['blocked']; connected = row['effect']['connected']
        require(blocked and all(x['exit_code'] in (1, 124) for x in blocked), 'network fault effect missing')
        require(connected and all(x['exit_code'] == 0 for x in connected), 'required surviving edges unavailable')
        if row['phase'] == 'client-endpoint-partition':
            require(len(connected) == 6 and len({(x['from'], x['to']) for x in connected}) == 6, 'voter connectivity not preserved')
            require(selector['pods'][namespace] == ['client-0'], 'wrong isolated client')
            for before, after in zip(row['groups_before'], row['groups_after']):
                require(before['node'] == after['node'] and before['process'] == after['process'], 'voter replaced during client-only fault')
                require([(g['region'], g['term'], g['leader']) for g in before['groups']] ==
                        [(g['region'], g['term'], g['leader']) for g in after['groups']], 'client-only fault altered leader epoch')
        elif row['phase'] == 'leader-partition':
            control = row['minority_control']; before = control['before']
            require(control['retried'] is False and control['exit_code'] == 1 and control['stdout'] == '' and
                    re.fullmatch(r'not_leader=true leader_node_id=(?:[1-9][0-9]*|unknown)\n(?:command terminated with exit code 1\n)?', control['stderr']), 'minority lacked exclusive refusal')
            require(row['injected_observed_ns'] <= control['started_ns'] <= control['finished_ns'] <= row['progress_finished_ns'], 'minority probe outside partition')
            require(before['node'] == control['node'] and any(g['region'] == control['region'] and g['role'] in ('Follower', 'Candidate') for g in before['groups']), 'minority control selected wrong group/process')
        elif row['phase'] == 'discovery-seed-partition':
            require(selector['pods'][namespace] == ['client-1'] and row['seed'] not in (row['data_leader_before'], row['metadata_leader_before']) and
                    len(connected) == 6 and blocked[0]['to'] == row['seed'], 'discovery seed isolation scope')
    return True


def verify_archive(path, expected):
    require(pin(path)['sha256'] == expected['sha256'] and Path(path).stat().st_size == expected['bytes'], 'archive hash/size')
    seen = {}; total = 0; end = 0
    with tarfile.open(path, 'r:') as archive:
        for m in archive:
            end = ((m.offset_data + m.size + 511)//512)*512
            name = m.name.removeprefix('./')
            require(not name.startswith('/') and '..' not in Path(name).parts and name not in seen, 'unsafe/duplicate tar member')
            require(m.isdir() or m.isfile(), 'archive contains nonordinary member')
            if m.isfile():
                require(0 <= m.size <= 128 * 1024**2, 'archive member limit'); total += m.size
                require(total <= 128 * 1024**2, 'archive decoded cap')
                stream = archive.extractfile(m); h = hashlib.file_digest(stream, 'sha256').hexdigest()
                seen[name] = {'bytes': m.size, 'sha256': h}
            else:
                seen[name] = {'directory': True}
    with Path(path).open('rb') as f:
        f.seek(end); trailer = f.read(10240 + 1024)
        require(len(trailer) >= 1024 and len(trailer) % 512 == 0 and not any(trailer) and not f.read(1), 'archive EOF/trailer differs')
    require(seen == expected['members'], 'archive full-member inventory')
    return total


def lifetime_check(states, owned):
    require(len(states) == len(owned['server_container_ids']) and
            {s['id'] for s in states} == {c.removeprefix('containerd://') for c in owned['server_container_ids']}, 'container lifetime inventory')
    for s in states:
        require(s['process_absent'] is True and s['process'] == owned['kind_processes']['containerd://'+s['id']], 'server process absence not checked')
        if s['state'] == 'GARBAGE_COLLECTED':
            require(s['exitCode'] is None and s['cri_absent'] is True and s['task_absent'] is True, 'GC cannot claim observed exit')
        else:
            require(s['state'] == 'CONTAINER_EXITED' and integer(s['exitCode']), 'server exit not observed')


def audit(run, require_cleanup=False):
    run = Path(run); summary = read(run/'result.json'); setup = read(run/'setup.json')
    require(summary['runtime_complete'] is True and summary['single_host_only'] is True and
            summary['namespace'] == setup['namespace'] and summary['namespace_uid'] == setup['namespace_uid'], 'runtime/namespace not complete')
    histories = []
    require(len(setup['clients']) == 2 and len({x['region'] for x in setup['clients']}) == 2 and
            len({x['config']['keyspace_id'] for x in setup['clients']}) == 2, 'two distinct group/keyspace clients required')
    for c in setup['clients']:
        config = c['config']
        require(config['root_digest'] == list(bytes.fromhex(setup['root_digest'])) and config['tenant_id'] == 0 and
                config['version'] == 1 and config['max_in_flight'] == 4 and config['max_attempts'] == 16 and
                config['deadline_ms'] == 5000 and config['probe_timeout_ms'] == 300 and config['retry_backoff_ms'] == 150 and
                config['cache_capacity'] == 8 and {p['node_id'] for p in config['seeds']} == {1,2,3}, 'client root/tenant/budget/seed scope')
        d = run/c['name']; h = history_check(lines(d/'commands.jsonl'), lines(d/'stdout.jsonl'), c['config'], c['region'])
        exit_record = read(d/'exit.json')
        require(exit_record['exit_code'] == 0 and exit_record['remote_absent'] is True and
                exit_record['remote_identity'] == c['process'], 'client exit/lifetime')
        for phase in ('baseline', 'container-kill', 'leader-partition', 'healed'):
            counts = h['phases'].get(phase, {})
            require(counts.get('success_reads', 0) > 0 and counts.get('success_writes', 0) > 0, 'phase lacks client progress')
        histories.append(h)
    require(histories[0]['bindings'] and histories[1]['bindings'] and histories[0]['bindings'] != histories[1]['bindings'], 'groups share route identity')
    faults = [read(run/f'fault-{p}.json') for p in PHASES[1:5]]
    require(len({f['fault']['metadata']['uid'] for f in faults}) == 4, 'reused fault UID')
    for f in faults:
        fault_check(f, setup['namespace'], setup['namespace_uid'])
    for f in faults:
        for client_number, c in enumerate(setup['clients']):
            commands = lines(run/c['name']/'commands.jsonl'); outputs = lines(run/c['name']/'stdout.jsonl')[1:]
            window = [(cmd, out) for cmd, out in zip(commands, outputs) if cmd['phase'] == f['phase']]
            require(window, 'client absent from fault window')
            require(all(f['injected_observed_ns'] <= out['invoked_unix_ns'] <= out['finished_unix_ns'] <= f['progress_finished_ns'] for _, out in window), 'operation outside actual injection window')
            if f['phase'] == 'discovery-seed-partition' and client_number == 1:
                attempts = [a for _, out in window for a in out['report']['attempts']]
                require(attempts[0]['phase'] == 'lookup' and attempts[0]['node_id'] == f['seed'] and
                        attempts[0]['failure'] is not None, 'first seed was not actually unavailable')
                require(any(a['phase'] == 'lookup' and a['node_id'] != f['seed'] for a in attempts) and
                        any(out['report']['outcome']['kind'] == 'success' for _, out in window), 'no surviving-seed discovery progress')
            if f['phase'] == 'client-endpoint-partition' and client_number == 0:
                require(all(out['report']['outcome']['kind'] != 'success' for _, out in window), 'isolated client unexpectedly reached leader')
            elif f['phase'] == 'client-endpoint-partition':
                require(any(out['report']['outcome']['kind'] == 'success' and cmd['operation']['kind'] == 'put' for cmd, out in window), 'unaffected client lacks progress')
    for c in setup['clients']:
        outputs = lines(run/c['name']/'stdout.jsonl')[1:]
        writes = [(x['operation']['key'], x['operation']['value'], x['finished_unix_ns'], x['report']['outcome']['kind'])
                  for x in outputs if x['operation']['kind'] == 'put']
        final_fault_end = max(f['progress_finished_ns'] for f in faults)
        for key, value, finished, outcome in writes:
            allowed = [value] if outcome == 'success' else [None, value] if outcome == 'unknown_write' else [None]
            require(any(x['operation'] == {'kind': 'get', 'key': key} and x['invoked_unix_ns'] > max(finished, final_fault_end) and
                        x['report']['outcome']['kind'] == 'success' and x['report']['outcome']['value']['value'] in allowed
                        for x in outputs), 'submitted mutation lacks post-fault outcome-consistent readback')
    drains = read(run/'drains.json')
    require(len(drains['first']) == len(drains['second']) == 3, 'three fresh drains required')
    for before, after in zip(drains['first'], drains['second']):
        require(before['node'] == after['node'] and before['process'] == after['process'] and
                before['root_digest'] == after['root_digest'] == setup['root_digest'] and
                int(after['status']['metrics_export_successes']) > int(before['status']['metrics_export_successes']), 'drain freshness/identity')
        for snapshot in (before, after):
            require(snapshot['status']['fatal'] == '', 'fatal voter')
            for c, history in zip(setup['clients'], histories):
                selected = [g for g in snapshot['groups'] if g['region'] == c['region']]
                require(len(selected) == 1, 'drain group identity')
                group = selected[0]; applied = group['driver_applied']
                require(group['state'] == 'active' and group['error'] is None and applied['index'] == group['committed'] and
                        applied['term'] == group['term'], 'group not fully driver-applied')
                require(all(group['engine_applied'] >= a['index'] and applied['index'] >= a['index'] and applied['term'] >= a['term']
                            for a in history['acknowledged_fences']), 'group below acknowledged command prefix')
    archives = read(run/'archives.json'); require(len(archives) == 3, 'three stopped stores required')
    for a in archives:
        require(a['child_exit_code'] == 0, 'archive child failed')
        verify_archive(run/a['file'], a)
    if require_cleanup:
        final = read(run/'accepted.json'); cleanup = read(run/'cleanup.json'); protected = read(run/'protected.json'); owned = read(run/'owned-resources.json')
        require(final['runtime_complete'] is True and final['cleanup_complete'] is True and cleanup['complete'] is True and
                cleanup['namespace_absent'] is True and cleanup['namespace'] == setup['namespace'] and cleanup['namespace_uid'] == setup['namespace_uid'], 'cleanup incomplete')
        require(cleanup['protected_namespaces'] == protected['namespaces'] and cleanup['protected_faults'] == protected['faults'], 'historical resources changed')
        require(owned['cleanup_complete'] is True and all(x['namespace'] == setup['namespace'] and
                x['namespace_uid'] == setup['namespace_uid'] for x in (final, owned)), 'cleanup ownership mismatch')
        lifetime_check(cleanup['recorded_server_containers'], owned)
        require(owned['client_host_exits'] == [0, 0], 'local client wrapper exit')
    return {'complete': True, 'cleanup_checked': require_cleanup, 'scope': 'Single-host actual Chaos, two serial unique-key histories with explicit unknown alternatives; not general concurrent linearizability, scaling or full21 replacement',
            'histories': histories, 'faults': [f['phase'] for f in faults], 'archives': len(archives),
            'originals': [pin(run/'result.json'), pin(run/'setup.json')]}


def main():
    p = argparse.ArgumentParser(description=__doc__); p.add_argument('--run', type=Path, required=True); p.add_argument('--output', type=Path, required=True)
    a = p.parse_args(); require(not a.output.exists(), 'output already exists')
    try:
        result = audit(a.run, require_cleanup=True)
    except Exception as e:
        result = {'complete': False, 'failure': repr(e)}
        with a.output.open('x') as f: json.dump(result, f, indent=2); f.write('\n')
        raise
    with a.output.open('x') as f: json.dump(result, f, indent=2); f.write('\n')
    print(json.dumps(result))


if __name__ == '__main__':
    main()
