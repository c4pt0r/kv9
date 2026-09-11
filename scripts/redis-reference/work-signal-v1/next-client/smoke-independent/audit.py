#!/usr/bin/env python3
"""Read back one completed v3 correctness smoke; never launch a workload."""
import argparse
import base64
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import time

if not __debug__:
    raise RuntimeError('assertions must remain enabled')
sys.dont_write_bytecode = True
SOURCE = Path('/tmp/kv9-point-write-measurement-v3')
PREP = Path('/tmp/kv9-point-write-v3-smoke-preparation')
RAW = Path('/tmp/kv9-point-write-v3-smoke-first')
HERE = Path(__file__).resolve().parent
REV = '0be806d9671e2c50701a64aa7889c8859b7648ba'
PINS = {'native': '8da9af469f962a938027d1970141bbe4622f7d42b2b795f720e288fb3f8d5957',
        'redis': '5a8ac274b8cc6548a072f5305b04936a08cc4d1ba84d50150f675bab563049af'}


def require(value, message):
    if not value:
        raise ValueError(message)


def read(path):
    return json.loads(Path(path).read_text())


def sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def save(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2, sort_keys=True)
        stream.write('\n')


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def placement(row, mask):
    require(row['expected'] == row['process'] == mask and row['threads'] and
            all(cpus == mask for cpus in row['threads'].values()), 'thread placement differs')


