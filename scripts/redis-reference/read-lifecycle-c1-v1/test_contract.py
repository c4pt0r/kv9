#!/usr/bin/env python3
"""Pure c1 binding controls; importing the reader does not run its main."""
from copy import deepcopy
import importlib.util
from pathlib import Path
import unittest

path = Path(__file__).with_name('read_lifecycle.py')
spec = importlib.util.spec_from_file_location('c1_reader_contract', path)
reader = importlib.util.module_from_spec(spec)
spec.loader.exec_module(reader)


def configuration(api):
    return dict(version=2, rpc_transport='tonic_stream', read_api=api,
                run_id='prfget' if api == 'point_get' else 'prfbat', seed=71,
                workers=1, keys=4096, batch_size=1, value_bytes=128, read_percent=100,
                warmup_calls=128, measure_ms=5000, max_calls=10000000,
                load={'kind': 'closed_loop'},
                client=dict(version=1, peers=[{'node_id': 1, 'address': '127.0.0.1:1'}],
                            keyspace_id=100, epoch_conf_ver=1, epoch_version=1,
                            max_in_flight=1, max_attempts=6, deadline_ms=1500,
                            retry_backoff_ms=5))


def report(config):
    return dict(configuration=deepcopy(config), config_sha256='a' * 64,
                build=dict(revision=reader.C1_PROTOCOL['client_revision'], dirty=False,
                           profile='release', binary_sha256=reader.C1_PROTOCOL['client_sha256']))


class C1ContractTests(unittest.TestCase):
    def test_lifecycle_only_protocol_and_both_apis(self):
        reader.check_c1_protocol(deepcopy(reader.C1_PROTOCOL))
        for api in ('point_get', 'batch_get'):
            c = configuration(api)
            reader.check_c1_configuration(api, c, report(c), 'a' * 64)

    def test_old_or_cpu_profile_protocol_is_rejected(self):
        for key, value in [('version', 1), ('workers', 64), ('cpu_profiling', True),
                           ('protocol_id', 'read-wait-profile'), ('event', 'cpu-clock'),
                           ('profiler_cpus', [6, 7])]:
            with self.subTest(key=key):
                p = deepcopy(reader.C1_PROTOCOL)
                p[key] = value
                with self.assertRaises(ValueError):
                    reader.check_c1_protocol(p)

    def test_false_numeric_source_placement_and_payload_claims_rejected(self):
        for key, value in [('workers', True), ('server_revision', '5ee897a'),
                           ('client_sha256', '0' * 64), ('observer_cpus', [0, 1]),
                           ('value_bytes', 64), ('measure_ms', 1500)]:
            p = deepcopy(reader.C1_PROTOCOL)
            p[key] = value
            with self.assertRaises(ValueError):
                reader.check_c1_protocol(p)

    def test_worker_and_sdk_concurrency_cannot_disagree(self):
        for workers, maximum in [(64, 64), (1, 64), (64, 1), (True, 1), (1, True)]:
            c = configuration('point_get')
            c['workers'], c['client']['max_in_flight'] = workers, maximum
            with self.assertRaises(ValueError):
                reader.check_c1_configuration('point_get', c, report(c), 'a' * 64)

    def test_api_run_id_and_deadline_are_frozen(self):
        for key, value in [('read_api', 'batch_get'), ('run_id', 'prfbat'),
                           ('keys', 64), ('warmup_calls', 0), ('max_calls', 1000000)]:
            c = configuration('point_get')
            c[key] = value
            with self.assertRaises(ValueError):
                reader.check_c1_configuration('point_get', c, report(c), 'a' * 64)
        for key, value in [('deadline_ms', 3000), ('max_attempts', 1), ('retry_backoff_ms', 0)]:
            c = configuration('point_get')
            c['client'][key] = value
            with self.assertRaises(ValueError):
                reader.check_c1_configuration('point_get', c, report(c), 'a' * 64)

    def test_report_config_hash_content_and_client_identity_are_bound(self):
        c = configuration('point_get')
        for key, value in [('config_sha256', 'b' * 64), ('configuration', configuration('batch_get'))]:
            r = report(c)
            r[key] = value
            with self.assertRaises(ValueError):
                reader.check_c1_configuration('point_get', c, r, 'a' * 64)
        for key, value in [('dirty', True), ('profile', 'debug'), ('binary_sha256', '0' * 64)]:
            r = report(c)
            r['build'][key] = value
            with self.assertRaises(ValueError):
                reader.check_c1_configuration('point_get', c, r, 'a' * 64)


if __name__ == '__main__':
    unittest.main(verbosity=2)
