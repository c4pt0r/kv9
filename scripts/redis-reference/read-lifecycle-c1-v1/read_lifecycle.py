#!/usr/bin/env python3
"""Read-only c1 sampled lifecycle reader; never launches a fixture or profiler."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import traceback

if not __debug__:
    raise RuntimeError('PYTHONOPTIMIZE=0 is required')
sys.dont_write_bytecode = True
PHASES = ['queue', 'quorum', 'apply', 'notification', 'total']
NODES = {'1', '2', '3'}
ZERO = ['public_rpc_in_flight', 'public_rpc_queued', 'public_rpc_running',
        'public_rpc_encoded_bytes', 'raft_async_apply_in_flight',
        'raft_async_apply_queued', 'raft_async_read_in_flight',
        'raft_async_read_queued', 'raft_async_read_active', 'raft_async_read_active_groups']
IDENTITY = ['pid', 'node_id', 'process_start_ticks', 'process_boot_id',
            'cluster_id', 'bootstrap_generation', 'store_incarnation']
CHECKER_SHA = '97d294d8b432dbda85073f2dae80f01308a640b686eb35e02689562c0c405cd7'
C1_PROTOCOL = dict(version=2, protocol_id='kv9-read-lifecycle-c1-v1',
    instrumented=True, throughput_acceptance=False, cpu_profiling=False,
    server_revision='d3dcea0355dd6c4ec23f0833708f6dc978412093',
    client_revision='03c1c776a5dd7d1cc67491ab253e02ce51665bf8',
    server_sha256='08e8103b5230933bf65b4b605fe864803f766a1b3cf3776d7bb796bf4e3611e1',
    client_sha256='22ca0883ca2900fab0b457d87ebc6bc50662d7145af0846304f02b5841a6d30b',
    measure_ms=5000, workers=1, keys=4096, batch_size=1, value_bytes=128,
    read_percent=100, warmup_calls=128, max_calls=10000000, deadline_ms=1500,
    client_cpus=[0, 1], server_cpus=[2, 3, 4, 5],
    observer_cpus=list(range(6, 16)) + list(range(22, 32)),
    storage='Volatile tmpfs data; normal Raft quorum and sync calls, but NO disk durability or power-loss guarantee',
    exclusive_host=False, profiles=['point_get', 'batch_get'])


def require(ok, message):
    if not ok:
        raise ValueError(message)


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def check_c1_protocol(protocol):
    require(json.dumps({k: protocol[k] for k in C1_PROTOCOL}, sort_keys=True) ==
            json.dumps(C1_PROTOCOL, sort_keys=True), 'frozen c1 protocol differs')
    require(not set(protocol) & {'profiler_cpus', 'event', 'frequency_hz', 'call_graph',
                                'recording_seconds', 'max_perf_bytes'}, 'unexpected CPU profiling protocol')


def check_c1_configuration(api, config, report, config_sha256):
    require(api in ('point_get', 'batch_get'), 'c1 API differs')
    expected = dict(version=2, rpc_transport='tonic_stream', read_api=api,
                    run_id='prfget' if api == 'point_get' else 'prfbat', seed=71,
                    workers=1, keys=4096, batch_size=1, value_bytes=128, read_percent=100,
                    warmup_calls=128, measure_ms=5000, max_calls=10000000,
                    load={'kind': 'closed_loop'})
    require(set(config) == set(expected) | {'client'} and
            json.dumps({k: config[k] for k in expected}, sort_keys=True) ==
            json.dumps(expected, sort_keys=True), 'frozen c1 workload differs')
    client = dict(version=1, epoch_conf_ver=1, epoch_version=1, max_in_flight=1,
                  max_attempts=6, deadline_ms=1500, retry_backoff_ms=5)
    require(set(config['client']) == set(client) | {'peers', 'keyspace_id'} and
            json.dumps({k: config['client'][k] for k in client}, sort_keys=True) ==
            json.dumps(client, sort_keys=True), 'frozen c1 SDK limits differ')
    require(json.dumps(report['configuration'], sort_keys=True) == json.dumps(config, sort_keys=True) and
            report['config_sha256'] == config_sha256, 'c1 report/requested configuration binding differs')
    build = report['build']
    require(build['revision'] == C1_PROTOCOL['client_revision'] and build['dirty'] is False and
            build['profile'] == 'release' and build['binary_sha256'] == C1_PROTOCOL['client_sha256'],
            'c1 report client build differs')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--run', type=Path, required=True)
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--root-session', type=int, required=True)
    parser.add_argument('--root-exit-code', type=int, choices=(0,), required=True)
    parser.add_argument('--compare-prior', type=Path)
    args = parser.parse_args()
    require(sorted(os.sched_getaffinity(0)) == list(range(6, 16)) + list(range(22, 32)), 'CPU mask differs')
    run, source, out = args.run.resolve(), args.source.resolve(), args.output.resolve()
    require(not out.is_relative_to(run) and not out.is_relative_to(source), 'output overlaps inputs')
    out.mkdir()
    inputs = {}
    result = {'accepted': False, 'scope': 'Sampled successful lifecycle envelope; no throughput acceptance.',
              'reader_sha256': sha(__file__), 'run': str(run), 'source': str(source),
              'protocol_id': C1_PROTOCOL['protocol_id'], 'cpu_profiling': False,
              'parent_confirmed_session': args.root_session,
              'parent_confirmed_exit_code': args.root_exit_code}

    def capture(path):
        path = Path(path).resolve()
        data = path.read_bytes()
        item = {'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()}
        require(path not in inputs or inputs[path] == item, 'input changed during read')
        inputs[path] = item
        return data

    def read(path):
        return json.loads(capture(path))

    def drained(state):
        require(state['bootstrap_state'] == 'Serving' and state['fatal'] == '', 'not healthy Serving')
        require(all(state[k] == '0' for k in ZERO), 'public/read/apply not drained')
        require(state['raft_async_apply_stopped'] == state['raft_async_read_stopped'] == 'false', 'registry stopped')
        require(state['applied_term'] == state['driver_applied_term'] and
                state['applied_index'] == state['driver_applied_index'], 'applied pair differs')

    try:
        protocol = read(run / 'protocol.json')
        check_c1_protocol(protocol)
        require(protocol['instrumented'] is True and protocol['throughput_acceptance'] is False, 'wrong diagnostic scope')
        require(protocol['profiles'] == ['point_get', 'batch_get'], 'API inventory differs')
        require(read(run / 'summary.json')['complete'] is True, 'runtime incomplete')
        require(read(run / 'readback-summary.json')['complete'] is True, 'fixture readback incomplete')
        manifest = read(run / 'build-bindings.json')['server']
        require(manifest['revision'] == protocol['server_revision'] and manifest['dirty'] is False and
                manifest['binaries']['kv9']['sha256'] == protocol['server_sha256'], 'server manifest binding differs')
        require(subprocess.check_output(['git', '-C', str(source), 'rev-parse', 'HEAD'], text=True).strip() ==
                protocol['server_revision'], 'source revision differs')
        for relative in ['crates/raft/src/read_profile.rs', 'crates/raft/src/async_read.rs',
                         'crates/raft/src/driver.rs', 'crates/raft/src/lib.rs',
                         'crates/server/src/runtime.rs', 'crates/server/src/observability.rs',
                         'scripts/check-latency-metrics.py']:
            data = capture(source / relative)
            require(hashlib.sha256(data).hexdigest() == manifest['sources'][relative], 'source hook/build differs: ' + relative)
        checker_path = source / 'scripts/check-latency-metrics.py'
        require(sha(checker_path) == CHECKER_SHA, 'original 31-metric checker changed')
        spec = importlib.util.spec_from_file_location('unchanged_latency_checker', checker_path)
        checker = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(checker)
        require(len(checker.NAMES) == 31, 'metric count differs')
        cohorts = []
        for api in protocol['profiles']:
            directory = run / api
            config = read(directory / 'requested-config.json')
            report = read(directory / 'run/report.json')
            check_c1_configuration(api, config, report, inputs[(directory / 'requested-config.json').resolve()]['sha256'])
            metrics = {side: read(directory / (side + '-metrics.json')) for side in ['before', 'after']}
            states = {side: read(directory / (side + '-status.json')) for side in ['before', 'after']}
            resources = {side: read(directory / (side + '-resources.json')) for side in ['before', 'after']}
            drains = {label: read(directory / (label + '-fresh-drain.json'))
                      for label in ['before', 'post-client', 'post-readback']}
            client = read(directory / 'client-identity.json')
            exited = read(directory / 'client-exit.json')
            require(exited['exit_code'] == 0, 'client did not exit successfully')
            require(read(directory / 'fixture-result.json')['complete'] is True, 'fixture incomplete')
            for side in metrics:
                require(set(metrics[side]) == set(states[side]) == set(resources[side]['processes']) == NODES,
                        'endpoint voter inventory differs')
            for label, drain in drains.items():
                require(set(drain['baseline']) == set(drain['first_advances']) == set(drain['final']) == NODES, 'drain voter inventory differs')
                require(0 <= drain['completed_unix_ns'] - drain['started_unix_ns'] <= 20_000_000_000, 'drain time exceeds bound')
                for node in sorted(NODES):
                    first = drain['first_advances'][node]
                    triple = [drain['baseline'][node], first['status'], drain['final'][node]]
                    require(drain['started_unix_ns'] <= first['observed_unix_ns'] <= drain['completed_unix_ns'], 'drain observation outside bounds')
                    require(int(triple[0]['metrics_export_successes']) < int(triple[1]['metrics_export_successes']) <
                            int(triple[2]['metrics_export_successes']), 'two serial export advances absent')
                    for state in triple:
                        require(all(state[k] == states['before'][node][k] for k in IDENTITY), 'drain lifetime changed')
                    drained(triple[1]); drained(triple[2])
                require(len({(s['applied_term'], s['applied_index']) for s in drain['final'].values()}) == 1, 'replica drain positions differ')
            require(exited['observed_unix_ns'] <= drains['post-client']['started_unix_ns'] <=
                    drains['post-client']['completed_unix_ns'] <= drains['post-readback']['started_unix_ns'], 'post-exit drain ordering differs')
            nodes, coverage = [], []
            for node in sorted(NODES):
                before, after = metrics['before'][node], metrics['after'][node]
                sb, sa = states['before'][node], states['after'][node]
                require(all(sb[k] == sa[k] for k in IDENTITY), 'endpoint lifetime changed')
                require(checker.identity(before) == checker.identity(after), 'metrics process/exporter changed')
                require(int(before['captured_unix_ns']) < int(after['captured_unix_ns']) and
                        before['exporter_uptime_ns'] < after['exporter_uptime_ns'], 'export time did not advance')
                for side, doc, state in [('before', before, sb), ('after', after, sa)]:
                    checker.validate(doc)
                    require(int(state['node_id']) == doc['node_id'] == int(node) and int(state['pid']) == doc['process_id'], 'status/metric identity mismatch')
                    resource = resources[side]['processes'][node]
                    require(resource['pid'] == int(state['pid']) and resource['start_ticks'] == int(state['process_start_ticks']), 'resource lifetime differs')
                    require(doc['export_failures_before_capture'] == 0 and doc['export_failures_saturated'] is False and
                            state['metrics_export_failures'] == '0' and state['metrics_export_failures_saturated'] == 'false' and
                            state['metrics_export_last'] == 'success', 'export failure present')
                    require(doc['apply_lag']['lag_entries'] == 0, 'endpoint apply lag')
                    drained(state)
                    hs = [checker.histogram(doc, 'raft_async_read_profile_' + phase) for phase in PHASES]
                    require(len({h['count'] for h in hs}) == 1 and sum(h['sum_ns'] for h in hs[:-1]) == hs[-1]['sum_ns'], 'endpoint phase population/partition differs')
                    for phase in PHASES:
                        for outcome in checker.OUTCOMES[1:]:
                            h = checker.histogram(doc, 'raft_async_read_profile_' + phase, outcome)
                            require(h['count'] == h['sum_ns'] == 0, 'invalid trace or non-success profile outcome')
                bcap, acap = int(before['captured_unix_ns']), int(after['captured_unix_ns'])
                require(drains['before']['completed_unix_ns'] < bcap < client['observed_unix_ns'] < exited['observed_unix_ns'] <
                        drains['post-readback']['completed_unix_ns'] < acap, 'capture does not bracket drained client envelope')
                require(int(sb['metrics_export_successes']) >= int(drains['before']['final'][node]['metrics_export_successes']) and
                        int(sa['metrics_export_successes']) >= int(drains['post-readback']['final'][node]['metrics_export_successes']) >
                        int(sb['metrics_export_successes']), 'endpoint export counters stale')
                phases = []
                for phase in PHASES:
                    a, b = [checker.histogram(doc, 'raft_async_read_profile_' + phase) for doc in [before, after]]
                    n, total = b['count'] - a['count'], b['sum_ns'] - a['sum_ns']
                    require(n >= 0 and total >= 0 and all(y >= x for x, y in zip(a['buckets'], b['buckets'])), 'profile counters regressed')
                    phases.append(dict(phase=phase, before_count=a['count'], after_count=b['count'], count_delta=n, sum_ns_delta=total))
                require(len({p['count_delta'] for p in phases}) == 1 and sum(p['sum_ns_delta'] for p in phases[:-1]) == phases[-1]['sum_ns_delta'], 'delta population/partition differs')
                nodes.append(dict(node_id=node, pid=sb['pid'], start_ticks=sb['process_start_ticks'], boot_id=sb['process_boot_id'],
                                  exporter_created_unix_ns=before['exporter_created_unix_ns'], samples=phases[-1]['count_delta'], phases=phases))
                coverage.append(dict(node_id=node, before_capture_ns=bcap, after_capture_ns=acap,
                                     before_drain_completed_ns=drains['before']['completed_unix_ns'],
                                     after_drain_completed_ns=drains['post-readback']['completed_unix_ns'],
                                     client_started_observed_ns=client['observed_unix_ns'], client_exit_ns=exited['observed_unix_ns']))
            totals = [dict(phase=phase, count_delta=sum(n['phases'][i]['count_delta'] for n in nodes),
                           sum_ns_delta=sum(n['phases'][i]['sum_ns_delta'] for n in nodes)) for i, phase in enumerate(PHASES)]
            require(totals[-1]['count_delta'] > 0, 'no successful lifecycle samples')
            for phase in totals:
                phase['mean_ns'] = phase['sum_ns_delta'] / phase['count_delta']
                phase['share_percent'] = 100 * phase['sum_ns_delta'] / totals[-1]['sum_ns_delta']
            cohorts.append(dict(api=api, workers=1, nodes=nodes, samples=totals[-1]['count_delta'], phases=totals, snapshot_coverage=coverage))
        if args.compare_prior:
            prior = read(args.compare_prior)
            require(prior['accepted'] is True and prior['source_revision'] == protocol['server_revision'], 'wrong prior analysis')
            for actual, expected in zip(cohorts, prior['cohorts'], strict=True):
                require(actual['api'] == expected['api'] and actual['samples'] == expected['samples'], 'prior sample totals differ')
                for a, b in zip(actual['nodes'], expected['nodes'], strict=True):
                    require(a['node_id'] == b['node_id'], 'prior node differs')
                    for x, y in zip(a['phases'], b['phases'], strict=True):
                        require(all(x[k] == y[k] for k in ['phase', 'before_count', 'after_count', 'count_delta', 'sum_ns_delta']), 'prior integer phase arithmetic differs')
            result['prior_exact_integer_agreement'] = True
        require(all(sha(path) == item['sha256'] and path.stat().st_size == item['bytes'] for path, item in inputs.items()), 'input changed during analysis')
        result.update(accepted=True, workers=1, source_revision=protocol['server_revision'], server_sha256=protocol['server_sha256'],
                      cohorts=cohorts, checks=dict(metric_documents=12, metric_inventory=31, stable_process_lifetimes=6,
                      fresh_drains=6, identical_success_populations=True, no_invalid_trace_errors=True, exact_phase_sum_conservation=True))
        print(json.dumps({'accepted': True, 'cohorts': [{'api': c['api'], 'samples': c['samples'], 'phases': c['phases']} for c in cohorts]}))
    except BaseException as error:
        result['failure'] = repr(error)
        result['traceback'] = traceback.format_exc()
        raise
    finally:
        result['inputs'] = {str(path): item for path, item in inputs.items()}
        (out / 'analysis.json').write_text(json.dumps(result, indent=2, sort_keys=True) + '\n')


if __name__ == '__main__':
    main()
