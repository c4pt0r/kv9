#!/usr/bin/env python3
"""Validate complete native-batch recorder evidence and independently check atomic histories.

This validates client artifacts, not server durability, receipt authenticity,
process identity or fault effects. The enclosing E2E/Chaos fixture binds those.
"""
import argparse
from collections import Counter
import hashlib
import json
import math
from pathlib import Path
import re

from workload_report import (OUTCOMES, PHASES, bounded, histogram, histogram_check,
                             keys, reason_population, require, sha, strict_json, uint)
from checker import History, check, coverage, verify_witness

KINDS = ['get', 'put', 'delete', 'batch_get', 'batch_put']
READS = {'get', 'batch_get'}
TRAFFIC_PHASES = (set(PHASES) - {'initialization', 'warmup', 'verify'}) | {
    'client-link-delay', 'client-link-loss', 'client-link-partition', 'client-stream-drop', 'quorum-loss'}
MAX_HISTORY = 268435456


def same_json(left, right):
    """JSON equality that cannot identify bool/float substitutions with ints."""
    if isinstance(left, dict) and isinstance(right, dict):
        return set(left) == set(right) and all(same_json(left[k], right[k]) for k in left)
    if type(left) is not type(right):
        return False
    if isinstance(left, list):
        return len(left) == len(right) and all(same_json(a, b) for a, b in zip(left, right))
    return left == right


def native_histogram_check(bucket):
    histogram_check(bucket)
    # The shared legacy checker compares percentile dictionaries by ordinary
    # equality, which would accept True for a one-nanosecond bound.
    for percentile in ('p50', 'p95', 'p99'):
        interval = bucket[percentile]
        if interval is not None:
            keys(interval, ['lower_ns', 'upper_ns'])
            uint(interval['upper_ns'], uint(interval['lower_ns']))


def config_check(c):
    keys(c, 'version client rpc_transport run_id keyspace_name workers seed keys batch_size value_bytes mix max_calls history_bytes measure_ms interval_ms'.split())
    uint(c['version'], 1, 1)
    require(c['rpc_transport'] in ('tonic_stream', 'tonic_unary', 'tarpc_tcp'), 'unknown native batch transport')
    for name in ('run_id', 'keyspace_name'):
        require(isinstance(c[name], str) and re.fullmatch('[A-Za-z0-9_-]{1,64}', c[name]), 'invalid workload name')
    client = c['client']
    keys(client, 'version peers keyspace_id epoch_conf_ver epoch_version max_in_flight max_attempts deadline_ms retry_backoff_ms'.split())
    for name in ('version', 'epoch_conf_ver', 'epoch_version'):
        uint(client[name], 1, 1)
    uint(client['keyspace_id'], 1, (1 << 24) - 1)
    require(isinstance(client['peers'], list) and 1 <= len(client['peers']) <= 32, 'invalid peer bound')
    ids, endpoints = set(), set()
    import ipaddress
    for peer in client['peers']:
        keys(peer, ['node_id', 'address']); uint(peer['node_id'], 1)
        require(isinstance(peer['address'], str), 'invalid peer address')
        host, port = peer['address'].rsplit(':', 1)
        endpoint = (ipaddress.ip_address(host.strip('[]')), int(port)); uint(endpoint[1], 1, 65535)
        require(peer['node_id'] not in ids and endpoint not in endpoints, 'duplicate peer identity')
        ids.add(peer['node_id']); endpoints.add(endpoint)
    uint(client['max_in_flight'], 1, 256); uint(client['max_attempts'], 1, 16)
    uint(client['deadline_ms'], 1, 30000); uint(client['retry_backoff_ms'], 0, client['deadline_ms'])
    uint(c['workers'], 1, client['max_in_flight']); uint(c['keys'], 1, 256)
    uint(c['batch_size'], 1, 256); uint(c['value_bytes'], 16, 8192); uint(c['seed'])
    require(isinstance(c['mix'], list) and len(c['mix']) == 5 and sum(uint(n, 0, 100) for n in c['mix']) == 100, 'invalid operation mix')
    uint(c['measure_ms'], 1, 3600000); uint(c['interval_ms'], 0, 1000)
    chunks = (c['keys'] + c['batch_size']) // c['batch_size']
    uint(c['max_calls'], 3 * chunks + c['workers'] + 2, 100000)
    allowance = 16384 + 8 * c['batch_size'] * (len(c['run_id']) + 17 + c['value_bytes'])
    uint(c['history_bytes'], 65536 + allowance * c['max_calls'], MAX_HISTORY)
    require(c['batch_size'] * (len(c['run_id']) + 17 + c['value_bytes'] + 32) + 32 <= 1048576, 'workload payload exceeds conservative message bound')
    return chunks


