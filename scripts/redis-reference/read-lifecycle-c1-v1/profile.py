#!/usr/bin/env python3
"""Two c1 read-lifecycle recordings; no CPU profiler or throughput acceptance."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import signal
import sys
import time
from types import SimpleNamespace

if not __debug__:
    raise RuntimeError('lifecycle recording requires PYTHONOPTIMIZE=0')
sys.dont_write_bytecode = True
OUT = Path(__file__).parent
LEGACY = Path('/tmp/kv9-rpc-pair-read-profile-run-first/profile.py')
LEGACY_SHA = 'ac86eded850433faef464c604948bb29a5e251ec5c523384b2d28d988f469e7c'
OBSERVER_CPUS = list(range(6, 16)) + list(range(22, 32))


def require(value, message):
    if not value:
        raise ValueError(message)


def sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def read(path):
    return json.loads(Path(path).read_text())


def save(path, value):
    Path(path).write_text(json.dumps(value, indent=2) + '\n')


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


require(sha(LEGACY) == LEGACY_SHA, 'original source/build preflight helper changed')
legacy = module('retained_c64_profile_preflight', LEGACY)
DRIVER, DRIVER_SHA = legacy.DRIVER, legacy.DRIVER_SHA
SOURCE, SERVER_SOURCE, SERVER_BUILD = legacy.SOURCE, legacy.SERVER_SOURCE, legacy.SERVER_BUILD
CLIENT_BUILD, CLIENT_REV, CLIENT_SHA, RESP = legacy.CLIENT_BUILD, legacy.CLIENT_REV, legacy.CLIENT_SHA, legacy.RESP
preflight = legacy.preflight


def main():
    require(sorted(os.sched_getaffinity(0)) == OBSERVER_CPUS, 'observer CPU mask differs')
    require(sha(DRIVER) == DRIVER_SHA, 'original fixture driver changed')
    require(not (OUT / 'summary.json').exists(), 'recording attempt already started')
    driver = module('retained_c1_lifecycle_fixture', DRIVER)
    frozen = preflight(driver)
    sys.path.insert(0, str(SOURCE / 'scripts'))
    import benchmark
    import batch_benchmark_report as native_validator
    import redis_batch_report as paired_validator
    support = module('native_lifecycle_support', SOURCE / 'scripts/native-batch-e2e.py')
    tmpfs = module('lifecycle_tmpfs_support', SOURCE / 'scripts/tmpfs-redis-diagnostic.py')
    comparison = module('lifecycle_resource_support', SOURCE / 'scripts/redis-comparison.py')
    resp = module('retained_lifecycle_readback', RESP)
    _, manifest, features = native_validator.build_check(CLIENT_BUILD, CLIENT_BUILD, CLIENT_REV)
    require(features == [] and manifest['profile'] == 'release', 'client is not a default release')
    roles = {'new': dict(build_directory=str(SERVER_BUILD), binary_sha256=driver.SERVER_PINS['new']['binary_sha256']),
             'client': dict(build_directory=str(CLIENT_BUILD), binary_sha256=CLIENT_SHA)}
    args = SimpleNamespace(smoke=True, storage='tmpfs', client_build=CLIENT_BUILD, expected_client_revision=CLIENT_REV)
    driver.placement = lambda _: dict(client_cpus=[0, 1], server_cpus=[2, 3, 4, 5], separated=True, exclusive_host=False)
    modules = benchmark, support, tmpfs, comparison, resp, native_validator, paired_validator
    sources = {str(DRIVER): DRIVER_SHA, str(LEGACY): LEGACY_SHA,
               str(Path(__file__)): sha(__file__), str(RESP): sha(RESP)}
    for loaded in list(sys.modules.values()) + [support, tmpfs, comparison]:
        filename = getattr(loaded, '__file__', None)
        if filename and Path(filename).resolve().is_relative_to(SOURCE):
            file = Path(filename).resolve()
            require(sha(file) == frozen['client_sources']['sources'][str(file.relative_to(SOURCE))], 'loaded helper changed')
            sources[str(file)] = sha(file)
    protocol = dict(version=2, protocol_id='kv9-read-lifecycle-c1-v1', instrumented=True,
                    throughput_acceptance=False, cpu_profiling=False, server_revision=driver.NEW_REVISION,
                    client_revision=CLIENT_REV, server_sha256=roles['new']['binary_sha256'], client_sha256=CLIENT_SHA,
                    measure_ms=5000, workers=1, keys=4096, batch_size=1, value_bytes=128, read_percent=100,
                    warmup_calls=128, max_calls=10000000, deadline_ms=1500, client_cpus=[0, 1],
                    server_cpus=[2, 3, 4, 5], observer_cpus=OBSERVER_CPUS, storage=tmpfs.VOLATILE,
                    exclusive_host=False, profiles=['point_get', 'batch_get'], helper_sources=sources)
    save(OUT / 'protocol.json', protocol)
    save(OUT / 'build-bindings.json', frozen)
    summary = dict(complete=False, instrumented=True, throughput_acceptance=False, cpu_profiling=False, profiles=[])
    save(OUT / 'summary.json', summary)

    def interrupted(signum, _):
        raise RuntimeError('lifecycle recording interrupted by signal ' + str(signum))

    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    try:
        for api, run_id in [('point_get', 'prfget'), ('batch_get', 'prfbat')]:
            directory = OUT / api
            directory.mkdir()
            row = dict(arm=api, server_role='new', read_api=api, target='kv9', repeat=0, mix='read',
                       shared=dict(run_id=run_id, seed=71, workers=1, keys=4096, batch_size=1,
                                   value_bytes=128, read_percent=100, warmup_calls=128, measure_ms=5000,
                                   max_calls=10000000, load=dict(kind='closed_loop')))
            save(directory / 'profile-plan.json', row)
            print('START: c1 read lifecycle ' + api, flush=True)
            outcome = driver.native_trial(row, directory, args, modules, roles)
            save(directory / 'fixture-result.json', outcome)
            summary['profiles'].append(dict(api=api, fixture_complete=outcome['complete'], directory=str(directory)))
            save(OUT / 'summary.json', summary)
            require(outcome['complete'], 'lifecycle fixture failed; original attempt retained')
            print('TERMINAL: c1 read lifecycle ' + api, flush=True)
        require(preflight(driver) == frozen and all(sha(p) == h for p, h in sources.items()), 'frozen inputs changed')
        summary['complete'] = True
    except BaseException as error:
        summary['failure'] = repr(error)
        raise
    finally:
        summary['ended_unix_ns'] = time.time_ns()
        save(OUT / 'summary.json', summary)
    print('PASS: two lifecycle fixtures complete; independent readbacks pending', flush=True)


if __name__ == '__main__':
    main()
