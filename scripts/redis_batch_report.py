#!/usr/bin/env python3
"""Validate native Redis batch client accounting; no Raft or performance acceptance."""
import argparse
import ipaddress
import json
from pathlib import Path
import re
from batch_benchmark_report import (REPORT_FIELDS, PHASES, build_check, cohort_check,
    dominance_check, equal, histogram_check, vector)
from workload_report import bounded, keys, require, sha, strict_json, uint

WORKLOAD_FIELDS = 'run_id seed workers keys batch_size value_bytes read_percent warmup_calls measure_ms max_calls load'.split()


def config_check(c):
    require(type(c) is dict, 'Redis configuration must be an object')
    version = uint(c.get('version'), 1, 2)
    keys(c, ['version', 'address', 'deadline_ms', *WORKLOAD_FIELDS,
             *(['read_api'] if version == 2 else [])])
    if version == 2:
        require(c['read_api'] in ('mget', 'get'), 'invalid Redis read API')
        if c['read_api'] == 'get':
            require(c['batch_size'] == 1 and c['read_percent'] == 100,
                    'Redis GET requires batch size one and read-only traffic')
    uint(c['deadline_ms'], 1, 30_000)
    address = c['address']
    require(type(address) is str and len(address) <= 128, 'invalid Redis address')
    host, port = address.rsplit(':', 1)
    ipaddress.ip_address(host.strip('[]'))
    uint(int(port), 1, 65535)
    require(type(c['run_id']) is str and re.fullmatch('[A-Za-z0-9_-]{1,64}', c['run_id']), 'invalid dataset prefix')
    uint(c['seed'])
    for field, lo, hi in [('workers', 1, 256), ('keys', 1, 65_536), ('batch_size', 1, 256),
                         ('value_bytes', 16, 8192), ('read_percent', 0, 100), ('warmup_calls', 0, 100_000),
                         ('measure_ms', 1, 60_000), ('max_calls', 1, 10_000_000)]:
        uint(c[field], lo, hi)
    require(c['batch_size'] <= c['keys'], 'batch exceeds dataset')
    load = c['load']
    require(type(load) is dict, 'load configuration missing')
    offered = None
    if load.get('kind') == 'closed_loop':
        keys(load, ['kind'])
    else:
        keys(load, ['kind', 'batches_per_second'])
        require(load['kind'] == 'fixed_rate', 'unknown load mode')
        rate = uint(load['batches_per_second'], 1, 2_000_000)
        offered = (c['measure_ms'] * rate + 999) // 1000
        require(offered <= c['max_calls'], 'offered slots exceed call budget')
    n = c['batch_size']
    bulk = lambda size: 1 + len(str(size)) + 2 + size + 2
    array = lambda size: 1 + len(str(size)) + 2
    size = dict(mget_request_bytes=array(1+n) + bulk(4) + n*bulk(len(c['run_id'])+17),
                mset_request_bytes=array(1+2*n) + bulk(4) + n*(bulk(len(c['run_id'])+17)+bulk(c['value_bytes'])),
                mget_response_bytes=array(n) + n*bulk(c['value_bytes']),
                maximum_input_items_in_flight=c['workers']*n,
                maximum_input_payload_bytes_in_flight=c['workers']*n*(len(c['run_id'])+17+c['value_bytes']))
    if c.get('read_api') == 'get':
        size.update(get_request_bytes=array(2)+bulk(3)+bulk(len(c['run_id'])+17),
                    get_response_bytes=bulk(c['value_bytes']))
    require(max(size[k] for k in ('mget_request_bytes', 'mset_request_bytes', 'mget_response_bytes')) <= 1_048_576,
            'encoded Redis batch exceeds bound')
    require(size['maximum_input_payload_bytes_in_flight'] <= 64*1024*1024, 'pending input exceeds bound')
    return offered, size


