#!/usr/bin/env python3
"""Independently validate bounded native batch measurement artifacts.

This checks aggregate accounting, histogram arithmetic and retained build inputs.
It cannot reconstruct linearizability, per-call retry histories, server durability
or environmental isolation. A separate fixture must establish those properties.
"""
import argparse
import hashlib
import ipaddress
import json
import math
from pathlib import Path
import re

from workload_report import bounded, keys, require, sha, strict_json, uint

U64 = (1 << 64) - 1
PHASES = ['initialization', 'warmup', 'measurement', 'verification']
OPERATIONS = ['batch_get', 'batch_put']
OUTCOMES = ['success', 'refused', 'unknown_write', 'read_failure', 'client_rejected']
REASONS = ['success', 'not_leader', 'admission_count', 'admission_bytes',
           'admission_oversize', 'read_quorum_unconfirmed', 'read_apply_unconfirmed',
           'rpc_status', 'protocol', 'deadline', 'client_capacity', 'client_input']
REPORT_FIELDS = '''version workload_model full_history_recorded independently_checked
complete failure configuration config_sha256 build build_sha256 wire_sizes process_id
runtime_threads measurement_start_unix_ns stages stop_reason measured_issued
measured_completed offered_slots dropped_slots task_failures cohort_elapsed_ns
completed_batches_per_second successful_batches_per_second
successful_input_items_per_second timing_eligible proc_stat_before proc_stat_after
workers metrics'''.split()


def same(a, b):
    if type(a) is not type(b):
        return False
    if isinstance(a, dict):
        return a.keys() == b.keys() and all(same(a[k], b[k]) for k in a)
    if isinstance(a, list):
        return len(a) == len(b) and all(same(x, y) for x, y in zip(a, b))
    return a == b


def equal(actual, expected, message):
    require(same(actual, expected), message)


def vector(value, length):
    require(type(value) is list and len(value) == length, 'invalid counter vector length')
    for item in value:
        uint(item)
    return value


def number(actual, expected, message):
    require(type(actual) in (float, int) and math.isfinite(actual) and
            math.isclose(actual, expected, rel_tol=1e-14, abs_tol=0), message)


