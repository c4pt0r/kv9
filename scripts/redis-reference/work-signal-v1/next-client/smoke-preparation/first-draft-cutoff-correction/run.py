#!/usr/bin/env python3
"""Twelve source-bound v3 client correctness cases. Never a timing acceptance."""
import argparse
import base64
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import time

if not __debug__:
    raise RuntimeError("assertions must remain enabled")
sys.dont_write_bytecode = True

HERE = Path(__file__).resolve().parent
SERVER_SOURCE = Path('/tmp/kv9-rpc-worker-pair')
SERVER_BUILD = Path('/tmp/kv9-rpc-pair-release-first')
SERVER_REV = '5ee897a2f58c57bdf17ea1757c96224adf0f0dbb'
SERVER_SHA = '0d5ffa081482945d213b88aef12222afab44a44ed03bf46cf27bd292db7b1711'
SERVER_MANIFEST = '1bc2c98587f344f0066033450018d0a4c81b4f16a5c25a691b18136659ebe417'
CLIENT_SOURCE = Path('/tmp/kv9-point-write-measurement-v3')
COMMON = 'crates/server/src/bin/kv9-batch-benchmark/common.rs'
COMMON_SHA = 'b98d4a49f9ee2199e183204b72fd52813907574a03c1cf3376cf5d281d7902fd'
LEGACY_PATH = Path('/tmp/kv9-stream-worker-pair-comparison-preparation/matched-driver.py')
LEGACY_SHA = '85ea6f8d886d542608571073e7a27869f24ce00433dc90970d27316635d36709'
RESP_PATH = Path('/tmp/kv9-redis-batch-reference-preparation/run_fixture.py')
RESP_SHA = '008c5bea0640a4a5a092feb6617798a84e9dd4889fd11bf8c6791cfe0852864a'
REDIS_SERVER = Path('/usr/bin/redis-server')  # Keep argv[0], including this symlink name.
REDIS_SERVER_SHA = '1b2950213684d355e0e49fe6702c7bfb9d40b78f0da9ce289fc0e8f2a295e540'
CPUS = list(range(6, 16)) + list(range(22, 32))


def require(ok, message):
    if not ok:
        raise ValueError(message)


def sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def read(path):
    return json.loads(Path(path).read_text())


def save(path, value):
    Path(path).write_text(json.dumps(value, indent=2, sort_keys=True) + '\n')


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    sys.modules[name] = result
    spec.loader.exec_module(result)
    return result


def cases():
    return [dict(name=f'{api}-r{reads:03d}', api=api, batch_size=batch, read_percent=reads)
            for api, batch in (('point', 1), ('batch', 4)) for reads in (0, 50, 100)]


def configs(case, peers, keyspace, address):
    shared = dict(run_id=case['name'], seed=71, workers=4, keys=128,
                  batch_size=case['batch_size'], value_bytes=128, read_percent=case['read_percent'],
                  warmup_calls=32, measure_ms=500, max_calls=100_000, load=dict(kind='closed_loop'))
    point = case['api'] == 'point'
    native = dict(version=3, read_api='point_get' if point else 'batch_get',
                  write_api='point_put' if point else 'batch_put', rpc_transport='tonic_stream', **shared,
                  client=dict(version=1, peers=peers, keyspace_id=keyspace, epoch_conf_ver=1,
                              epoch_version=1, max_in_flight=4, max_attempts=6,
                              deadline_ms=1500, retry_backoff_ms=5))
    redis = dict(version=3, read_api='get' if point else 'mget', write_api='set' if point else 'mset',
                 address=address, deadline_ms=1500, **shared)
    return native, redis