def execute(args, output, result):
    require(args.root_exit_code == 0, 'root runtime did not exit successfully')
    require(sha(PREP/'run.py') == args.runner_sha256, 'frozen runner changed')
    terminal = read(PREP/'terminal-first.json')
    require(args.root_session == 19187 and terminal['exit_code'] == 0 and
            terminal['driver_sha256'] == args.runner_sha256 and
            terminal['log_sha256'] == sha(PREP/'runtime-first.log') and
            not Path('/proc', str(terminal['pid'])).exists(), 'runtime terminal identity differs')
    summary = read(RAW/'summary.json')
    require(summary['complete'] and summary['cleanup_complete'] and summary['inputs_unchanged'] and
            summary['workload_complete'] and not summary.get('failure') and not summary.get('closeout_failure'),
            'original runtime or cleanup failed')
    require(summary['throughput_acceptance'] is False, 'smoke claims performance acceptance')
    inventory = read(RAW/'inventory.json')
    for relative, entry in inventory.items():
        path = RAW/relative
        require(path.resolve().is_relative_to(RAW) and not path.is_symlink() and path.is_file() and
                path.stat().st_size == entry['bytes'] and sha(path) == entry['sha256'],
                'original retained artifact changed: '+relative)
    require(set(inventory) == {str(p.relative_to(RAW)) for p in RAW.rglob('*') if p.is_file()} - {'inventory.json'},
            'raw artifact inventory is not complete')
    bound = read(RAW/'bound-inputs.json')
    require(all(sha(path) == expected for path, expected in bound.items()), 'bound input changed')
    runner = load('reviewed_v3_smoke', PREP/'run.py')
    legacy = load('reviewed_legacy_smoke', runner.LEGACY_PATH)
    resp = load('reviewed_resp_smoke', runner.RESP_PATH)
    sys.path.insert(0, str(SOURCE/'scripts'))
    import batch_benchmark_report as native_validator
    import redis_batch_report as redis_validator
    require(subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=SOURCE, text=True).strip() == REV and
            not subprocess.check_output(['git', 'status', '--porcelain'], cwd=SOURCE, text=True), 'source revision/clean state differs')
    roles = summary['roles']
    for role, entry in roles.items():
        source = entry['source']
        require(all(sha(Path(source['path'])/name) == digest for name, digest in source['sources'].items()),
                'source inventory changed: '+role)
        build = Path(entry['build_directory'])
        if role == 'server':
            require(sha(build/'kv9') == runner.SERVER_SHA == entry['binary_sha256'], 'server binary differs')
            require(legacy.source_binding(Path(source['path']), read(build/'build.json'), runner.SERVER_REV) == source,
                    'server source binding differs')
            require(legacy.cargo_server(build/'kv9-cargo.jsonl') == entry['cargo'], 'server Cargo binding differs')
        else:
            _, manifest, features = native_validator.build_check(build, build, REV, reference=role == 'redis')
            require(manifest['binary_sha256'] == PINS[role] == entry['binary_sha256'] and features == [] and
                    manifest['profile'] == 'release' and not manifest['dirty'], 'client release differs')
            require(len(source['sources']) == 581 and source['path'] == str(SOURCE), 'client source inventory differs')
    require(roles['native']['source'] == roles['redis']['source'], 'clients do not share exact source')
    fixture = read(RAW/'fixture/fixture.json')
    require(fixture['complete'] and fixture['target'] == 'wal' and set(fixture['addresses']) == {'1', '2', '3'},
            'not a completed three-voter ordinary-WAL fixture')
    native_cleanup = read(RAW/'native-cleanup.json')['children']
    require(len(native_cleanup) == 9 and all(row['exit_code'] is not None and row['pid_absent']
            and not Path('/proc', str(row['pid'])).exists() for row in native_cleanup), 'native lifetime cleanup differs')
    expected = [(f'{api}-r{reads:03d}', target, api, batch, reads)
                for api, batch in [('point', 1), ('batch', 4)] for reads in [0, 50, 100]
                for target in ['native', 'redis']]
    require([(r['case'], r['target']) for r in summary['cases']] == [(r[0], r[1]) for r in expected], '12-case order differs')
    rows = []
    drain_count = 0
    lifetimes = {(row['pid'], row['identity']['start_ticks'], row['identity']['boot_id']) for row in native_cleanup}
    for case, target, api, batch, reads in expected:
        directory = RAW/case/target
        reference = target == 'redis'
        config = read(directory/'requested-config.json')
        shared = dict(run_id=case, seed=71, workers=4, keys=128, batch_size=batch, value_bytes=128,
                      read_percent=reads, warmup_calls=32, measure_ms=500, max_calls=100000,
                      load=dict(kind='closed_loop'))
        require(all(config[key] == value for key, value in shared.items()), 'declared workload differs')
        expected_apis = (['get', 'set'] if api == 'point' else ['mget', 'mset']) if reference else (
            ['point_get', 'point_put'] if api == 'point' else ['batch_get', 'batch_put'])
        require(config['version'] == 3 and [config['read_api'], config['write_api']] == expected_apis, 'v3 selector differs')
        if reference:
            require(config['deadline_ms'] == 1500, 'Redis deadline differs')
            require(redis_validator.paired_configuration(config, read(RAW/case/'native/requested-config.json')) ==
                    read(RAW/case/'paired-configuration.json'), 'independent API pairing differs')
        else:
            client = config['client']
            require(config['rpc_transport'] == 'tonic_stream' and client['deadline_ms'] == 1500 and
                    client['max_in_flight'] == 4 and client['max_attempts'] == 6 and client['retry_backoff_ms'] == 5 and
                    {str(p['node_id']): p['address'] for p in client['peers']} == fixture['addresses'], 'native client differs')
        validator = redis_validator if reference else native_validator
        checked = validator.validate(directory/'run', Path(roles[target]['build_directory']), directory/'requested-config.json', REV)
        require(checked == read(directory/'validation.json'), 'fresh strict report validation differs')
        report = read(directory/'run/report.json')
        populations = runner.check_populations(report, config, reference, resp)
        identity = read(directory/'client-identity.json')
        exited = read(directory/'client-exit.json')
        require(identity['pid'] == report['process_id'] == exited['pid'] and exited['exit_code'] == 0 and
                exited['pid_absent'] and not Path('/proc', str(identity['pid'])).exists() and
                checked['process_start_ticks'] == identity['start_ticks'], 'client report/lifetime differs')
        if reference:
            client_executable = Path(roles[target]['build_directory'])/'kv9-redis-batch-reference'
            executable_stat = client_executable.stat()
            require(identity['executable'] == str(client_executable) and
                    identity['executable_device'] == executable_stat.st_dev and
                    identity['executable_inode'] == executable_stat.st_ino, 'Redis client executable identity differs')
            dataset = {base64.b64decode(k, validate=True): base64.b64decode(v, validate=True)
                       for k, v in read(directory/'final-dataset.json').items()}
            cleanup = read(directory/'cleanup.json')
            require(cleanup['errors'] == [] and len(cleanup['children']) == 2, 'Redis cleanup incomplete')
            for child in cleanup['children']:
                ident = child['identity']
                require(child['exit_code'] is not None and child['pid_absent_after_reap'] and
                        not Path('/proc', str(ident['pid'])).exists(), 'Redis lifetime remains')
                lifetimes.add((ident['pid'], ident['start_ticks'], ident['boot_id']))
            before, after = [read(directory/(phase+'-server.json')) for phase in ['before', 'after']]
            for key in ['pid', 'start_ticks', 'boot_id', 'executable_device', 'executable_inode']:
                require(before['identity'][key] == after['identity'][key], 'Redis server identity changed')
            for row in [before, after]:
                require(row['settings'] == {'save': '', 'appendonly': 'no', 'io-threads': '1'} and
                        row['listener']['pid'] == row['identity']['pid'] and
                        row['listener']['port'] == int(config['address'].rsplit(':', 1)[1]), 'Redis listener/config differs')
                placement(row['threads'], list(range(8, 16)) + list(range(22, 32)))
            placement(read(directory/'client-threads.json'), [6, 7])
        else:
            require(identity['executable_sha256'] == PINS['native'] and
                    (identity['pid'], identity['start_ticks'], identity['boot_id']) in lifetimes,
                    'native client executable or cleanup identity differs')
            lines = (directory/'final-scan.txt').read_text().splitlines()
            pairs = [dict(part.split('=', 1) for part in line.split()) for line in lines[:-1]]
            dataset = {bytes.fromhex(p['key_hex']): bytes.fromhex(p['value_hex']) for p in pairs}
            require(lines[-1] == 'count=129' and len(pairs) == len(dataset) == 129 and
                    list(dataset) == sorted(dataset), 'native scan truncated/duplicated/unsorted')
            placement(identity['thread_placement'], [6, 7])
            for phase in ['before', 'after']:
                voters = read(directory/(phase+'-voters.json'))
                require(set(voters) == {'1', '2', '3'}, 'missing voter')
                for node, row in voters.items():
                    ident, state, listener = row['identity'], row['status'], row['listener']
                    require((ident['pid'], ident['start_ticks'], ident['boot_id']) in lifetimes and
                            str(ident['pid']) == state['pid'] and str(ident['start_ticks']) == state['process_start_ticks'] and
                            ident['boot_id'] == state['process_boot_id'] and listener['pid'] == ident['pid'] and
                            listener['advertised_endpoint'] == fixture['addresses'][node] and
                            listener['experimental_environment_absent'] and len(listener['listener_inodes']) == 1,
                            'voter status/listener lifetime differs')
                    placement(row['threads'], list(range(8, 16)) + list(range(22, 32)))
            for phase in ['before', 'post-client', 'post-readback']:
                drain = read(directory/(phase+'-fresh-drain.json'))
                require(0 <= drain['completed_unix_ns'] - drain['started_unix_ns'] <= 20000000000, 'drain exceeds freshness bound')
                require(set(drain['final']) == {'1', '2', '3'}, 'missing final drain voter')
                positions = set()
                for node, final in drain['final'].items():
                    baseline, first = drain['baseline'][node], drain['first_advances'][node]['status']
                    require(int(baseline['metrics_export_successes']) < int(first['metrics_export_successes']) <
                            int(final['metrics_export_successes']), 'drain exports not fresh')
                    for state in [first, final]:
                        require(all(state[k] == baseline[k] for k in ['pid', 'process_start_ticks', 'process_boot_id']) and
                                state['bootstrap_state'] == 'Serving' and state['fatal'] == '' and
                                all(state[k] == '0' for k in legacy.ZERO) and
                                state['raft_async_apply_stopped'] == state['raft_async_read_stopped'] == 'false' and
                                state['applied_term'] == state['driver_applied_term'] and
                                state['applied_index'] == state['driver_applied_index'], 'drain status not qualifying')
                    positions.add((final['applied_term'], final['applied_index']))
                require(len(positions) == 1, 'voters did not converge after drain')
                drain_count += 1
        data = runner.dataset_check(config, dataset, report, resp)
        original = read(directory/'result.json')
        require(data == original['dataset'] and populations == original['populations'] and original['complete'], 'retained checks differ')
        rows.append(dict(case=case, target=target, report_sha256=sha(directory/'run/report.json'),
                         populations=populations, dataset=data, measured_issued=report['measured_issued'],
                         measured_completed=report['measured_completed'], stop_reason=report['stop_reason']))
    require(len(lifetimes) == 21 and drain_count == 18, 'owned lifetimes/drain inventory differs')
    require(all(sha(RAW/relative) == entry['sha256'] for relative, entry in inventory.items()) and
            all(sha(path) == expected for path, expected in bound.items()), 'inputs changed during readback')
    result.update(accepted=True, cases=rows, source_revision=REV, source_files_per_client=581,
                  owned_lifetimes_exited=len(lifetimes), fresh_drain_stages=drain_count,
                  voter_drain_bindings=drain_count*3, retained_files=len(inventory),
                  retained_bytes=sum(row['bytes'] for row in inventory.values()),
                  raw_summary_sha256=sha(RAW/'summary.json'), raw_inventory_sha256=sha(RAW/'inventory.json'),
                  bound_inputs_sha256=sha(RAW/'bound-inputs.json'), runner_sha256=args.runner_sha256,
                  scope='Aggregate v3 API/accounting correctness and final deterministic data/sentinel, source/lifecycle/cleanup readback. No issued nonce ledger, per-call history, linearizability, Chaos, durability or performance acceptance.')


def main():
    parser = argparse.ArgumentParser(allow_abbrev=False)
    parser.add_argument('--root-session', type=int, required=True)
    parser.add_argument('--root-exit-code', type=int, required=True)
    parser.add_argument('--runner-sha256', required=True)
    args = parser.parse_args()
    require(sorted(os.sched_getaffinity(0)) == list(range(6, 16)) + list(range(22, 32)), 'wrong CPU mask')
    output = HERE/'results-first'
    output.mkdir(exist_ok=False)
    result = dict(accepted=False, root_session=args.root_session, root_exit_code=args.root_exit_code,
                  audit_pid=os.getpid(), started_ns=time.time_ns(), audit_sha256=sha(Path(__file__)))
    try:
        execute(args, output, result)
    except BaseException as error:
        result['failure'] = repr(error)
        raise
    finally:
        result['ended_ns'] = time.time_ns()
        save(output/'audit.json', result)
    print('PASS: 12 v3 aggregate correctness cases read back; no performance acceptance')


if __name__ == '__main__':
    main()