def varint_size(n):
    return max(1, (n.bit_length() + 6) // 7)


def blob_size(length):
    return 1 + varint_size(length) + length


def config_check(c):
    keys(c, '''version client rpc_transport run_id seed workers keys batch_size
value_bytes read_percent warmup_calls measure_ms max_calls load'''.split())
    uint(c['version'], 1, 1)
    require(c['rpc_transport'] in ('tonic_stream', 'tonic_unary', 'tarpc_tcp'), 'invalid transport')
    require(type(c['run_id']) is str and re.fullmatch(r'[A-Za-z0-9_-]{1,64}', c['run_id']), 'invalid run ID')
    client = c['client']
    keys(client, '''version peers keyspace_id epoch_conf_ver epoch_version
max_in_flight max_attempts deadline_ms retry_backoff_ms'''.split())
    for name in ('version', 'epoch_conf_ver', 'epoch_version'):
        uint(client[name], 1, 1)
    uint(client['keyspace_id'], 1, (1 << 24) - 1)
    uint(client['max_in_flight'], 1, 256)
    uint(client['max_attempts'], 1, 16)
    uint(client['deadline_ms'], 1, 30_000)
    uint(client['retry_backoff_ms'], 0, client['deadline_ms'])
    peers = client['peers']
    require(type(peers) is list and 1 <= len(peers) <= 32, 'invalid peer count')
    ids, addresses = set(), set()
    for peer in peers:
        keys(peer, ['node_id', 'address'])
        uint(peer['node_id'], 1)
        require(type(peer['address']) is str, 'invalid peer address')
        match = re.fullmatch(r'(\[[0-9a-fA-F:.]+\]|[0-9.]+):([0-9]+)', peer['address'])
        require(match is not None, 'peer must be an explicit IP and port')
        address = (ipaddress.ip_address(match[1].strip('[]')), int(match[2]))
        uint(address[1], 1, 65535)
        require(peer['node_id'] not in ids and address not in addresses, 'duplicate peer')
        ids.add(peer['node_id'])
        addresses.add(address)
    uint(c['seed'])
    uint(c['workers'], 1, client['max_in_flight'])
    uint(c['keys'], 1, 65_536)
    uint(c['batch_size'], 1, min(256, c['keys']))
    uint(c['value_bytes'], 16, 8192)
    uint(c['read_percent'], 0, 100)
    uint(c['warmup_calls'], 0, 100_000)
    uint(c['measure_ms'], 1, 60_000)
    uint(c['max_calls'], 1, 10_000_000)
    require(type(c['load']) is dict, 'invalid load')
    if c['load'].get('kind') == 'fixed_rate':
        keys(c['load'], ['kind', 'batches_per_second'])
        rate = uint(c['load']['batches_per_second'], 1, 2_000_000)
        offered = (c['measure_ms'] * rate + 999) // 1000
        require(offered <= c['max_calls'], 'offered load exceeds call budget')
    else:
        equal(c['load'], {'kind': 'closed_loop'}, 'invalid load kind')
        offered = None
    # Independent protobuf size calculation: all keys/values are nonempty, and
    # keyspace plus both epoch fields are nonzero. Field numbers fit one byte.
    context = blob_size(1 + varint_size(client['keyspace_id']) + blob_size(4))
    key_bytes = len(c['run_id']) + 17
    key_field, value_field = blob_size(key_bytes), blob_size(c['value_bytes'])
    sizes = {
        'get_request_bytes': context + c['batch_size'] * key_field,
        'put_request_bytes': context + c['batch_size'] * blob_size(key_field + value_field),
        'get_response_bytes': c['batch_size'] * blob_size(2 + value_field),
        'maximum_input_items_in_flight': c['workers'] * c['batch_size'],
        'maximum_input_payload_bytes_in_flight': c['workers'] * c['batch_size'] * (key_bytes + c['value_bytes']),
    }
    require(all(sizes[k] <= 1_048_576 for k in ('get_request_bytes', 'put_request_bytes', 'get_response_bytes')), 'encoded batch exceeds limit')
    require(sizes['maximum_input_payload_bytes_in_flight'] <= 64 * 1024 * 1024, 'pending input exceeds limit')
    return offered, sizes


def bucket_bounds(index):
    if index < 64:
        return index, index
    exponent, part = divmod(index - 64, 64)
    lower = (64 + part) * 2 ** exponent
    return lower, lower + 2 ** exponent - 1


def histogram_check(h):
    keys(h, ['raw', 'mean_ns', 'p50', 'p95', 'p99'])
    r = h['raw']
    keys(r, ['count', 'sum_ns', 'min_ns', 'max_ns', 'valid', 'buckets'])
    require(r['valid'] is True, 'invalid or overflowed histogram')
    count, total = uint(r['count']), uint(r['sum_ns'])
    require(type(r['buckets']) is list and len(r['buckets']) in (0, 3776), 'invalid histogram bucket count')
    vector(r['buckets'], len(r['buckets']))
    require(sum(r['buckets']) == count, 'histogram population differs')
    if not count:
        require(total == 0 and r['min_ns'] is None and r['max_ns'] is None, 'nonempty zero-count histogram')
        require(all(h[k] is None for k in ('mean_ns', 'p50', 'p95', 'p99')), 'zero-count latency summary')
        return r
    minimum, maximum = uint(r['min_ns']), uint(r['max_ns'])
    require(minimum <= maximum, 'latency extrema reversed')
    used = [(i, n) for i, n in enumerate(r['buckets']) if n]
    low, high = bucket_bounds(used[0][0])
    require(low <= minimum <= high, 'minimum outside first occupied bucket')
    low, high = bucket_bounds(used[-1][0])
    require(low <= maximum <= high, 'maximum outside last occupied bucket')
    # Tighten the possible sum using every occupied bucket and the exact extrema.
    lower_sum = upper_sum = 0
    for i, n in used:
        low, high = bucket_bounds(i)
        low, high = max(low, minimum), min(high, maximum)
        require(low <= high, 'occupied bucket outside extrema')
        lower_sum += low * n
        upper_sum += high * n
    if count == 1:
        require(minimum == maximum == total, 'single-sample histogram differs')
    else:
        # Both extrema must occur at least once, even within the same bucket.
        lower_sum += maximum - max(bucket_bounds(used[-1][0])[0], minimum)
        upper_sum -= min(bucket_bounds(used[0][0])[1], maximum) - minimum
    require(lower_sum <= total <= upper_sum, 'sum impossible for retained histogram')
    number(h['mean_ns'], total / count, 'histogram mean differs')
    for percentile in (50, 95, 99):
        rank, cumulative = (count * percentile + 99) // 100, 0
        for i, n in used:
            cumulative += n
            if cumulative >= rank:
                low, high = bucket_bounds(i)
                equal(h['p' + str(percentile)], {'lower_ns': low, 'upper_ns': high}, 'histogram quantile differs')
                break
    return r


def dominance_check(containing, contained):
    """Necessary bucket-order condition for paired samples where A >= B.

    At each bucket's upper boundary, A cannot have more completed samples below
    that boundary than B. Bucket intervals prevent exact per-call reconstruction.
    """
    require(containing['count'] == contained['count'], 'paired latency counts differ')
    if not containing['count']:
        return
    require(containing['min_ns'] >= contained['min_ns'] and containing['max_ns'] >= contained['max_ns'],
            'paired latency extrema violate containment')
    left = right = 0
    for a, b in zip(containing['buckets'], contained['buckets']):
        left += a
        right += b
        require(left <= right, 'paired latency distributions violate containment')


def metrics_check(m, c, phase):
    keys(m, ['operations', 'outcomes', 'reasons', 'attempt_outcomes', 'histogram_subdivisions', 'valid', 'statistics'])
    for name, expected in [('operations', OPERATIONS), ('outcomes', OUTCOMES), ('reasons', REASONS),
                           ('attempt_outcomes', ['success', 'refused', 'failed_or_unknown'])]:
        equal(m[name], expected, 'metric vocabulary differs: ' + name)
    uint(m['histogram_subdivisions'], 64, 64)
    require(m['valid'] is True and type(m['statistics']) is list and len(m['statistics']) == 2, 'invalid metric statistics')
    fixed = phase == 'measurement' and c['load']['kind'] == 'fixed_rate'
    counts, successes, items, attempts = [], 0, 0, 0
    for kind, op in enumerate(m['statistics']):
        keys(op, ['populations', 'reasons', 'rpc_codes', 'attempt_reasons', 'attempt_rpc_codes',
                  'attempts', 'dispatch_lateness', 'data_failures'])
        uint(op['data_failures'], 0, 0)
        require(type(op['populations']) is list and len(op['populations']) == 5, 'outcome populations missing')
        calls = sdk_sum = whole_sum = scheduled_sum = sdk_max = 0
        scheduled_buckets = [0] * 3776 if fixed else []
        scheduled_min = scheduled_max = None
        for outcome, pop in enumerate(op['populations']):
            keys(pop, ['calls', 'input_items', 'completed_before_cutoff', 'whole_call', 'sdk_call', 'scheduled_to_completion'])
            n = uint(pop['calls'], 0, 10_500_000)
            uint(pop['input_items'], n, n * c['batch_size'])
            uint(pop['completed_before_cutoff'], 0, n)
            if phase in ('measurement', 'warmup'):
                equal(pop['input_items'], n * c['batch_size'], 'whole-batch item accounting differs')
            if phase != 'measurement':
                equal(pop['completed_before_cutoff'], n, 'setup/warmup/verification population truncated')
                require(outcome == 0 or n == 0, 'incomplete setup/warmup/verification')
            require(not (kind == 0 and outcome == 2 and n), 'unknown write in read population')
            require(not (kind == 1 and outcome == 3 and n), 'read failure in write population')
            whole, sdk, scheduled = [histogram_check(pop[name]) for name in ('whole_call', 'sdk_call', 'scheduled_to_completion')]
            require(whole['count'] == sdk['count'] == n and scheduled['count'] == (n if fixed else 0), 'logical latency population differs')
            require(whole['sum_ns'] >= sdk['sum_ns'], 'SDK time exceeds containing call time')
            dominance_check(whole, sdk)
            if n:
                sdk_max = max(sdk_max, sdk['max_ns'])
            if fixed:
                dominance_check(scheduled, whole)
                if n:
                    scheduled_min = scheduled['min_ns'] if scheduled_min is None else min(scheduled_min, scheduled['min_ns'])
                    scheduled_max = scheduled['max_ns'] if scheduled_max is None else max(scheduled_max, scheduled['max_ns'])
                for i, count in enumerate(scheduled['buckets']):
                    scheduled_buckets[i] += count
            calls += n
            sdk_sum += sdk['sum_ns']
            whole_sum += whole['sum_ns']
            scheduled_sum += scheduled['sum_ns']
        p = op['populations']
        reasons, codes = vector(op['reasons'], 12), vector(op['rpc_codes'], 17)
        ar, ac = vector(op['attempt_reasons'], 12), vector(op['attempt_rpc_codes'], 17)
        require(sum(reasons) == calls and sum(codes) == reasons[7], 'terminal reason/RPC accounting differs')
        require(reasons[0] == p[0]['calls'] and sum(reasons[1:5]) == p[1]['calls'], 'terminal success/refusal accounting differs')
        require(reasons[8] == 0, 'protocol failure in accepted report')
        rejected = p[4]['calls']
        rejected_deadline = rejected - reasons[10] - reasons[11]
        require(0 <= rejected_deadline <= reasons[9], 'client rejection reason population differs')
        require(sum(ac) == ar[7] and ac == codes, 'attempt/terminal RPC status population differs')
        require(ar[0] == reasons[0] and ar[1] >= reasons[1] and ar[2:9] == reasons[2:9] and
                ar[9] == reasons[9] - rejected_deadline and ar[10:] == [0, 0], 'attempt/terminal reason population differs')
        require(type(op['attempts']) is list and len(op['attempts']) == 3, 'attempt outcomes missing')
        ah = [histogram_check(h) for h in op['attempts']]
        require([h['count'] for h in ah] == [ar[0], sum(ar[1:5]), sum(ar[5:])], 'attempt histogram populations differ')
        attempt_count = sum(ar)
        require(attempt_count == calls - rejected + ar[1] - reasons[1], 'non-NotLeader retry or omitted attempt')
        require(calls - rejected <= attempt_count <= (calls - rejected) * c['client']['max_attempts'], 'attempt budget differs')
        require(sum(h['sum_ns'] for h in ah) <= sdk_sum, 'attempt durations exceed SDK time')
        require(all(not h['count'] or h['max_ns'] <= sdk_max for h in ah), 'attempt sample exceeds every SDK call')
        lateness = histogram_check(op['dispatch_lateness'])
        require(lateness['count'] == (calls if fixed else 0), 'dispatch lateness population differs')
        require(scheduled_sum == (whole_sum + lateness['sum_ns'] if fixed else 0), 'scheduled latency omits dispatch wait or call time')
        if fixed:
            dominance_check({'count': calls, 'buckets': scheduled_buckets,
                             'min_ns': scheduled_min, 'max_ns': scheduled_max}, lateness)
        counts.append(calls)
        successes += p[0]['calls']
        items += p[0]['input_items']
        attempts += attempt_count
    return {'calls': counts, 'successes': successes, 'successful_items': items, 'attempts': attempts}


def mix(value):
    value = (value + 0x9e3779b97f4a7c15) & U64
    value = ((value ^ (value >> 30)) * 0xbf58476d1ce4e5b9) & U64
    value = ((value ^ (value >> 27)) * 0x94d049bb133111eb) & U64
    return value ^ (value >> 31)


def proc_check(before, after, pid):
    def parse(text):
        require(type(text) is str and 0 < len(text) <= 8192, 'missing process stat')
        match = re.fullmatch(r'(\d+) \((.*)\) (.*)\s*', text, re.DOTALL)
        require(match is not None and int(match[1]) == pid, 'process stat PID differs')
        fields = match[3].split()
        require(len(fields) >= 22, 'process stat fields missing')
        return {'name': match[2], 'start_ticks': uint(int(fields[19]), 1),
                'user_ticks': uint(int(fields[11])), 'system_ticks': uint(int(fields[12]))}
    a, b = parse(before), parse(after)
    require(a['name'] == b['name'] and a['start_ticks'] == b['start_ticks'], 'process lifetime differs')
    require(a['user_ticks'] <= b['user_ticks'] and a['system_ticks'] <= b['system_ticks'], 'process CPU counters regressed')
    return a['start_ticks']


def report_check(r, c, b):
    keys(r, REPORT_FIELDS)
    uint(r['version'], 1, 1)
    require(r['workload_model'] == 'bounded_native_batch_performance', 'wrong workload model')
    require(r['complete'] is True and r['failure'] is None, 'measurement incomplete')
    require(r['full_history_recorded'] is False and r['independently_checked'] is False, 'client claims independent history verification')
    equal(r['configuration'], c, 'reported configuration differs')
    equal(r['build'], b, 'reported build differs')
    offered, sizes = config_check(c)
    equal(r['wire_sizes'], sizes, 'wire size or declared in-flight bound differs')
    uint(r['process_id'], 1, (1 << 32) - 1)
    uint(r['runtime_threads'], 2, 2)
    uint(r['measurement_start_unix_ns'], 1)
    uint(r['task_failures'], 0, 0)
    require(r['stop_reason'] in ('duration', 'operation_limit'), 'invalid measurement stop')
    equal(r['timing_eligible'], r['stop_reason'] == 'duration' and b['profile'] == 'release' and not b['dirty'], 'timing eligibility differs')
    keys(r['metrics'], PHASES)
    metrics = {name: metrics_check(r['metrics'][name], c, name) for name in PHASES}
    return cohort_check(r, c, metrics, offered)


def cohort_check(r, c, metrics, offered, call_latency='sdk_call'):
    """Shared schedule, stage and population arithmetic; no transport claims."""
    chunks = (c['keys'] + c['batch_size']) // c['batch_size']
    equal(metrics['initialization']['calls'], [chunks, chunks], 'initialization calls missing')
    equal(metrics['verification']['calls'], [chunks, 0], 'verification calls missing')
    for phase, kinds in [('initialization', (0, 1)), ('verification', (0,))]:
        for kind in kinds:
            pop = r['metrics'][phase]['statistics'][kind]['populations'][0]
            equal(pop['input_items'], c['keys'] + 1, 'dataset or sentinel item missing')
    warm_reads = sum(mix(c['seed'] ^ n) % 100 < c['read_percent'] for n in range(1, c['warmup_calls'] + 1))
    equal(metrics['warmup']['calls'], [warm_reads, c['warmup_calls'] - warm_reads], 'warmup operation mix differs')
    measured = metrics['measurement']
    total = sum(measured['calls'])
    uint(r['measured_issued'], 1, c['max_calls'])
    require(r['measured_issued'] == uint(r['measured_completed']) == total, 'issued/terminal accounting differs')
    if c['read_percent'] in (0, 100):
        require(measured['calls'][0 if c['read_percent'] == 0 else 1] == 0, 'measurement operation mix differs')
    equal(r['offered_slots'], offered, 'offered slot count differs')
    dropped = uint(r['dropped_slots'], 0, c['max_calls'])
    if offered is None:
        require(dropped == 0, 'closed loop reports dropped slots')
    else:
        require(r['stop_reason'] == 'duration' and total + dropped == offered, 'offered work omitted or duplicated')
    workers = r['workers']
    require(type(workers) is list and len(workers) == c['workers'], 'worker population differs')
    last = stopped = worker_issued = worker_dropped = 0
    for identity, worker in enumerate(workers):
        keys(worker, ['worker', 'issued', 'dropped_slots', 'last_terminal_ns', 'stopped_ns', 'stopped_for_failure'])
        uint(worker['worker'], identity, identity)
        n = uint(worker['issued'], 0, total)
        drop = uint(worker['dropped_slots'], 0, dropped)
        end = uint(worker['last_terminal_ns'])
        stop = uint(worker['stopped_ns'], end)
        require(worker['stopped_for_failure'] is False and ((n == 0) == (end == 0)), 'worker termination differs')
        if offered is not None:
            owned = len(range(identity, offered, c['workers']))
            require(n + drop == owned, 'worker strided slot population differs')
        last, stopped = max(last, end), max(stopped, stop)
        worker_issued += n
        worker_dropped += drop
    require(worker_issued == total and worker_dropped == dropped, 'worker/global accounting differs')
    nominal = c['measure_ms'] * 1_000_000
    after_cutoff = sum(w['issued'] > 0 and w['last_terminal_ns'] >= nominal for w in workers)
    before_cutoff = sum(p['completed_before_cutoff'] for op in r['metrics']['measurement']['statistics'] for p in op['populations'])
    # One sequential call per worker: if any call completes after the dispatch
    # cutoff it is that worker's final call, because no new call may then start.
    require(before_cutoff == total - after_cutoff, 'cutoff completion count differs from worker tails')
    keys(r['stages'], ['initialization', 'warmup', 'measurement', 'drain', 'verification'])
    previous = 0
    for stage in ('initialization', 'warmup', 'measurement', 'drain', 'verification'):
        item = r['stages'][stage]
        keys(item, ['start_ns', 'end_ns'])
        uint(item['start_ns'], previous)
        previous = uint(item['end_ns'], item['start_ns'])
    require(r['stages']['initialization']['start_ns'] == 0, 'initialization origin differs')
    start = r['stages']['measurement']['start_ns']
    cutoff = r['stages']['measurement']['end_ns'] - start
    if r['stop_reason'] == 'duration':
        require(cutoff == nominal, 'measurement duration differs')
    else:
        require(offered is None and cutoff == min(stopped, nominal) and total >= c['max_calls'] - c['workers'], 'operation-limit stop differs')
    elapsed = uint(r['cohort_elapsed_ns'], 1)
    require(elapsed == max(cutoff, last), 'terminal drain excluded or extra time included')
    require(r['stages']['drain']['start_ns'] == start + cutoff and r['stages']['drain']['end_ns'] == start + elapsed, 'drain stage differs')
    require(stopped <= r['stages']['verification']['start_ns'] - start, 'worker stopped after verification began')
    for phase in PHASES:
        span = elapsed if phase == 'measurement' else r['stages'][phase]['end_ns'] - r['stages'][phase]['start_ns']
        whole_sum = 0
        for op in r['metrics'][phase]['statistics']:
            for pop in op['populations']:
                whole_sum += pop['whole_call']['raw']['sum_ns']
                for name in ('whole_call', call_latency, 'scheduled_to_completion'):
                    h = pop[name]['raw']
                    require(not h['count'] or h['max_ns'] <= (last if phase == 'measurement' else span), 'latency sample exceeds containing stage')
        # Whole calls are sequential within a worker. Scheduled-to-completion
        # intervals may overlap, so their sums do not have this same bound.
        maximum_sum = sum(w['last_terminal_ns'] for w in workers) if phase == 'measurement' else span
        require(whole_sum <= maximum_sum, 'whole-call durations exceed available worker time')
    for name, numerator in [('completed_batches_per_second', total),
                            ('successful_batches_per_second', measured['successes']),
                            ('successful_input_items_per_second', measured['successful_items'])]:
        number(r[name], numerator / (elapsed / 1e9), 'throughput denominator/population differs')
    ticks = proc_check(r['proc_stat_before'], r['proc_stat_after'], r['process_id'])
    return {'calls': total, 'successful_calls': measured['successes'],
            'successful_input_items': measured['successful_items'], 'attempts': measured['attempts'],
            'offered_slots': offered, 'dropped_slots': dropped, 'process_start_ticks': ticks}


def build_check(directory, retained, expected_revision=None, reference=False):
    raw = bounded(directory / 'build.json', 65_536)
    b = strict_json(raw)
    keys(b, 'version revision dirty source_tree_sha256 binary_sha256 profile rustc'.split())
    uint(b['version'], 1, 1)
    require(type(b['dirty']) is bool and b['profile'] in ('debug', 'release'), 'invalid build profile')
    require(not (b['dirty'] and b['profile'] == 'release'), 'dirty release artifact')
    require(type(b['rustc']) is str and 0 < len(b['rustc']) <= 4096, 'invalid rustc identity')
    for name, length in [('revision', 40), ('source_tree_sha256', 64), ('binary_sha256', 64)]:
        require(type(b[name]) is str and re.fullmatch(f'[0-9a-f]{{{length}}}', b[name]), 'invalid build hash')
    require(raw == bounded(retained / 'build.json', 65_536), 'retained build manifest differs')
    binary_name = 'kv9-redis-batch-reference' if reference else 'kv9-batch-benchmark'
    binary = retained / binary_name
    require(0 < binary.stat().st_size <= 512 * 1024 * 1024, 'invalid executable size')
    with binary.open('rb') as stream:
        require(hashlib.file_digest(stream, 'sha256').hexdigest() == b['binary_sha256'], 'retained executable differs')
    inventory = strict_json(bounded(retained / 'sources.json', 2 * 1024 * 1024))
    keys(inventory, 'version revision dirty sources command build_environment source_tree_sha256 binary_sha256'.split())
    uint(inventory['version'], 1, 1)
    for name in ('revision', 'dirty', 'source_tree_sha256', 'binary_sha256'):
        equal(inventory[name], b[name], 'source inventory identity differs')
    sources = inventory['sources']
    require(type(sources) is dict and 1 <= len(sources) <= 10_000, 'invalid source inventory')
    for path, digest in sources.items():
        require(type(path) is str and path and not Path(path).is_absolute() and '..' not in Path(path).parts, 'invalid source path')
        require((digest is None and b['dirty']) or (type(digest) is str and re.fullmatch('[0-9a-f]{64}', digest)), 'invalid source digest')
    require(sha(json.dumps(sources, sort_keys=True, separators=(',', ':')).encode()) == b['source_tree_sha256'], 'source inventory hash differs')
    env = inventory['build_environment']
    require(type(env) is dict and all(type(k) is str and type(v) is str for k, v in env.items()), 'invalid build environment')
    records = [strict_json(line) for line in bounded(retained / 'cargo.jsonl', 16 * 1024 * 1024).splitlines()]
    require(all(type(row) is dict and type(row.get('target', {})) is dict for row in records), 'invalid Cargo output')
    require(sum(row.get('reason') == 'build-finished' and row.get('success') is True for row in records) == 1 and
            not any(row.get('reason') == 'build-finished' and row.get('success') is not True for row in records), 'Cargo did not complete successfully')
    selected = {}
    for name in ([binary_name] if reference else [binary_name, 'kv9_server', 'kv9_engine', 'kv9_raft']):
        rows = [row for row in records if row.get('reason') == 'compiler-artifact' and row.get('target', {}).get('name') == name]
        require(len(rows) == 1, 'missing or ambiguous Cargo artifact')
        row = rows[0]
        require(row.get('features') in ([[]] if reference else [[], ['rpc-experiment']]), 'unexpected Cargo features')
        if name in ('kv9_engine', 'kv9_raft'):
            equal(row['features'], [], 'engine or Raft feature contamination')
        profile = row.get('profile')
        require(type(profile) is dict and profile.get('test') is False, 'test executable in workload build')
        require(profile.get('opt_level') == ('3' if b['profile'] == 'release' else '0'), 'Cargo optimization profile differs')
        selected[name] = row
    artifact = selected[binary_name]
    if not reference:
        equal(artifact['features'], selected['kv9_server']['features'], 'client/server feature graphs differ')
    else:
        for source in ('scripts/redis-reference/src/bin/kv9-redis-batch-reference.rs',
                       'crates/server/src/bin/kv9-batch-benchmark/common.rs'):
            require(type(sources.get(source)) is str, 'Redis reference source missing')
    require(type(artifact.get('executable')) is str and artifact['executable'] and artifact['target'].get('kind') == ['bin'], 'workload artifact is not executable')
    selection = ['--manifest-path', 'scripts/redis-reference/Cargo.toml'] if reference else ['-p', 'kv9-server']
    expected = ['cargo', 'build', '--locked', *selection, '--bin', binary_name, '--message-format=json-render-diagnostics']
    if b['profile'] == 'release':
        expected.append('--release')
    if artifact['features']:
        expected.extend(['--features', 'rpc-experiment'])
    equal(inventory['command'], expected, 'workload build command differs')
    if expected_revision is not None:
        require(b['revision'] == expected_revision and b['dirty'] is False, 'not the requested clean revision')
    return raw, b, artifact['features']


def validate(directory, build_directory, expected_config=None, expected_revision=None, require_timing=False):
    directory, retained = Path(directory), Path(build_directory)
    raw = bounded(directory / 'report.json', 16 * 1024 * 1024)
    r = strict_json(raw)
    config_raw = bounded(directory / 'config.json', 65_536)
    c = strict_json(config_raw)
    require(type(c) is dict, 'invalid input configuration')
    c.setdefault('rpc_transport', 'tonic_stream')
    if expected_config is not None:
        expected = strict_json(bounded(expected_config, 65_536))
        require(type(expected) is dict, 'invalid requested configuration')
        expected.setdefault('rpc_transport', 'tonic_stream')
        equal(c, expected, 'run differs from requested configuration')
    build_raw, b, features = build_check(directory, retained, expected_revision)
    require(c['rpc_transport'] != 'tarpc_tcp' or features == ['rpc-experiment'], 'tarpc selected without feature')
    result = report_check(r, c, b)
    require(r['config_sha256'] == sha(config_raw) and r['build_sha256'] == sha(build_raw), 'input artifact digest differs')
    if require_timing:
        require(r['timing_eligible'] is True and expected_revision is not None and expected_config is not None, 'timing requires a clean release, exact revision and requested configuration')
    result.update(accepted=True, version=1, report_sha256=sha(raw), revision=b['revision'],
                  dirty=b['dirty'], profile=b['profile'], cargo_features=features,
                  client_timing_eligible=r['timing_eligible'],
                  full_history_independently_checked=False,
                  environment_independently_checked=False,
                  scope='Aggregate client arithmetic and retained build inputs; no server, history, isolation or performance-comparison acceptance')
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--directory', type=Path, required=True)
    parser.add_argument('--build-directory', type=Path, required=True)
    parser.add_argument('--expected-config', type=Path)
    parser.add_argument('--expected-revision')
    parser.add_argument('--require-timing', action='store_true')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    require(not args.output.exists(), 'validation output must be new')
    result = validate(args.directory, args.build_directory, args.expected_config, args.expected_revision, args.require_timing)
    with args.output.open('x') as stream:
        json.dump(result, stream, indent=2, sort_keys=True)
        stream.write('\n')
    print('PASS: independently validated batch client accounting; enclosing acceptance remains separate')


if __name__ == '__main__':
    main()