def metrics_check(m, c, phase):
    keys(m, ['operations', 'outcomes', 'reasons', 'histogram_subdivisions', 'valid', 'statistics'])
    point = c.get('read_api') == 'get' and phase in ('warmup', 'measurement')
    equal(m['operations'], ['get' if point else 'mget', 'mset'], 'operation vocabulary differs')
    equal(m['outcomes'], ['success', 'unknown_write', 'read_failure'], 'outcome vocabulary differs')
    equal(m['reasons'], ['success', 'deadline', 'io', 'server_error', 'protocol', 'data_integrity'], 'reason vocabulary differs')
    uint(m['histogram_subdivisions'], 64, 64)
    require(m['valid'] is True, 'invalid measurement population')
    require(type(m['statistics']) is list and len(m['statistics']) == 2, 'missing operations')
    fixed = phase == 'measurement' and c['load']['kind'] == 'fixed_rate'
    counts = []
    successes = items = commands = 0
    for kind, op in enumerate(m['statistics']):
        keys(op, ['populations', 'reasons', 'dispatch_lateness', 'connection_attempts', 'connection_failures', 'command_attempts', 'data_failures'])
        uint(op['data_failures'], 0, 0)
        require(type(op['populations']) is list and len(op['populations']) == 3, 'missing failure population')
        calls = whole_sum = scheduled_sum = 0
        scheduled_buckets = [0]*3776
        minima, maxima = [], []
        for outcome, p in enumerate(op['populations']):
            keys(p, ['calls', 'input_items', 'completed_before_cutoff', 'whole_call', 'client_call', 'scheduled_to_completion'])
            n = uint(p['calls'], 0, 10_500_000)
            before = uint(p['completed_before_cutoff'], 0, n)
            count = uint(p['input_items'])
            if phase in ('warmup', 'measurement'):
                require(count == n*c['batch_size'], 'batch input population differs')
            else:
                require(n <= count <= n*c['batch_size'], 'setup input population differs')
            if phase != 'measurement':
                require(outcome == 0 or n == 0, 'failed setup or verification')
                require(before == n, 'setup cutoff count differs')
            require(n == 0 or not ((kind == 0 and outcome == 1) or (kind == 1 and outcome == 2)), 'wrong operation outcome')
            whole, client, scheduled = [histogram_check(p[name]) for name in ('whole_call', 'client_call', 'scheduled_to_completion')]
            require(whole['count'] == client['count'] == n and scheduled['count'] == (n if fixed else 0), 'latency population differs')
            require(whole['sum_ns'] >= client['sum_ns'], 'client time exceeds whole call')
            dominance_check(whole, client)
            if fixed:
                dominance_check(scheduled, whole)
                if n:
                    minima.append(scheduled['min_ns']); maxima.append(scheduled['max_ns'])
                for i, count in enumerate(scheduled['buckets']):
                    scheduled_buckets[i] += count
            calls += n
            whole_sum += whole['sum_ns']; scheduled_sum += scheduled['sum_ns']
        reasons = vector(op['reasons'], 6)
        require(sum(reasons) == calls and reasons[0] == op['populations'][0]['calls'], 'terminal reason population differs')
        require(reasons[4] == reasons[5] == 0, 'protocol or data integrity failure')
        connects = uint(op['connection_attempts'], 0, calls)
        failed = uint(op['connection_failures'], 0, connects)
        attempted = uint(op['command_attempts'], 0, calls)
        require(attempted + failed == calls and attempted >= reasons[0] + reasons[3], 'command/connection accounting differs')
        late = histogram_check(op['dispatch_lateness'])
        require(not (point and kind == 1) or calls == 0, 'point GET stage contains writes')
        require(late['count'] == (calls if fixed else 0), 'dispatch lateness population differs')
        require(scheduled_sum == (whole_sum+late['sum_ns'] if fixed else 0), 'scheduled time omits delayed dispatch')
        if fixed:
            dominance_check(dict(count=calls, buckets=scheduled_buckets, min_ns=min(minima) if minima else None,
                                 max_ns=max(maxima) if maxima else None), late)
        counts.append(calls); successes += reasons[0]; items += op['populations'][0]['input_items']; commands += attempted
    return dict(calls=counts, successes=successes, successful_items=items, attempts=commands)