def check_populations(report, config, reference, resp):
    """Independent closed-loop mix and complete, one-attempt outcome accounting."""
    result = {}
    for phase, metrics in report['metrics'].items():
        traffic = phase in ('warmup', 'measurement')
        labels = ([config['read_api'].removeprefix('point_'), config['write_api'].removeprefix('point_')] if traffic else
                  ['mget', 'mset'] if reference else ['batch_get', 'batch_put'])
        require(metrics['operations'] == labels, 'actual operation labels differ from the selected APIs')
        counts = []
        for op in metrics['statistics']:
            counts.append(op['populations'][0]['calls'])
            require(all(p['calls'] == 0 for p in op['populations'][1:]), 'non-success outcome in healthy smoke')
            require(op['reasons'][0] == counts[-1] and sum(op['reasons'][1:]) == 0,
                    'non-success reason in healthy smoke')
            if reference:
                require(op['command_attempts'] == counts[-1] and op['connection_failures'] == 0,
                        'Redis command attempts are not one per successful call')
            else:
                require([h['raw']['count'] for h in op['attempts']] == [counts[-1], 0, 0],
                        'native call has a refused, unknown, missing or repeated SDK attempt')
        if traffic:
            first = 1 if phase == 'warmup' else config['warmup_calls'] + 1
            count = config['warmup_calls'] if phase == 'warmup' else report['measured_issued']
            reads = sum(resp.mix(config['seed'] ^ n) % 100 < config['read_percent']
                        for n in range(first, first + count))
            require(counts == [reads, count - reads], 'deterministic issued nonce/mix population differs')
            if phase == 'measurement':
                require(count > 0 and all(counts[i] > 0 for i in range(2)
                        if (i == 0 and config['read_percent'] > 0) or
                           (i == 1 and config['read_percent'] < 100)), 'selected API has no measured success')
        result[phase] = dict(operations=labels, success_calls=counts, other_outcomes=0,
                             attempts=sum(counts), input_items=[op['populations'][0]['input_items']
                                                               for op in metrics['statistics']])
    return result


def dataset_check(config, dataset, report, resp):
    """Value validity/membership only; concurrent whole histories are not recorded."""
    require(len(dataset) == config['keys'] + 1, 'final dataset count differs')
    result = resp.valid_dataset(config, dataset)
    limit = config['warmup_calls'] + report['measured_issued']
    changed = 0
    for index in range(config['keys']):
        value = dataset[f'{config["run_id"]}:{index:016x}'.encode()]
        nonce = int.from_bytes(value[8:16], 'big')
        if nonce:
            require(nonce <= limit and resp.mix(config['seed'] ^ nonce) % 100 >= config['read_percent'],
                    'final value refers to an unissued or read-only nonce')
            first = resp.mix(config['seed'] ^ nonce ^ 0xc6275c213842315b) % config['keys']
            require((index - first) % config['keys'] < config['batch_size'],
                    'final value nonce never wrote this key')
            changed += 1
    require(changed > 0 if config['read_percent'] < 100 else changed == 0,
            'final writes absent or a read-only workload changed data')
    return dict(result, changed_keys=changed, maximum_issued_nonce=limit,
                issued_write_key_membership_checked=True, full_linearizability_checked=False)


def native_scan(fixture, config, directory):
    start, end = config['run_id'].encode() + b':', config['run_id'].encode() + b';'
    leader = fixture.wait('leader for final scan', fixture.leader)
    text = fixture.command([SERVER_BUILD/'kv9', 'client', 'raw-scan', '--addr', fixture.addresses[leader],
        '--keyspace', config['client']['keyspace_id'], '--start-hex', start.hex(), '--end-hex', end.hex(), '--limit', '256'])
    (directory/'final-scan.txt').write_text(text + '\n')
    lines, pairs = text.splitlines(), []
    require(lines and lines[-1].startswith('count='), 'independent scan count missing')
    for line in lines[:-1]:
        fields = dict(part.split('=', 1) for part in line.split())
        require(set(fields) == {'key_hex', 'value_hex'}, 'unexpected scan fields')
        pairs.append((bytes.fromhex(fields['key_hex']), bytes.fromhex(fields['value_hex'])))
    require(len(pairs) == int(lines[-1][6:]) == config['keys'] + 1 < 256, 'final scan missing items/truncated')
    require([k for k, _ in pairs] == sorted(k for k, _ in pairs) and len(dict(pairs)) == len(pairs)
            and all(start <= k < end for k, _ in pairs), 'scan order, duplicate or range failure')
    return dict(pairs)


