#!/usr/bin/env python3
"""Independently audit one frozen, completed native-batch process fixture.

Never launches a server, workload, fault, build or benchmark. Original inputs stay
unchanged; every checker output and attempted audit is retained in a NEW directory.
"""
import argparse
from collections import Counter
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import time

sys.dont_write_bytecode = True
os.environ['PYTHONDONTWRITEBYTECODE'] = '1'

KINDS = ('get', 'put', 'delete', 'batch_get', 'batch_put')
ZERO = ('public_rpc_in_flight', 'public_rpc_queued', 'public_rpc_running', 'public_rpc_encoded_bytes',
        'raft_async_apply_queued', 'raft_async_apply_in_flight', 'raft_async_read_queued',
        'raft_async_read_active', 'raft_async_read_in_flight', 'raft_async_read_active_groups')


def sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def read(path):
    return json.loads(Path(path).read_text())


def save(path, data):
    Path(path).write_text(json.dumps(data, indent=2, sort_keys=True) + '\n')


def require(value, message):
    if not value:
        raise ValueError(message)


def inventory(root):
    result = {}
    for path in sorted(root.rglob('*')):
        if path.is_symlink():
            raise ValueError('unexpected symlink in native fixture/build input: ' + str(path))
        if path.is_file():
            result[str(path.relative_to(root))] = dict(bytes=path.stat().st_size, sha256=sha(path))
    return result