def report_check(r, c, b):
    keys(r, [*REPORT_FIELDS, 'protocol', 'preconnected_workers'])
    uint(r['version'], 1, 2)
    equal(r['version'], c['version'], 'report and configuration versions differ')
    model = ('bounded_redis_point_get_diagnostic' if c.get('read_api') == 'get'
             else 'bounded_redis_batch_performance')
    require(r['workload_model'] == model and r['protocol'] == 'resp2', 'wrong reference protocol/model')
    require(r['complete'] is True and r['failure'] is None, 'measurement incomplete')
    require(r['full_history_recorded'] is False and r['independently_checked'] is False, 'aggregate client claims history acceptance')
    equal(r['configuration'], c, 'reported configuration differs')
    equal(r['build'], b, 'reported build differs')
    offered, size = config_check(c)
    equal(r['wire_sizes'], size, 'encoded sizes or in-flight bounds differ')
    uint(r['process_id'], 1, (1 << 32)-1)
    uint(r['runtime_threads'], 2, 2)
    uint(r['preconnected_workers'], c['workers'], c['workers'])
    uint(r['measurement_start_unix_ns'], 1)
    uint(r['task_failures'], 0, 0)
    require(r['stop_reason'] in ('duration', 'operation_limit'), 'invalid stop reason')
    equal(r['timing_eligible'], r['stop_reason'] == 'duration' and b['profile'] == 'release' and not b['dirty'], 'timing eligibility differs')
    keys(r['metrics'], PHASES)
    metrics = {phase: metrics_check(r['metrics'][phase], c, phase) for phase in PHASES}
    result = cohort_check(r, c, metrics, offered, call_latency='client_call')
    result['command_attempts'] = result.pop('attempts')
    return result


def validate(directory, build_directory, expected_config=None, expected_revision=None, require_timing=False):
    directory, retained = Path(directory), Path(build_directory)
    raw = bounded(directory/'report.json', 16*1024*1024)
    r = strict_json(raw)
    config_raw = bounded(directory/'config.json', 65_536)
    c = strict_json(config_raw)
    if expected_config is not None:
        equal(c, strict_json(bounded(expected_config, 65_536)), 'requested configuration differs')
    build_raw, b, features = build_check(directory, retained, expected_revision, reference=True)
    result = report_check(r, c, b)
    require(r['config_sha256'] == sha(config_raw) and r['build_sha256'] == sha(build_raw), 'artifact digest differs')
    if require_timing:
        require(r['timing_eligible'] is True and expected_config is not None and expected_revision is not None, 'timing requires exact clean release/configuration')
    result.update(accepted=True, version=1, report_sha256=sha(raw), revision=b['revision'], dirty=b['dirty'],
                  profile=b['profile'], cargo_features=features, client_timing_eligible=r['timing_eligible'],
                  full_history_independently_checked=False, environment_independently_checked=False,
                  scope='Redis aggregate arithmetic and retained build inputs; no server identity, isolation, Raft or performance-comparison acceptance')
    return result


def paired_configuration(redis, native):
    config_check(redis)
    from batch_benchmark_report import config_check as native_config_check
    native_config_check(native)
    redis_api = redis.get('read_api', 'mget')
    native_api = native.get('read_api', 'batch_get')
    require((redis_api, native_api) in (('mget', 'batch_get'), ('get', 'point_get')),
            'paired read APIs differ')
    for field in WORKLOAD_FIELDS:
        equal(redis[field], native[field], 'paired workload differs: '+field)
    equal(redis['deadline_ms'], native['client']['deadline_ms'], 'paired deadlines differ')
    result = {field: redis[field] for field in WORKLOAD_FIELDS}
    if redis['version'] == 2 or native['version'] == 2:
        result['read_api_pair'] = {'redis': redis_api, 'native': native_api}
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
        json.dump(result, stream, indent=2, sort_keys=True); stream.write('\n')
    print('PASS: Redis batch aggregate report validated; enclosing acceptance remains separate')


if __name__ == '__main__':
    main()