def finish_client(child, directory, config, build, revision, validator, identity, reference, resp):
    code = child.wait(timeout=60)
    save(directory/'client-exit.json', dict(pid=child.pid, exit_code=code, reaped_unix_ns=time.time_ns(),
                                          pid_absent=not Path('/proc', str(child.pid)).exists()))
    require(code == 0 and not Path('/proc', str(child.pid)).exists(), 'client failed or was not reaped')
    validated = validator.validate(directory/'run', build, directory/'requested-config.json', revision,
                                   require_timing=False)
    save(directory/'validation.json', validated)
    report = read(directory/'run/report.json')
    require(report['version'] == config['version'] == 3, 'legacy report/config substituted')
    require(validated['process_start_ticks'] == identity['start_ticks'] and report['process_id'] == child.pid
            and read(directory/'run/ready.json')['process_id'] == child.pid, 'client process/report binding differs')
    # Either honest stop is retained. No success rate, cutoff or timing flag is rewritten.
    require(report['measured_completed'] == report['measured_issued'], 'unfinished issued client calls')
    counts = check_populations(report, config, reference, resp)
    return report, dict(complete=True, pid=child.pid, exit_code=code, report_sha256=sha(directory/'run/report.json'),
                       validation_sha256=sha(directory/'validation.json'), populations=counts,
                       stop_reason=report['stop_reason'], client_timing_eligible=report['timing_eligible'],
                       throughput_acceptance=False)


def bind_inputs(args, legacy, validator):
    roles = {}
    manifest = read(SERVER_BUILD/'build.json')
    require(sha(SERVER_BUILD/'build.json') == SERVER_MANIFEST and
            sha(SERVER_BUILD/'kv9') == manifest['binaries']['kv9']['sha256'] == SERVER_SHA,
            'accepted control server manifest or executable changed')
    roles['server'] = dict(source=legacy.source_binding(SERVER_SOURCE, manifest, SERVER_REV),
        cargo=legacy.cargo_server(SERVER_BUILD/'kv9-cargo.jsonl'), binary_sha256=SERVER_SHA,
        build_directory=str(SERVER_BUILD))
    for name, build, manifest_sha, binary_sha, reference in (
        ('native', args.native_build, args.native_manifest_sha256, args.native_binary_sha256, False),
        ('redis', args.redis_build, args.redis_manifest_sha256, args.redis_binary_sha256, True)):
        _, declaration, features = validator.build_check(build, build, args.client_revision, reference=reference)
        require(sha(build/'build.json') == manifest_sha and declaration['binary_sha256'] == binary_sha
                and features == [] and declaration['profile'] == 'release', 'client release pins differ')
        inventory = read(build/'sources.json')
        source = legacy.source_binding(CLIENT_SOURCE, dict(declaration, sources=inventory['sources']), args.client_revision)
        require(source['sources'][COMMON] == COMMON_SHA, 'shared deterministic source changed')
        roles[name] = dict(source=source, binary_sha256=binary_sha, build_directory=str(build), features=features)
    require(roles['native']['source'] == roles['redis']['source'], 'native and Redis source inventories differ')
    require(sha(REDIS_SERVER) == REDIS_SERVER_SHA, 'Redis server executable changed')
    return roles