def key(c, i):
    return f'{c["run_id"]}:{i:016x}'.encode().hex()


def value(c, nonce, item):
    if nonce == 0 and item == c['keys']:
        return ''  # Immutable sentinel also covers present empty values.
    return (nonce.to_bytes(8, 'big') + item.to_bytes(8, 'big') + b'v' * (c['value_bytes'] - 16)).hex()


def generated(c, nonce):
    mask = (1 << 64) - 1
    hashed = ((nonce ^ c['seed']) + 0x9e3779b97f4a7c15) & mask
    hashed = ((hashed ^ (hashed >> 30)) * 0xbf58476d1ce4e5b9) & mask
    hashed = ((hashed ^ (hashed >> 27)) * 0x94d049bb133111eb) & mask
    hashed ^= hashed >> 31
    first = hashed % c['keys']
    slot = (nonce * 73 + 19 + c['seed'] % 100) % 100
    cumulative = 0
    for operation, weight in zip(KINDS, c['mix']):
        cumulative += weight
        if slot < cumulative:
            break
    args = {'keyspace': c['client']['keyspace_id']}
    if operation in ('get', 'delete'):
        args['key'] = key(c, first)
    elif operation == 'put':
        args.update(key=key(c, first), value=value(c, nonce + 1, 0))
    elif operation == 'batch_get':
        args['keys'] = [key(c, (first + i) % c['keys']) for i in range(c['batch_size'])]
    else:
        args['pairs'] = [[key(c, (first + i) % c['keys']), value(c, nonce + 1, i)] for i in range(c['batch_size'])]
    return operation, args


def build_check(run, retained, expected_revision):
    data = bounded(run / 'build.json', 65536); b = strict_json(data)
    keys(b, 'version revision dirty source_tree_sha256 binary_sha256 profile rustc'.split()); uint(b['version'], 1, 1)
    require(type(b['dirty']) is bool and b['profile'] in ('debug', 'release') and isinstance(b['rustc'], str) and 0 < len(b['rustc']) <= 4096, 'invalid build identity')
    for name, length in (('revision', 40), ('source_tree_sha256', 64), ('binary_sha256', 64)):
        require(isinstance(b[name], str) and re.fullmatch(f'[0-9a-f]{{{length}}}', b[name]), 'invalid build hash')
    require(data == bounded(retained / 'build.json', 65536), 'retained build manifest differs')
    binary = retained / 'kv9-batch-workload'
    require(0 < binary.stat().st_size <= 536870912, 'invalid executable bound')
    with binary.open('rb') as f:
        require(hashlib.file_digest(f, 'sha256').hexdigest() == b['binary_sha256'], 'retained executable differs')
    inventory = strict_json(bounded(retained / 'sources.json', 2 * 1024 * 1024))
    keys(inventory, 'version revision dirty sources command build_environment source_tree_sha256 binary_sha256'.split())
    uint(inventory['version'], 1, 1)
    require(type(inventory['dirty']) is bool, 'source inventory dirty is not boolean')
    require(all(same_json(inventory[k], b[k]) for k in ('revision', 'dirty', 'binary_sha256', 'source_tree_sha256')), 'source inventory build differs')
    require(isinstance(inventory['sources'], dict) and 0 < len(inventory['sources']) <= 10000, 'invalid source inventory mapping')
    for name, digest in inventory['sources'].items():
        require(isinstance(name, str) and name and not Path(name).is_absolute() and '..' not in Path(name).parts, 'invalid source inventory path')
        require((digest is None and b['dirty'] is True) or
                (isinstance(digest, str) and re.fullmatch('[0-9a-f]{64}', digest)), 'invalid source inventory digest')
    require(isinstance(inventory['build_environment'], dict) and
            all(isinstance(k, str) and isinstance(v, str) for k, v in inventory['build_environment'].items()), 'invalid source build environment')
    require(sha(json.dumps(inventory['sources'], sort_keys=True, separators=(',', ':')).encode()) == b['source_tree_sha256'], 'source inventory hash differs')
    command = inventory['command']
    require(isinstance(command, list) and all(isinstance(arg, str) for arg in command) and
            command.count('--bin') == 1 and command.index('--bin') + 1 < len(command) and
            command[command.index('--bin') + 1] == 'kv9-batch-workload', 'wrong workload build command')
    records = [strict_json(line) for line in bounded(retained / 'cargo.jsonl', 16 * 1024 * 1024).splitlines()]
    require(all(isinstance(row, dict) and isinstance(row.get('target', {}), dict) for row in records), 'invalid Cargo record shape')
    selected = {}
    for target in ('kv9-batch-workload', 'kv9_server', 'kv9_engine', 'kv9_raft'):
        rows = [r for r in records if r.get('reason') == 'compiler-artifact' and r.get('target', {}).get('name') == target]
        require(len(rows) == 1, 'missing or ambiguous native build artifact')
        features = rows[0].get('features')
        require(isinstance(features, list) and features in ([], ['rpc-experiment']) and (target not in ('kv9_engine', 'kv9_raft') or not features), 'unexpected runtime build features')
        selected[target] = rows[0]
    artifact = selected['kv9-batch-workload']
    require(isinstance(artifact.get('executable'), str) and artifact['executable'] and
            artifact['target'].get('kind') == ['bin'], 'native artifact is not executable')
    require(selected['kv9_server']['features'] == artifact['features'], 'native server/client feature graphs differ')
    if expected_revision:
        require(b['revision'] == expected_revision and b['dirty'] is False, 'native run is not the requested clean revision')
    return data, b, inventory, artifact