def execute(args, output, result):
    source, build, raw = args.source.resolve(), args.build.resolve(), args.run.resolve()
    for root in (source, build, raw):
        require(root.is_dir() and output != root and not output.is_relative_to(root), 'audit output overlaps input')
    commands = []

    def command(name, argv):
        started = time.time_ns()
        try:
            done = subprocess.run(argv, cwd=source, text=True, capture_output=True, timeout=120)
        except subprocess.TimeoutExpired as error:
            def text(value):
                return value.decode(errors='replace') if isinstance(value, bytes) else value or ''
            (output / (name + '.stdout')).write_text(text(error.stdout))
            (output / (name + '.stderr')).write_text(text(error.stderr))
            commands.append(dict(name=name, command=argv, cwd=str(source), started_unix_ns=started,
                                 ended_unix_ns=time.time_ns(), exit_code='timeout', timeout_seconds=120))
            save(output / 'commands.json', commands)
            raise
        (output / (name + '.stdout')).write_text(done.stdout)
        (output / (name + '.stderr')).write_text(done.stderr)
        commands.append(dict(name=name, command=argv, cwd=str(source), started_unix_ns=started,
                             ended_unix_ns=time.time_ns(), exit_code=done.returncode))
        save(output / 'commands.json', commands)
        require(done.returncode == 0, f'{name} failed with exit {done.returncode}')
        return done.stdout

    summary = read(raw / 'summary.json')
    require(summary['complete'] is True and not summary.get('failure'), 'fixture is incomplete')
    require(all(summary[key] is True for key in ('workloads_complete', 'owned_lifetimes_exited',
        'owned_children_exited', 'owned_children_identified', 'bound_artifacts_unchanged', 'helpers_unchanged')),
        'fixture acceptance/cleanup flags incomplete')
    require(summary['cleanup_errors'] == [], 'fixture cleanup retained an error')
    require(command('source-head', ['git', 'rev-parse', 'HEAD']).strip() == args.revision,
            'source is not expected frozen revision')
    require(not command('source-status', ['git', 'status', '--porcelain', '--untracked-files=all']),
            'source is dirty')
    manifest = read(build / 'build.json')
    workload = read(build / 'workload/build.json')
    sources = read(build / 'workload/sources.json')
    require(manifest['revision'] == workload['revision'] == sources['revision'] ==
            summary['source_revision'] == args.revision, 'revision lineage mismatch')
    require(manifest['dirty'] is workload['dirty'] is sources['dirty'] is False, 'dirty build evidence')
    require(manifest['sources'] == sources['sources'], 'server/client source inventories differ')
    require(hashlib.sha256(json.dumps(manifest['sources'], sort_keys=True, separators=(',', ':')).encode()).hexdigest()
            == manifest['source_tree_sha256'] == workload['source_tree_sha256'] == sources['source_tree_sha256'],
            'canonical source inventory digest differs')
    for rel, expected in manifest['sources'].items():
        require(isinstance(expected, str) and sha(source / rel) == expected, 'source hash mismatch: ' + rel)
    for rel, expected in summary['helper_sources'].items():
        require(manifest['sources']['scripts/' + rel] == expected == sha(source / 'scripts' / rel),
                'executing helper does not match frozen source: ' + rel)
    for path, expected in summary['bound_artifacts'].items():
        require(sha(path) == expected, 'bound artifact changed: ' + path)
    require(sha(build / 'build.json') == summary['build_sha256'], 'build manifest differs')
    server_hash = sha(build / 'kv9')
    client_hash = sha(build / 'workload/kv9-batch-workload')
    require(server_hash == manifest['binaries']['kv9']['sha256'] == summary['server_sha256'], 'server hash differs')
    require(client_hash == workload['binary_sha256'] == sources['binary_sha256'], 'client hash differs')
    require(sha(build / 'workload/build.json') == manifest['workload_build_sha256'], 'client manifest differs')
    require(all(manifest[key] == workload[key] for key in ('profile', 'rustc')), 'compiler/profile lineage differs')
    artifact_groups = {}
    for name, rel, wanted, build_command in (
        ('server', 'kv9-cargo.jsonl', ('kv9', 'kv9_engine', 'kv9_raft', 'kv9_server'), manifest['binaries']['kv9']['command']),
        ('workload', 'workload/cargo.jsonl', ('kv9-batch-workload', 'kv9_engine', 'kv9_raft', 'kv9_server'), sources['command']),
    ):
        require(build_command[:2] == ['cargo', 'build'] and '--locked' in build_command,
                'not a locked standalone Cargo build')
        require('--bin' in build_command and build_command[build_command.index('--bin') + 1] == wanted[0],
                'wrong standalone target')
        require(not any(arg.startswith(('--features', '--all-features', '--no-default-features')) for arg in build_command),
                'non-default feature build command')
        records = [json.loads(line) for line in (build / rel).read_text().splitlines()]
        require(records[-1] == {'reason': 'build-finished', 'success': True}, 'Cargo build is not successfully terminal')
        selected = {}
        for target in wanted:
            rows = [row for row in records if row.get('reason') == 'compiler-artifact' and row.get('target', {}).get('name') == target]
            require(len(rows) == 1 and rows[0]['features'] == [] and rows[0]['profile']['test'] is False,
                    'missing, ambiguous, test or non-default runtime artifact: ' + target)
            require(Path(rows[0]['manifest_path']).is_relative_to(source), 'Cargo artifact comes from a different checkout')
            selected[target] = rows[0]
        require(selected[wanted[0]]['executable'] and selected[wanted[0]]['target']['kind'] == ['bin'],
                'target is not the standalone executable')
        require(summary['feature_attestation'][name] == dict(command=build_command,
                artifacts=selected, cargo_sha256=sha(build / rel)), 'retained feature attestation differs')
        artifact_groups[name] = selected
    raw_before, build_before = inventory(raw), inventory(build)
    save(output / 'original-inputs.json', dict(run=str(raw), build=str(build), run_files=raw_before, build_files=build_before))
    fixture = read(raw / 'fixture/fixture.json')
    require(fixture['complete'] is True and fixture['target'] == 'wal' and set(fixture['addresses']) == {'1', '2', '3'},
            'fixture is not three completed ordinary WAL voters')
    require(len(set(fixture['addresses'].values())) == 3 and all(a.startswith('127.0.0.1:') for a in fixture['addresses'].values()),
            'ordinary endpoints are not unique loopback addresses')
    children = summary['owned_children']
    require(len(children) == 7 and all(row['identity_captured'] is True and row['exit_code'] is not None for row in children),
            'expected five server and two client lifetimes are not complete')
    child_by_id = {(row['pid'], row['start_ticks']): row for row in children}
    require(len(child_by_id) == 7, 'duplicate owned process lifetime')
    boot = Path('/proc/sys/kernel/random/boot_id').read_text().strip()
    lifetimes = summary['lifetimes']
    require(len(lifetimes) == 5, 'expected initial three plus two restarted voter lifetimes')
    for row in lifetimes:
        identity = (row['pid'], row['start_ticks'])
        require(identity in child_by_id and row['exited'] is True and row['executable_sha256'] == server_hash
                == child_by_id[identity]['executable_sha256'], 'voter identity/executable mismatch')
        require(row['cpu_affinity'] == list(range(8, 16)) + list(range(22, 32)), 'voter CPU placement differs')
        state = row['status']
        require(state['pid'] == str(row['pid']) and state['process_start_ticks'] == str(row['start_ticks'])
                and state['process_boot_id'] == row['boot_id'], 'status writer is not exact lifetime')
        listener = row['ordinary_listener']; endpoint = fixture['addresses'][str(row['node_id'])]
        require(listener['experimental_environment_absent'] is True and listener['pid'] == row['pid']
                and listener['advertised_endpoint'] == endpoint, 'ordinary listener identity differs')
        require(len(listener['listener_inodes']) == 1 and listener['listener_inodes'][0]['table'] == 'tcp'
                and listener['listener_inodes'][0]['local_address'] == f'0100007F:{int(endpoint.rsplit(":", 1)[1]):04X}',
                'listener is not exactly advertised ordinary port')
    for row in children:
        path = Path('/proc') / str(row['pid']) / 'stat'
        if path.exists():
            ticks = int(path.read_text().rsplit(')', 1)[1].split()[19])
            old_boot = next((r['boot_id'] for r in lifetimes if r['pid'] == row['pid']), lifetimes[0]['boot_id'])
            require(old_boot != boot or ticks != row['start_ticks'], 'owned lifetime remains alive')
    require([case['transport'] for case in summary['cases']] == ['tonic_stream', 'tonic_unary'], 'transport arms differ')
    # Load only the verified source checkout. The report validator checks the
    # recorder contract; a separate CLI checker replays the whole atomic history.
    sys.path.insert(0, str(source / 'scripts'))
    import batch_workload_report as validator
    from checker import History, verify_witness
    cases = []
    for case in summary['cases']:
        transport = case['transport']; name = 'batch-' + transport.replace('_', '-'); directory = raw / name
        requested = read(raw / (name + '-config.json')); report = read(directory / 'report.json'); config = report['configuration']
        require(('rpc_transport' in requested) == case['input_transport_explicit'] == (transport == 'tonic_unary'),
                'default streaming / explicit unary selection changed')
        require(config == dict(requested, rpc_transport=transport), 'requested and effective configurations differ')
        expected = dict(version=1, run_id=name, keyspace_name=name, workers=4, keys=4, batch_size=8,
            value_bytes=128, seed=40, mix=[10, 10, 10, 35, 35], max_calls=2000,
            measure_ms=30000, interval_ms=20, history_bytes=128 * 1024 * 1024, rpc_transport=transport)
        require({k:v for k,v in config.items() if k != 'client'} == expected, 'predeclared native workload shape differs')
        peers = {str(peer['node_id']): peer['address'] for peer in config['client']['peers']}
        require(peers == fixture['addresses'], 'client peers differ from actual voters')
        require({k:v for k,v in config['client'].items() if k not in ['keyspace_id', 'peers']} == dict(
            version=1, epoch_conf_ver=1, epoch_version=1, max_in_flight=4, max_attempts=6, deadline_ms=1500, retry_backoff_ms=100),
            'client concurrency/deadline/retry contract changed')
        client = case['client_identity']; identity = (client['pid'], client['start_ticks'])
        require(identity in child_by_id and client['pid'] == report['process_id'] and child_by_id[identity]['exit_code'] == 0,
                'client report/lifetime/exit differs')
        require(client['executable_sha256'] == child_by_id[identity]['executable_sha256'] == client_hash
                and client['cpu_affinity'] == [6, 7] and report['runtime_threads'] == 2, 'client identity/placement differs')
        checked = validator.validate(directory, build / 'workload', expected_revision=args.revision, seconds=60)
        require(checked['accepted'] and checked['full_history_independently_checked'], 'native validator did not accept full history')
        save(output / (name + '-report-checked.json'), checked)
        require(case['checked'] == read(raw / (name + '-checked.json')), 'original checked result is not the retained one')
        require(case['checked']['report_sha256'] == checked['report_sha256'], 'checker report input changed')
        destination = output / (name + '-history-checked.json')
        command(name + '-raw-history', ['python3', 'scripts/history/checker.py', str(directory / 'history.jsonl'),
            '--output', str(destination), '--acceptance', '--require', *KINDS, '--max-states', '400000', '--seconds', '60'])
        verdict = read(destination); require(verdict['verdict'] == 'valid', 'complete atomic history invalid/inconclusive')
        records = [json.loads(line) for line in (directory / 'history.jsonl').read_text().splitlines()]
        parsed = History.parse(records); require(verify_witness(parsed, verdict['witness']), 'raw atomic witness does not replay')
        invocations = {r['id']: r for r in records if r['type'] == 'invoke'}
        returns = {r['id']: r for r in records if r['type'] == 'return'}
        require(set(invocations) == set(returns), 'incomplete operation history')
        offset = report['wall_anchor_unix_ns'] - report['wall_anchor_monotonic_ns']; windows = case['windows']
        require([window['label'] for window in windows] == ['voter-lost', 'voter-restarted'], 'fault window labels differ')
        require(windows[0]['start_unix_ns'] == case['fault_start_unix_ns'] and
                windows[0]['start_unix_ns'] < windows[0]['end_unix_ns'] < windows[1]['start_unix_ns'] < windows[1]['end_unix_ns'],
                'fault/restart intervals overlap or are invalid')
        require(case['lost_leader'] != case['new_leader'] and str(case['lost_leader']) in peers and str(case['new_leader']) in peers,
                'leader change identity is invalid')
        old = [r for r in lifetimes if r['node_id'] == case['lost_leader'] and r['observed_unix_ns'] < windows[0]['start_unix_ns']]
        require(old, 'lost voter has no prior bound lifetime'); previous = max(old, key=lambda r:r['observed_unix_ns'])
        require(child_by_id[(previous['pid'], previous['start_ticks'])]['exit_code'] == -9, 'lost voter was not killed')
        successors = [r for r in lifetimes if r['node_id'] == case['lost_leader'] and
            windows[0]['end_unix_ns'] < r['observed_unix_ns'] < windows[1]['start_unix_ns']]
        require(len(successors) == 1 and successors[0]['start_ticks'] != previous['start_ticks'],
                'restart has no distinct bound original voter lifetime')
        recomputed = []
        for window in windows:
            success = {kind: [] for kind in KINDS}
            for oid, returned in returns.items():
                call = invocations[oid]
                if returned['outcome'] == 'ok' and call['phase'] == 'measure' and (
                    window['start_unix_ns'] + 1_000_000 <= offset + call['monotonic_ns'] <= offset + returned['monotonic_ns']
                    <= window['end_unix_ns'] - 1_000_000):
                    success[call['op']].append(oid)
            require(success == window['successes'] and success['batch_get'] and success['batch_put'],
                    'no complete successful batch operations inside claimed fault/restart interval')
            recomputed.append(dict(label=window['label'], counts={kind:len(ids) for kind,ids in success.items()}, operation_ids=success))
        freshness = case['drain_freshness']; final = case['drained_status']
        require(set(final) == set(freshness['baseline']) == set(freshness['first_advances']) == {'1','2','3'},
                'drain omitted a voter')
        require(freshness['started_unix_ns'] > windows[1]['end_unix_ns'] and
                freshness['completed_unix_ns'] > freshness['started_unix_ns'], 'drain predates finished workload')
        for node, state in final.items():
            baseline = freshness['baseline'][node]; first = freshness['first_advances'][node]
            for key in ['pid', 'process_start_ticks', 'process_boot_id']:
                require(baseline[key] == first['status'][key] == state[key], 'drain crosses status writers')
            require(int(baseline['metrics_export_successes']) < int(first['status']['metrics_export_successes']) <
                    int(state['metrics_export_successes']), 'two serial fresh export advances missing')
            require(freshness['started_unix_ns'] <= first['observed_unix_ns'] <= freshness['completed_unix_ns'],
                    'fresh export observation outside drain interval')
            require(state['bootstrap_state'] == 'Serving' and state['fatal'] == '' and
                    state['raft_async_apply_stopped'] == state['raft_async_read_stopped'] == 'false' and
                    all(state[key] == '0' for key in ZERO), 'final healthy/drained contract failed')
            require(any(row['node_id'] == int(node) and str(row['pid']) == state['pid'] and
                str(row['start_ticks']) == state['process_start_ticks'] and row['boot_id'] == state['process_boot_id'] for row in lifetimes),
                'final status writer lacks captured executable lifetime')
        points = [r for r in invocations.values() if r['phase'] == 'measure' and r['op'] in ['put','delete']]
        batches = [r for r in invocations.values() if r['phase'] == 'measure' and r['op'] in ['batch_get','batch_put']]
        overlaps = []
        for point in points:
            for batch in batches:
                keys = batch['args']['keys'] if batch['op'] == 'batch_get' else [pair[0] for pair in batch['args']['pairs']]
                if point['args']['key'] in keys and point['seq'] < returns[batch['id']]['seq'] and batch['seq'] < returns[point['id']]['seq']:
                    overlaps.append([point['id'],batch['id']])
        require(overlaps, 'history has no overlapping point mutation and batch on a shared key')
        cases.append(dict(transport=transport,history_sha256=sha(directory/'history.jsonl'),operations=len(invocations),
            outcomes=dict(Counter(row['outcome'] for row in returns.values())),traffic_successes=checked['traffic_successes'],
            windows=recomputed,point_batch_overlap_count=len(overlaps),point_batch_overlap_examples=overlaps[:16],
            fresh_drained_voters=3,client_pid=client['pid'],native_checker_states=checked['history']['states'],
            raw_checker_states=verdict['states']))
    require(raw_before == inventory(raw) and build_before == inventory(build), 'original runtime/build artifacts changed during audit')
    for rel, expected in manifest['sources'].items():
        require(sha(source / rel) == expected, 'source changed during independent audit: ' + rel)
    result.update(complete=True, accepted=True, revision=args.revision, source=str(source), build=str(build), run=str(raw),
        server_sha256=server_hash, client_sha256=client_hash, source_files=len(manifest['sources']),
        source_tree_sha256=manifest['source_tree_sha256'], source_unchanged=True, original_inputs_unchanged=True,
        server_lifetimes=5, client_lifetimes=2, owned_lifetimes_exited=True, cases=cases, commands=commands,
        scope='Independent readback of one frozen default-feature native atomic batch process fixture, stream default plus explicit unary on normal endpoints; two complete atomic histories and source-bound SIGKILL/restart intervals. One local host, ordinary WAL, no benchmark or actual Chaos Mesh/native-batch Chaos acceptance. Interval timestamps and listener/proc records are retained observations from the source-bound fixture, not an external continuous observer.')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for arg in ('source', 'build', 'run', 'output'):
        parser.add_argument('--' + arg, type=Path, required=True)
    parser.add_argument('--revision', required=True)
    args = parser.parse_args(); output = args.output.resolve()
    for root in (args.source.resolve(), args.build.resolve(), args.run.resolve()):
        require(root.is_dir() and output != root and not output.is_relative_to(root), 'audit output overlaps input')
    output.mkdir(parents=True, exist_ok=False)
    result = dict(complete=False, accepted=False, started_unix_ns=time.time_ns(), audit_script_sha256=sha(__file__))
    save(output / 'audit.json', result)
    try:
        execute(args, output, result)
    except Exception as error:
        result['failure'] = repr(error)
        raise
    finally:
        result['ended_unix_ns'] = time.time_ns()
        save(output / 'audit.json', result)
    print(json.dumps({key:result[key] for key in ('complete','accepted','revision','server_lifetimes','client_lifetimes','cases')}, sort_keys=True))


if __name__ == '__main__':
    main()