def main():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument('--client-revision', required=True)
    for role in ('native', 'redis'):
        parser.add_argument('--'+role+'-build', type=Path, required=True)
        parser.add_argument('--'+role+'-manifest-sha256', required=True)
        parser.add_argument('--'+role+'-binary-sha256', required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    require(sorted(os.sched_getaffinity(0)) == CPUS, 'runner must use the declared background CPU mask')
    require(os.environ.get('PYTHONOPTIMIZE') == '0' and os.environ.get('PYTHONDONTWRITEBYTECODE') == '1',
            'required Python environment differs')
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    summary = dict(complete=False, throughput_acceptance=False, protocol_id='kv9-point-write-v3-smoke-v1',
                   cases=[], runner_pid=os.getpid(), command=sys.argv, started_unix_ns=time.time_ns(),
                   scope='Twelve v3 API/accounting correctness cases; one-host WAL trio and standalone Redis. '
                         'Full final value validity, not per-call history, linearizability, Chaos or QPS acceptance.')
    fixture = None
    bound = {}
    def interrupted(signum, _frame):
        raise RuntimeError('smoke interrupted by signal '+str(signum))
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    try:
        require(sha(LEGACY_PATH) == LEGACY_SHA and sha(RESP_PATH) == RESP_SHA, 'retained helper changed')
        legacy, resp = module('retained_smoke_helpers', LEGACY_PATH), module('retained_resp_helpers', RESP_PATH)
        sys.path.insert(0, str(CLIENT_SOURCE/'scripts'))
        import batch_benchmark_report as native_validator
        import redis_batch_report as redis_validator
        support = module('native_process_support', CLIENT_SOURCE/'scripts/native-batch-e2e.py')
        summary['roles'] = bind_inputs(args, legacy, native_validator)
        inputs = [Path(__file__), HERE/'protocol.json', LEGACY_PATH, RESP_PATH, REDIS_SERVER,
                  SERVER_BUILD/'build.json', SERVER_BUILD/'kv9', SERVER_BUILD/'kv9-cargo.jsonl']
        for name, binary in (('native', 'kv9-batch-benchmark'), ('redis', 'kv9-redis-batch-reference')):
            build = getattr(args, name+'_build')
            inputs.extend(build/p for p in ('build.json', 'sources.json', 'cargo.jsonl', binary))
        helper_names = ['native-batch-e2e.py', 'benchmark.py', 'batch_benchmark_report.py', 'redis_batch_report.py',
                        'batch_workload_report.py', 'workload_report.py', 'root_provision.py', 'wal_layout.py', 'history/checker.py']
        for name in helper_names:
            path = CLIENT_SOURCE/'scripts'/name
            require(sha(path) == summary['roles']['native']['source']['sources']['scripts/'+name],
                    'executing source helper is not build-bound: '+name)
            inputs.append(path)
        bound = {str(p.resolve()): sha(p) for p in inputs}
        save(out/'bound-inputs.json', bound)
        protocol = read(HERE/'protocol.json')
        require(protocol['cases'] == cases() and protocol['protocol_id'] == summary['protocol_id']
                and protocol['throughput_acceptance'] is False, 'protocol case/scope inventory differs')
        save(out/'protocol.json', protocol)
        place = legacy.placement(True)
        class Owned(support.StreamingFixture):
            def __init__(self):
                super().__init__(out/'fixture', SERVER_BUILD)
                self.placement.update(place)
                self.record['placement'] = dict(place)
            def launch(self, command, logfile, client=False):
                child = super().launch(command, logfile, client)
                identity = self.identities[child.pid]
                expected = args.native_binary_sha256 if client else SERVER_SHA
                require(identity['executable_sha256'] == expected, 'actual native role executable differs')
                identity['boot_id'] = Path('/proc/sys/kernel/random/boot_id').read_text().strip()
                identity['thread_placement'] = legacy.affinity(child.pid, place['client_cpus' if client else 'server_cpus'])
                return child
        fixture = Owned()
        fixture.start()
        def voters(directory, phase):
            rows = {}
            for n, child in fixture.nodes.items():
                state = fixture.state(n)
                identity = resp.serial_identity(resp.process_identity(child.pid))
                require(state and state['process_start_ticks'] == str(identity['start_ticks']) and
                        state['process_boot_id'] == identity['boot_id'] == fixture.identities[child.pid]['boot_id']
                        and identity['start_ticks'] == fixture.identities[child.pid]['start_ticks'] and
                        sha(Path('/proc', str(child.pid), 'exe')) == SERVER_SHA, 'voter lifetime changed')
                require(state.get('public_rpc_limit_requests') == '64' and
                        state.get('public_rpc_limit_encoded_bytes') == '16777216', 'fixture public capacity differs')
                rows[str(n)] = dict(identity=identity, status=state, listener=fixture.listener_evidence(n),
                    threads=legacy.affinity(child.pid, place['server_cpus']))
            save(directory/(phase+'-voters.json'), rows)
            fixture.snapshot(directory, phase)
        for case in cases():
            directory = out/case['name']
            directory.mkdir()
            native_dir, redis_dir = directory/'native', directory/'redis'
            native_dir.mkdir(); redis_dir.mkdir()
            leader = fixture.wait('leader for fresh smoke keyspace', fixture.leader)
            receipt = fixture.command([SERVER_BUILD/'kv9', 'client', 'create-keyspace', '--addr', fixture.addresses[leader],
                                       '--name', case['name'], '--api-type', 'raw'])
            keyspace = int(dict(line.split('=', 1) for line in receipt.splitlines())['keyspace_id'])
            # Initial routing is explicit: retain all three peers, with the current leader first.
            peers = [dict(node_id=n, address=fixture.addresses[n]) for n in
                     [leader, *[n for n in fixture.nodes if n != leader]]]
            native, _ = configs(case, peers, keyspace, '127.0.0.1:6379')
            save(native_dir/'requested-config.json', native)
            legacy.fresh_drain(fixture, native_dir, 'before')
            voters(native_dir, 'before')
            row = dict(case=case['name'], target='native', complete=False)
            summary['cases'].append(row); save(out/'summary.json', summary)
            child = fixture.launch([args.native_build/'kv9-batch-benchmark', '--config', native_dir/'requested-config.json',
                '--build-manifest', args.native_build/'build.json', '--output', native_dir/'run'], native_dir/'client.log', client=True)
            fixture.workloads.append(child)
            save(native_dir/'client-identity.json', fixture.identities[child.pid])
            report, checked = finish_client(child, native_dir, native, args.native_build, args.client_revision,
                native_validator, fixture.identities[child.pid], False, resp)
            row.update(checked, complete=False)
            legacy.fresh_drain(fixture, native_dir, 'post-client')
            row['dataset'] = dataset_check(native, native_scan(fixture, native, native_dir), report, resp)
            legacy.fresh_drain(fixture, native_dir, 'post-readback')
            voters(native_dir, 'after')
            row['complete'] = True; save(native_dir/'result.json', row); save(out/'summary.json', summary)
            print('PASS native '+case['name'], flush=True)
            redis_row = dict(case=case['name'], target='redis', complete=False, cleanup_complete=False)
            summary['cases'].append(redis_row); save(out/'summary.json', summary)
            class Placed(resp.Children):
                def launch(self, label, command, executable):
                    expected = args.redis_binary_sha256 if label == 'client' else REDIS_SERVER_SHA
                    original = os.sched_getaffinity(0)
                    mask = place['client_cpus' if label == 'client' else 'server_cpus']
                    try:
                        os.sched_setaffinity(0, mask)
                        p = super().launch(label, command, executable)
                    finally:
                        os.sched_setaffinity(0, original)
                    require(sha(Path('/proc', str(p.pid), 'exe')) == expected, 'actual Redis role executable differs')
                    save(redis_dir/(label+'-threads.json'), legacy.affinity(p.pid, mask))
                    return p
            children = Placed(redis_dir)
            reservation = socket.socket()
            try:
                reservation.bind(('127.0.0.1', 0)); port = reservation.getsockname()[1]
                _, reference = configs(case, peers, keyspace, f'127.0.0.1:{port}')
                save(redis_dir/'requested-config.json', reference)
                save(directory/'paired-configuration.json', redis_validator.paired_configuration(reference, native))
                (redis_dir/'redis.conf').write_text(f'bind 127.0.0.1\nport {port}\nprotected-mode yes\nsave ""\n'
                    f'appendonly no\ndir {redis_dir}\ndaemonize no\nlogfile ""\nio-threads 1\nmaxclients 512\n')
                reservation.close()
                server = children.launch('redis', [REDIS_SERVER, redis_dir/'redis.conf'], REDIS_SERVER)
                for _ in range(150):
                    require(server.poll() is None, 'Redis exited during readiness')
                    try:
                        if resp.request(reference['address'], [b'PING']) == b'PONG':
                            break
                    except OSError:
                        pass
                    time.sleep(.02)
                else:
                    raise TimeoutError('owned Redis readiness timed out')
                def redis_state(phase):
                    identity = resp.process_identity(server.pid)
                    initial = children.rows[0]['identity']
                    require(all(identity[k] == initial[k] for k in ('pid', 'start_ticks', 'boot_id',
                            'executable_device', 'executable_inode')), 'Redis lifetime changed')
                    values = resp.request(reference['address'], [b'CONFIG', b'GET', b'save', b'appendonly', b'io-threads'])
                    settings = {values[i].decode(): values[i+1].decode() for i in range(0, len(values), 2)}
                    replication = resp.request(reference['address'], [b'INFO', b'replication']).decode()
                    require(settings == dict(save='', appendonly='no', **{'io-threads': '1'}) and
                            'role:master\r\n' in replication and 'connected_slaves:0\r\n' in replication,
                            'owned Redis configuration differs')
                    save(redis_dir/(phase+'-server.json'), dict(identity=resp.serial_identity(identity),
                        listener=resp.listener_identity(server.pid, port), settings=settings, replication=replication,
                        threads=legacy.affinity(server.pid, place['server_cpus'])))
                redis_state('before')
                child = children.launch('client', [args.redis_build/'kv9-redis-batch-reference', '--config',
                    redis_dir/'requested-config.json', '--build-manifest', args.redis_build/'build.json',
                    '--output', redis_dir/'run'], args.redis_build/'kv9-redis-batch-reference')
                report, checked = finish_client(child, redis_dir, reference, args.redis_build, args.client_revision,
                    redis_validator, children.rows[-1]['identity'], True, resp)
                redis_row.update(checked, complete=False)
                dataset = {}
                for first in range(0, reference['keys'] + 1, 128):
                    keys = [f'{reference["run_id"]}:{i:016x}'.encode() for i in range(first, min(reference['keys']+1, first+128))]
                    values = resp.request(reference['address'], [b'MGET', *keys])
                    require(isinstance(values, list) and len(values) == len(keys), 'final MGET cardinality differs')
                    dataset.update(zip(keys, values))
                save(redis_dir/'final-dataset.json', {base64.b64encode(k).decode(): base64.b64encode(v).decode()
                     if v is not None else None for k, v in dataset.items()})
                redis_row['dataset'] = dataset_check(reference, dataset, report, resp)
                redis_state('after')
            finally:
                reservation.close()
                redis_row['cleanup_errors'] = children.cleanup()
                redis_row['cleanup_complete'] = not redis_row['cleanup_errors']
                save(redis_dir/'result.json', redis_row)
            require(not redis_row['cleanup_errors'], 'owned Redis cleanup failed')
            redis_row['complete'] = True; save(redis_dir/'result.json', redis_row); save(out/'summary.json', summary)
            print('PASS redis '+case['name'], flush=True)
        fixture.finish()
        require(len(summary['cases']) == 12 and all(row['complete'] for row in summary['cases']), 'case inventory incomplete')
        summary['workload_complete'] = True
    except BaseException as error:
        summary['failure'] = repr(error)
    finally:
        signal.signal(signal.SIGTERM, signal.SIG_IGN)
        signal.signal(signal.SIGINT, signal.SIG_IGN)
        summary['cleanup_complete'] = False
        try:
            if fixture is not None:
                fixture.close()
                lifetimes = [dict(pid=p.pid, identity=fixture.identities.get(p.pid), exit_code=p.poll(),
                                  pid_absent=not Path('/proc', str(p.pid)).exists()) for p in fixture.children]
                save(out/'native-cleanup.json', dict(children=lifetimes))
                require(all(row['exit_code'] is not None and row['pid_absent'] for row in lifetimes), 'native owned child remains')
            require(all(row.get('cleanup_complete') is True for row in summary['cases'] if row['target'] == 'redis'),
                    'Redis cleanup errors or incomplete cleanup retained')
            summary['cleanup_complete'] = True
            require(bound and all(sha(path) == value for path, value in bound.items()), 'frozen input bytes changed')
            # Recheck the whole original source inventories, not just executed helper files.
            if 'roles' in summary:
                for role in summary['roles'].values():
                    source = role['source']
                    require(all(sha(Path(source['path'])/name) == value for name, value in source['sources'].items()),
                            'source changed during smoke')
            summary['inputs_unchanged'] = True
        except BaseException as error:
            summary['closeout_failure'] = repr(error)
        summary['complete'] = (summary.get('workload_complete', False) and summary['cleanup_complete']
                               and summary.get('inputs_unchanged', False) and 'failure' not in summary)
        summary['completed_unix_ns'] = time.time_ns()
        save(out/'summary.json', summary)
        files = {str(p.relative_to(out)): dict(bytes=p.stat().st_size, sha256=sha(p))
                 for p in sorted(out.rglob('*')) if p.is_file()}
        save(out/'inventory.json', files)
    return 0 if summary['complete'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