def validate(directory, build_directory, expected_revision=None, seconds=30):
    require(type(seconds) in (int, float) and math.isfinite(seconds) and seconds > 0, 'invalid native checker time budget')
    directory, build_directory = Path(directory), Path(build_directory)
    config_bytes = bounded(directory / 'config.json', 65536); c = strict_json(config_bytes); chunks = config_check(c)
    build_bytes, b, inventory, artifact = build_check(directory, build_directory, expected_revision)
    if c['rpc_transport'] == 'tarpc_tcp':
        command = inventory['command']
        require(command.count('--features') == 1 and command.index('--features') + 1 < len(command) and
                command[command.index('--features') + 1] == 'rpc-experiment' and
                'rpc-experiment' in artifact['features'], 'tarpc lacks explicit build feature evidence')
    r = strict_json(bounded(directory / 'report.json', 4 * 1024 * 1024))
    keys(r, 'version complete failure configuration config_sha256 build build_sha256 history_sha256 process_id wall_anchor_unix_ns wall_anchor_monotonic_ns elapsed_ns runtime_threads workload_model independently_checked stages stop history'.split())
    uint(r['version'], 1, 1)
    require(r['complete'] is True and r['failure'] is None and r['independently_checked'] is False, 'native workload is incomplete or falsely preverified')
    require(same_json(r['configuration'], c) and r['config_sha256'] == sha(config_bytes) and
            same_json(r['build'], b) and r['build_sha256'] == sha(build_bytes), 'native report config/build identity differs')
    require(r['workload_model'] == 'closed_loop_correctness', 'native workload is not the accepted correctness model')
    uint(r['process_id'], 1); uint(r['runtime_threads'], 2, 2); uint(r['wall_anchor_unix_ns'], 1)
    elapsed = uint(r['elapsed_ns'], 1); anchor = uint(r['wall_anchor_monotonic_ns'], 0, elapsed)
    stages = r['stages']; keys(stages, ['initialization', 'measurement_and_drain', 'verification']); previous = anchor
    for label in ('initialization', 'measurement_and_drain', 'verification'):
        span = stages[label]; keys(span, ['start_ns', 'end_ns']); previous = uint(span['end_ns'], uint(span['start_ns'], previous, elapsed), elapsed)
    keys(r['stop'], ['reason', 'monotonic_ns']); require(r['stop']['reason'] in ('stop_file', 'duration', 'operation_limit'), 'unsuccessful native stop reason')
    stop_time = uint(r['stop']['monotonic_ns'], 0, elapsed)
    require(stop_time == stages['measurement_and_drain']['end_ns'], 'native stop/drain boundary differs')
    # The producer records `end` before selecting the stop reason. Scheduling
    # between those reads can cross the duration deadline, so that reason alone
    # does not establish a stricter lower bound on the recorded end timestamp.
    h = r['history']; keys(h, 'issued terminal events bytes peak_in_flight accounting_complete failure metrics'.split())
    for field in ('issued', 'terminal', 'events', 'bytes', 'peak_in_flight'):
        uint(h[field])
    require(h['accounting_complete'] is True and h['failure'] is None and h['issued'] == h['terminal'] <= c['max_calls'] and h['events'] == h['issued'] * 2, 'native history accounting incomplete')
    data = bounded(directory / 'history.jsonl', c['history_bytes'])
    require(len(data) == h['bytes'] and sha(data) == r['history_sha256'], 'native history bytes/hash differ')
    require(data.endswith(b'\n'), 'complete native history lacks its final newline')
    records = [strict_json(line) for line in data.splitlines()]
    require(records and same_json(records[0], dict(type='header', version=2, range_chunk_size=1024, generator='kv9-native-batch-workload', configuration=c, initial={'keyspaces':[{'name':c['keyspace_name'], 'id':c['client']['keyspace_id']}], 'kv':[]})), 'native header differs')
    history = History.parse(records)
    issued, active, completed, nonces, workers = {}, {}, {}, set(), set()
    last_time, peak = 0, 0
    aggregate, samples = {}, {}
    for seq, event in enumerate(records[1:]):
        require(uint(event['seq']) == seq, 'missing or reordered native event')
        now = uint(event['monotonic_ns'], last_time, elapsed); last_time = now; identity = uint(event['id'], 0, c['max_calls'] - 1)
        if event['type'] == 'invoke':
            keys(event, 'type seq monotonic_ns id client phase nonce op args'.split())
            require(identity == len(issued) and event['client'] in {str(i) for i in range(c['workers'])} and event['client'] not in workers, 'native invocation/worker mismatch')
            phase, operation = event['phase'], event['op']; require(operation in KINDS, 'unexpected native operation')
            if phase in ('initialization', 'verify'):
                require(event['nonce'] is None and event['client'] == '0', 'setup/verification has traffic identity')
            else:
                require(phase in TRAFFIC_PHASES, 'invalid native traffic phase')
                nonce = uint(event['nonce'], 0, c['max_calls'] - 3 * chunks - 1)
                require(nonce not in nonces and (operation, event['args']) == generated(c, nonce), 'duplicate nonce or altered generated batch')
                nonces.add(nonce)
            span = stages[{'initialization':'initialization', 'verify':'verification'}.get(phase, 'measurement_and_drain')]
            require(span['start_ns'] <= now <= span['end_ns'], 'native invocation outside stage')
            issued[identity] = event; active[identity] = event; workers.add(event['client']); peak = max(peak, len(active)); require(peak <= c['workers'], 'native concurrency exceeds bound')
            continue
        keys(event, 'type seq monotonic_ns id outcome result observation'.split()); require(event['type'] == 'return' and identity in active, 'unmatched/duplicate native terminal')
        call = active.pop(identity); completed[identity] = event; workers.remove(call['client'])
        p, operation, args = call['phase'], call['op'], call['args']; read = operation in READS
        span = stages[{'initialization':'initialization', 'verify':'verification'}.get(p, 'measurement_and_drain')]; require(now <= span['end_ns'], 'native terminal outside drained stage')
        observation = event['observation']; keys(observation, 'attempts elapsed_ns stop reason receipt malformed'.split()); require(observation['malformed'] is None, 'malformed native reply')
        duration = uint(observation['elapsed_ns'], 0, now - call['monotonic_ns']); population = reason_population(observation['reason'])
        attempts = observation['attempts']; require(isinstance(attempts, list) and len(attempts) <= c['client']['max_attempts'], 'native attempt bound exceeded')
        local = not attempts
        require(not local or population in (9, 10, 11), 'unattempted call lacks local rejection')
        require(local or population not in (10, 11), 'attempted call claims local capacity/input rejection')
        require(read or population not in (5, 6), 'batch write claims a read-quorum outcome')
        outcome = 'ok' if population == 0 else 'refused' if local or population in (1, 2, 3, 4) else 'unknown'
        require(event['outcome'] == outcome, 'native terminal reason/outcome differ')
        require(observation['stop'] in ('terminal', 'deadline', 'attempt_limit', 'client_rejected'), 'invalid native stop')
        if local:
            require(observation['stop'] == ('deadline' if population == 9 else 'client_rejected'), 'local stop differs')
        elif observation['stop'] == 'attempt_limit':
            require(population == 1 and len(attempts) == c['client']['max_attempts'], 'attempt limit lacks refused final attempt')
        elif observation['stop'] == 'deadline':
            require(population == 1, 'retry deadline lacks prior refusal')
        else:
            require(observation['stop'] == 'terminal', 'attempted call claims local stop')
            require(population != 1, 'NotLeader terminal lacks retry stop classification')
        if observation['stop'] == 'deadline' or population == 9:
            require(duration >= c['client']['deadline_ms'] * 1000000, 'native deadline observation precedes its absolute deadline')
        if outcome == 'ok' and not read:
            receipt = observation['receipt']; keys(receipt, ['applied_term', 'applied_index']); uint(receipt['applied_term'], 1); uint(receipt['applied_index'], 1)
        else:
            require(observation['receipt'] is None, 'unexpected batch write receipt')
        count = len(args['keys']) if operation == 'batch_get' else len(args['pairs']) if operation == 'batch_put' else 1
        row = aggregate.setdefault((p, operation), dict(phase=p, operation=operation, calls=0, input_items=0, successful_items=0, unknown_write_items=0, refused_items=0, attempts=0, reasons=Counter()))
        row['calls'] += 1; row['input_items'] += count; row['attempts'] += len(attempts)
        if outcome == 'ok': row['successful_items'] += count
        elif outcome == 'refused': row['refused_items'] += count
        elif not read: row['unknown_write_items'] += count
        row['reasons']['success' if population == 0 else observation['reason']['kind']] += 1
        groups = samples.setdefault((p, operation), {level:[[] for _ in OUTCOMES] for level in ('logical', 'attempt')})
        group = 0 if outcome == 'ok' else 3 if local else 5 if outcome == 'refused' else 1 if read else 6
        groups['logical'][group].append(duration)
        total = 0
        for ordinal, attempt in enumerate(attempts, 1):
            keys(attempt, ['ordinal', 'node_id', 'elapsed_ns', 'failure'])
            require(uint(attempt['ordinal'], 1, c['client']['max_attempts']) == ordinal and
                    uint(attempt['node_id'], 1) in {peer['node_id'] for peer in c['client']['peers']}, 'native attempt identity mismatch')
            attempt_duration = uint(attempt['elapsed_ns'], 0, duration); total += attempt_duration; reason = reason_population(attempt['failure'])
            require(ordinal == len(attempts) or reason == 1, 'native retry lacks exclusive NotLeader refusal')
            if ordinal == len(attempts): require(same_json(attempt['failure'], observation['reason']), 'native final attempt differs from terminal')
            else:
                peers = [peer['node_id'] for peer in c['client']['peers']]
                hint = attempt['failure']['leader']
                following = hint if hint in peers and hint != attempt['node_id'] else peers[(peers.index(attempt['node_id']) + 1) % len(peers)]
                require(uint(attempts[ordinal]['node_id'], 1) == following, 'native redirect does not follow its fixed peer mapping')
            group = 0 if reason == 0 else 5 if reason in (1, 2, 3, 4) else 1 if read else 6
            groups['attempt'][group].append(attempt_duration)
        require(total <= duration, 'native attempt durations exceed whole batch latency')
    require(not active and len(issued) == len(completed) == h['issued'] and len(records) - 1 == h['events'] and peak == h['peak_in_flight'], 'native complete-history ledger differs')
    require(nonces and nonces == set(range(len(nonces))), 'native traffic is empty or omits a generated invocation')
    if r['stop']['reason'] == 'operation_limit': require(len(nonces) == c['max_calls'] - 3 * chunks, 'native operation-limit cohort is incomplete')
    setup = [call for call in issued.values() if call['phase'] == 'initialization']; final = [call for call in issued.values() if call['phase'] == 'verify']
    groups = [list(range(first, min(first + c['batch_size'], c['keys'] + 1))) for first in range(0, c['keys'] + 1, c['batch_size'])]
    expected = [('batch_get', {'keyspace':c['client']['keyspace_id'], 'keys':[key(c, i) for i in group]}) for group in groups]
    expected += [('batch_put', {'keyspace':c['client']['keyspace_id'], 'pairs':[[key(c, i), value(c, 0, i)] for i in group]}) for group in groups]
    require([(call['op'], call['args']) for call in setup] == expected, 'native initialization chunks differ or are omitted')
    require([(call['op'], call['args']) for call in final] == expected[:chunks], 'native verification chunks differ or are omitted')
    for call in setup + final:
        result = completed[call['id']]; require(result['outcome'] == 'ok', 'native setup/verification did not succeed')
        if call['phase'] == 'initialization' and call['op'] == 'batch_get': require(all(v is None for v in result['result']['values']), 'native initial dataset was not empty')
    for call in final:
        if key(c, c['keys']) in call['args']['keys']:
            at = call['args']['keys'].index(key(c, c['keys'])); require(completed[call['id']]['result']['values'][at] == '', 'native empty sentinel changed')
    require(isinstance(h['metrics'], list) and len(h['metrics']) == len(aggregate), 'native metric rows differ')
    seen = set()
    for row in h['metrics']:
        keys(row, 'phase operation calls input_items successful_items unknown_write_items refused_items attempts reasons logical_latency attempt_latency'.split())
        identity = (row['phase'], row['operation']); require(identity in aggregate and identity not in seen, 'unknown/duplicate native metric row'); seen.add(identity)
        for name in ('calls', 'input_items', 'successful_items', 'unknown_write_items', 'refused_items', 'attempts'): uint(row[name])
        require(isinstance(row['reasons'], dict), 'native metric reasons are not a count mapping')
        for count in row['reasons'].values():
            uint(count, 1, row['calls'])
        require(same_json({k:v for k,v in row.items() if k not in ('logical_latency', 'attempt_latency')}, aggregate[identity]), 'native RPC/item/reason counts differ from full history')
        for level in ('logical', 'attempt'):
            metrics = row[level + '_latency']; keys(metrics, ['valid', 'outcomes'])
            require(metrics['valid'] is True, 'invalid native histogram observer')
            require(isinstance(metrics['outcomes'], list) and len(metrics['outcomes']) == len(OUTCOMES), 'native histogram population differs')
            for index, bucket in enumerate(metrics['outcomes']):
                native_histogram_check(bucket); require(bucket['outcome'] == OUTCOMES[index], 'native histogram outcome reordered')
                require(all(same_json(bucket[k], v) for k,v in histogram(samples[identity][level][index]).items()), 'native histogram is not one sample per whole call/attempt')
    verdict = check(history, max_states=400000, seconds=seconds)
    require(verdict['verdict'] == 'valid' and verify_witness(history, verdict['witness']), 'native atomic history invalid or inconclusive')
    traffic = [op for op in history.operations if issued[op.id]['phase'] in TRAFFIC_PHASES]
    successes = Counter(op.kind for op in traffic if op.outcome == 'ok')
    require(any(successes[k] for k in READS) and any(successes[k] for k in ('put', 'delete', 'batch_put')), 'native history lacks successful traffic reads and writes')
    for kind, weight in zip(KINDS, c['mix']):
        if weight: require(successes[kind] > 0, 'native configured operation has no successful traffic coverage')
    return dict(accepted=True, version=1, revision=b['revision'], dirty=b['dirty'], report_sha256=sha(bounded(directory/'report.json', 4*1024*1024)), full_history_independently_checked=True, history={**verdict, 'coverage':coverage(history)}, traffic_successes=dict(successes))


def main():
    parser = argparse.ArgumentParser(description=__doc__); parser.add_argument('--run', type=Path, required=True); parser.add_argument('--build', type=Path, required=True); parser.add_argument('--revision'); parser.add_argument('--seconds', type=float, default=30); parser.add_argument('--output', type=Path, required=True); args = parser.parse_args()
    try:
        result = validate(args.run, args.build, args.revision, args.seconds)
    except (ValueError, KeyError, TypeError, IndexError, OSError) as error:
        result = {'accepted':False, 'failure':str(error)}
    args.output.write_text(json.dumps(result, sort_keys=True, indent=2) + '\n')
    print(json.dumps({k:v for k,v in result.items() if k != 'history'}, sort_keys=True)); return 0 if result['accepted'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
