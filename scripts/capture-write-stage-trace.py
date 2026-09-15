#!/usr/bin/env python3
"""Bounded observer diagnostic; not the ten-second candidate acceptance screen."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import time
from types import SimpleNamespace
sys.path.insert(0, '/mnt/data/kv9-work/write-stage-capture-preparation-20260915-first')
import health
import write_stage_capture

if not __debug__:
    raise RuntimeError('diagnostic requires Python assertions enabled')
sys.dont_write_bytecode = True
HERE = Path('/mnt/data/kv9-work/write-stage-capture-preparation-20260915-first')
REVISION = '8c0008569d50e51a39eb04ff2ebb6cde1d00ba13'
CLIENT_REVISION = '0be806d9671e2c50701a64aa7889c8859b7648ba'
CLIENT_SHA = '1b8060eb168610328c10a27480de16bd8b2d6805166d8169638f85ceef0992c4'
GIB = 1024 ** 3
POLICY = dict(id='kv9-write-stage-loaded-batch-capture-v1', host_floor_bytes=8*GIB,
              max_campaign_decrease_bytes=13*GIB, max_cohort_payload_bytes=3*GIB,
              preflight_tmpfs_bytes=32*GIB, runtime_tmpfs_bytes=16*GIB,
              max_cohorts=4, measurement_ms=2000)


def require(ok, message):
    if not ok:
        raise ValueError(message)


def digest(path):
    with Path(path).open('rb') as f:
        return hashlib.file_digest(f, 'sha256').hexdigest()


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def available(path):
    s = os.statvfs(path)
    return s.f_bavail * s.f_frsize


def check_space(current, baseline, forthcoming=0):
    require(type(current) is int and type(baseline) is int and type(forthcoming) is int,
            'space inputs must be integers')
    require(forthcoming >= 0, 'negative prospective copy')
    require(current - forthcoming >= POLICY['host_floor_bytes'], 'diagnostic host floor')
    require(baseline - current + forthcoming <= POLICY['max_campaign_decrease_bytes'],
            'diagnostic campaign space decrease')


def plan_rows(driver):
    rows = [r for r in driver.make_plan(False)
            if r['write_api'] == 'batch_put' and r['shared']['workers'] == 64]
    for ordinal, row in enumerate(rows):
        row['ordinal'] = ordinal
        row['mode'] = {'old': 'default', 'new': 'instrumented'}[row['server_role']]
        row['shared']['measure_ms'] = POLICY['measurement_ms']
    require(len(rows) == POLICY['max_cohorts'] and
            [r['server_role'] for r in rows] == ['old', 'new', 'new', 'old'], 'focused row order')
    return rows


def server_binding(driver, source, build, instrumented):
    manifest = driver.read(build / 'build.json')
    bound = driver.source_binding(source, manifest, REVISION)
    binary = manifest['binaries']['kv9']
    expected_pin = json.loads((HERE/'release-pins.json').read_text())['instrumented' if instrumented else 'default']
    require(digest(build/'build.json') == expected_pin['manifest_sha256'] and
            binary['sha256'] == expected_pin['binary_sha256'], 'release authority differs')
    require(digest(build / 'kv9') == binary['sha256'], 'server binary hash differs')
    records = [json.loads(line) for line in (build / 'kv9-cargo.jsonl').read_text().splitlines()]
    require(records[-1] == {'reason': 'build-finished', 'success': True}, 'Cargo completion missing')
    selected = {}
    for name in ['kv9', 'kv9_engine', 'kv9_raft', 'kv9_server']:
        candidates = [r for r in records if r.get('reason') == 'compiler-artifact'
                      and r.get('target', {}).get('name') == name]
        expected = ['write-path-diagnostics', 'write-stage-tracing'] if instrumented and name != 'kv9_engine' else []
        require(len(candidates) == 1, 'ambiguous Cargo unit: ' + name)
        unit = candidates[0]
        require(unit['features'] == expected, 'unexpected features: ' + name)
        require(unit['profile']['opt_level'] == '3' and unit['profile']['test'] is False,
                'server is not a release executable')
        selected[name] = unit
    command = binary['command']
    require('--release' in command and '--bin' in command and command[command.index('--bin')+1] == 'kv9',
            'wrong server build command')
    require(('--features' in command) == instrumented, 'feature command differs')
    if instrumented:
        require(command[command.index('--features')+1] == 'write-path-diagnostics,write-stage-tracing', 'wrong feature selection')
    return dict(build_directory=str(build), binary_sha256=binary['sha256'], source=bound,
                rustc=manifest['rustc'], cargo_artifacts=selected,
                build_manifest_sha256=digest(build/'build.json'),
                cargo_sha256=digest(build/'kv9-cargo.jsonl'))


def main():
    parser = argparse.ArgumentParser(allow_abbrev=False)
    for name in ['output', 'isolation-snapshot', 'client-source', 'client-build',
                 'old-server-source', 'old-server-build', 'new-server-source', 'new-server-build']:
        parser.add_argument('--'+name, type=Path, required=True)
    parser.add_argument('--expected-client-revision', required=True)
    parser.add_argument('--storage', choices=['tmpfs'], required=True)
    parser.add_argument('--resp-helper', type=Path,
                        default=Path('/mnt/data/kv9-work/performance-input-recovery-20260915-first/run_fixture.py'))
    args = parser.parse_args()
    for key, value in vars(args).items():
        if isinstance(value, Path):
            setattr(args, key, value.resolve())
    require(not args.output.exists(), 'preserve original output; use a fresh directory')
    args.output.mkdir()
    require(args.output.is_relative_to(Path('/mnt/data/kv9-work')), 'output must remain on data volume')
    baseline = available(args.output)
    root_device = Path('/').stat().st_dev
    retention_device = args.output.stat().st_dev
    require(available('/') >= 24*GIB, 'root filesystem launch floor')
    require(baseline >= POLICY['host_floor_bytes'] + POLICY['max_campaign_decrease_bytes'] + 8*1024**2,
            'insufficient prospective diagnostic capacity')
    require(available('/dev/shm') >= POLICY['preflight_tmpfs_bytes'], 'insufficient tmpfs capacity')
    pins = json.loads((HERE/'inherited-inputs.json').read_text())
    for name, pin in pins.items():
        require(digest(HERE/name) == pin['sha256'], 'inherited helper changed')
    driver = load('observer_inherited_driver', HERE/'inherited-driver.py')
    require(digest(args.resp_helper) == driver.RESP_HELPER_SHA, 'RESP helper differs')
    require(args.expected_client_revision == CLIENT_REVISION, 'wrong client revision')
    cm = driver.read(args.client_build/'build.json')
    ci = driver.read(args.client_build/'sources.json')
    require(cm['binary_sha256'] == digest(args.client_build/'kv9-batch-benchmark') == CLIENT_SHA,
            'fixed client differs')
    require(all(cm[k] == ci[k] for k in ['revision','dirty','source_tree_sha256','binary_sha256']),
            'client source/build identity differs')
    client = driver.source_binding(args.client_source, dict(cm, sources=ci['sources']), CLIENT_REVISION)
    roles = {
        'old': server_binding(driver, args.old_server_source, args.old_server_build, False),
        'new': server_binding(driver, args.new_server_source, args.new_server_build, True),
        'client': dict(build_directory=str(args.client_build), binary_sha256=CLIENT_SHA, source=client,
                       rustc=cm['rustc']),
    }
    require(roles['old']['source']['sources'] == roles['new']['source']['sources'], 'server sources differ')
    require(len({v['rustc'] for v in roles.values()}) == 1, 'compiler identities differ')
    isolation = driver.read(args.isolation_snapshot)
    require(isolation['complete'] and isolation['driver_sha256'] == digest(__file__), 'isolation binding differs')
    require(isolation['measured_client_cpus'] == [0,1] and isolation['measured_server_cpus'] == [2,3,4,5],
            'isolation placement differs')
    sys.path.insert(0, str(args.client_source/'scripts'))
    import benchmark
    import batch_benchmark_report as native_validator
    support = load('observer_support', args.client_source/'scripts/native-batch-e2e.py')
    tmpfs = load('observer_tmpfs', args.client_source/'scripts/tmpfs-redis-diagnostic.py')
    comparison = load('observer_resource', args.client_source/'scripts/redis-comparison.py')
    redis_validator = load('observer_pairing', args.client_source/'scripts/redis_batch_report.py')
    resp = load('observer_dataset', args.resp_helper)
    modules = benchmark, support, tmpfs, comparison, resp, native_validator, redis_validator
    driver.verify_client_cargo(args, native_validator)
    helpers = {str(HERE/'inherited-driver.py'): digest(HERE/'inherited-driver.py'),
               str(args.resp_helper): digest(args.resp_helper), str(Path(__file__)): digest(__file__)}
    for imported in list(sys.modules.values()):
        name = getattr(imported, '__file__', None)
        if name and Path(name).resolve().is_relative_to(args.client_source):
            path = Path(name).resolve(); relative = str(path.relative_to(args.client_source))
            require(client['sources'].get(relative) == digest(path), 'imported fixed helper changed')
            helpers[str(path)] = digest(path)
    # The inherited trial function is reused for its process/placement, native
    # client arithmetic, fresh drains and dataset controls, not its old campaign
    # authority. This prospective uncompressed capture has its own explicit
    # bounds; storage-v3 promotion/restoration predicates remain unchanged there.
    driver.SPACE_GUARDS = {
        'preflight': {'tmpfs': POLICY['preflight_tmpfs_bytes'], 'retention': POLICY['host_floor_bytes'], 'root': 24*GIB},
        'runtime': {'tmpfs': POLICY['runtime_tmpfs_bytes'], 'retention': POLICY['host_floor_bytes'], 'root': 24*GIB},
    }
    original_observe = driver.storage_observation
    def observe(directory):
        check_space(available(args.output), baseline)
        observation = original_observe(directory)
        require(observation['filesystems']['root']['device'] == root_device and
                observation['filesystems']['retention']['device'] == retention_device, 'filesystem device changed')
        return observation
    driver.storage_observation = observe
    def retain(fixture, close, campaign):
        close(fixture)
        require(all(p.poll() is not None for p in fixture.children), 'live writer during retention')
        original = tmpfs.files(fixture.scratch)
        size = sum(entry['bytes'] for entry in original.values())
        require(size <= POLICY['max_cohort_payload_bytes'], 'diagnostic cohort payload cap')
        check_space(available(args.output), baseline, size)
        driver.require_storage(observe(args.output), 'runtime')
        # Existing exact-copy/full-readback/scratch-removal implementation. Its
        # second owned close is idempotent; no live process is restarted.
        tmpfs.TmpfsFixture.close(fixture)
        check_space(available(args.output), baseline)
        driver.require_storage(observe(args.output), 'runtime')
        retained = driver.read(fixture.out/'tmpfs-retention.json')
        require(retained['files'] == original and retained['complete'], 'retained data identity differs')
        driver.save(fixture.out/'diagnostic-retention-budget.json',
                    dict(bytes=size, original_available=baseline, available_after=available(args.output),
                         policy=POLICY, full_readback=True))
    args.retention_helper = SimpleNamespace(close_fixture=retain)
    args.smoke = False
    rows = plan_rows(driver)
    helpers[str(HERE/'write_stage_capture.py')] = digest(HERE/'write_stage_capture.py')
    helpers[str(HERE/'check-write-stage-trace.py')] = digest(HERE/'check-write-stage-trace.py')
    helpers[str(HERE/'health.py')] = digest(HERE/'health.py')
    record = dict(schema_version=1, protocol_id='kv9-write-stage-loaded-batch-capture-v1', complete=False,
                  scope='Four two-second loaded Batch64/c64 cohorts; current-source combined trace+diagnostics overhead, no historical comparison or promotion',
                  no_new_baseline=True, source_revision=REVISION, policy=POLICY,
                  source_bindings=roles, helper_hashes=helpers, isolation=isolation,
                  available_before=baseline, root_device=root_device, retention_device=retention_device,
                  root_floor_bytes=24*GIB, raw_capture=write_stage_capture.POLICY, rows=rows, attempts=[])
    driver.save(args.output/'plan.json', record)
    def interrupted(signum, _frame):
        raise RuntimeError('diagnostic interrupted: '+str(signum))
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    try:
        for row in rows:
            check_space(available(args.output), baseline)
            driver.require_storage(observe(args.output), 'preflight')
            directory = args.output/f'{row["ordinal"]:02d}-{row["mode"]}-{row["workload"]}-c{row["shared"]["workers"]}'
            directory.mkdir()
            print(json.dumps(dict(starting=row['ordinal'], mode=row['mode'], directory=str(directory))), flush=True)
            with write_stage_capture.Hooks(driver, benchmark, directory, row['mode']=='instrumented',
                    lambda: driver.require_storage(observe(args.output), 'runtime'), HERE/'check-write-stage-trace.py'):
                result = driver.native_trial(row, directory, args, modules, roles)
            driver.save(directory/'result.json', result)
            record['attempts'].append(dict(ordinal=row['ordinal'], directory=str(directory), result=result))
            driver.save(args.output/'matrix.json', record)
            require(result['complete'], 'incomplete cohort; original failure preserved')
            record['attempts'][-1]['health'] = health.health(result, driver.read(directory/'run/report.json'))
            for phase in ['before', 'after']:
                statuses = driver.read(directory/(phase+'-status.json'))
                require(all(('write_path_diagnostics' in s) == (row['mode']=='instrumented')
                            for s in statuses.values()), 'runtime observer feature differs')
                require(all(('write_stage_trace' in s) == (row['mode']=='instrumented')
                            for s in statuses.values()), 'runtime trace feature differs')
            write_stage_capture.compare_retained(directory, row['mode']=='instrumented', HERE/'check-write-stage-trace.py')
            print(json.dumps(dict(completed=row['ordinal'], available_bytes=available(args.output))), flush=True)
        record['complete'] = True
    except BaseException as error:
        record['failure'] = repr(error)
        raise
    finally:
        record['available_after'] = available(args.output)
        driver.save(args.output/'matrix.json', record)
    print(json.dumps(dict(complete=True, cohorts=len(rows), available_bytes=available(args.output))), flush=True)


if __name__ == '__main__':
    main()
