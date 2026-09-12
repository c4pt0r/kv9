"""Independent replication-confirmation accounting and false-success controls."""
import copy
import unittest

from redis_batch_report import config_check, metrics_check, paired_configuration
from test_redis_batch_report import config, metrics
from test_batch_benchmark_report import exact_histogram


def confirmed_config(replicas=1, batch=16):
    return dict(config(), version=4, read_api='get' if batch == 1 else 'mget',
                write_api='set' if batch == 1 else 'mset', batch_size=batch,
                write_confirmation=dict(kind='wait', replicas=replicas, timeout_ms=1000))


def observation(acknowledged=None, reason='deadline', batch=16):
    m = metrics()
    m['reasons'].append('replication_shortfall')
    for op in m['statistics']:
        op['reasons'].append(0)
        op.update(confirmation_attempts=0, replica_confirmation_replies=[0, 0, 0],
                  total_resp_command_attempts=0)
    op = m['statistics'][1]
    op['populations'][1]['input_items'] = batch
    op['reasons'] = [0] * 7
    op['reasons'][m['reasons'].index(reason)] = 1
    if reason == 'success':
        op['populations'][0], op['populations'][1] = op['populations'][1], op['populations'][0]
    op.update(confirmation_attempts=1, total_resp_command_attempts=2)
    if acknowledged is not None:
        op['replica_confirmation_replies'][acknowledged] = 1
    if batch == 1:
        m['operations'] = ['get', 'set']
    return m


class ReplicationConfiguration(unittest.TestCase):
    def test_wait_sizes_match_actual_resp_and_timeout_is_inside_call_budget(self):
        for replicas in (1, 2):
            for batch in (1, 64):
                c = confirmed_config(replicas, batch)
                _, size = config_check(c)
                self.assertEqual(size['confirmation_request_bytes'],
                                 len(f'*3\r\n$4\r\nWAIT\r\n$1\r\n{replicas}\r\n$4\r\n1000\r\n'.encode()))
        for field in (None, False, {}, {'kind': 'wait'}, {'kind': 'unknown'},
                      {'kind': 'async', 'replicas': 1},
                      {'kind': 'wait', 'replicas': True, 'timeout_ms': 1},
                      {'kind': 'wait', 'replicas': 0, 'timeout_ms': 1},
                      {'kind': 'wait', 'replicas': 3, 'timeout_ms': 1},
                      {'kind': 'wait', 'replicas': 1, 'timeout_ms': 0},
                      {'kind': 'wait', 'replicas': 1, 'timeout_ms': 1501}):
            with self.subTest(confirmation=field):
                with self.assertRaises(ValueError):
                    config_check(dict(confirmed_config(), write_confirmation=field))
        missing = confirmed_config(); del missing['write_confirmation']
        with self.assertRaises(ValueError): config_check(missing)
        for version in (1, 2, 3):
            with self.assertRaises(ValueError): config_check(dict(confirmed_config(), version=version))
        asynchronous = dict(confirmed_config(), write_confirmation={'kind': 'async'})
        self.assertNotIn('confirmation_request_bytes', config_check(asynchronous)[1])

    def test_pairing_keeps_replication_mode_explicit_and_checks_native_api(self):
        c = confirmed_config(2, 1)
        native = {k: copy.deepcopy(v) for k, v in c.items()
                  if k not in ('address', 'deadline_ms', 'write_confirmation')}
        native.update(version=3, read_api='point_get', write_api='point_put', rpc_transport='tonic_stream',
                      client=dict(version=1, peers=[dict(node_id=1, address='127.0.0.1:20160')],
                                  keyspace_id=1, epoch_conf_ver=1, epoch_version=1, max_in_flight=4,
                                  max_attempts=6, deadline_ms=1500, retry_backoff_ms=5))
        pair = paired_configuration(c, native)
        self.assertEqual(pair['redis_write_confirmation'], c['write_confirmation'])
        self.assertEqual(pair['write_api_pair'], {'redis': 'set', 'native': 'point_put'})
        with self.assertRaises(ValueError):
            paired_configuration(c, dict(native, write_api='batch_put'))


class ReplicationAccounting(unittest.TestCase):
    def test_success_needs_sufficient_acknowledgments_for_each_requested_count(self):
        for required in (1, 2):
            for acknowledged in (0, 1, 2):
                for batch in (1, 64):
                    m = observation(acknowledged, 'success', batch)
                    if acknowledged >= required:
                        result = metrics_check(m, confirmed_config(required, batch), 'measurement')
                        self.assertEqual(result['successful_items'], batch)
                        self.assertEqual(result['total_resp_command_attempts'], 2)
                    else:
                        with self.assertRaisesRegex(ValueError, 'acknowledgments|replica replies'):
                            metrics_check(m, confirmed_config(required, batch), 'measurement')

    def test_shortfall_is_unknown_and_missing_ack_remains_deadline_or_io(self):
        for required in (1, 2):
            for ack in range(required):
                result = metrics_check(observation(ack, 'replication_shortfall'),
                                       confirmed_config(required), 'measurement')
                self.assertEqual(result['calls'], [0, 1])
                self.assertEqual(result['successes'], 0)
                self.assertEqual(result['confirmation_attempts'], 1)
        for reason in ('deadline', 'io', 'server_error'):
            result = metrics_check(observation(None, reason), confirmed_config(), 'measurement')
            self.assertEqual(result['successes'], 0)
            self.assertEqual(result['replica_confirmation_replies'], [0, 0, 0])
        with self.assertRaises(ValueError):
            metrics_check(observation(None, 'success'), confirmed_config(), 'measurement')
        with self.assertRaises(ValueError):
            metrics_check(observation(1, 'deadline'), confirmed_config(), 'measurement')

    def test_replay_missing_attempt_and_fabricated_replica_counters_are_rejected(self):
        for field, value in (
            ('confirmation_attempts', 0), ('confirmation_attempts', 2),
            ('total_resp_command_attempts', 1), ('total_resp_command_attempts', 3),
            ('replica_confirmation_replies', [0, 2, 0]),
            ('replica_confirmation_replies', [0, 0, 0]),
            ('replica_confirmation_replies', [0, True, 0]),
            ('replica_confirmation_replies', [0, 1, 0, 0]),
            ('command_attempts', 2),
        ):
            m = observation(1, 'success'); m['statistics'][1][field] = value
            with self.subTest(field=field, value=value):
                with self.assertRaises(ValueError): metrics_check(m, confirmed_config(), 'measurement')

    def test_connect_failure_has_no_write_or_confirmation_attempt(self):
        m = observation(None, 'io'); op = m['statistics'][1]
        op.update(command_attempts=0, confirmation_attempts=0, total_resp_command_attempts=0,
                  connection_attempts=1, connection_failures=1)
        metrics_check(m, confirmed_config(), 'measurement')
        op['confirmation_attempts'] = 1
        with self.assertRaises(ValueError): metrics_check(m, confirmed_config(), 'measurement')

    def test_reads_and_async_writes_cannot_claim_replication(self):
        c = dict(confirmed_config(), write_confirmation={'kind': 'async'})
        m = observation(None, 'success'); op = m['statistics'][1]
        op.update(confirmation_attempts=0, total_resp_command_attempts=1)
        metrics_check(m, c, 'measurement')
        with self.assertRaises(ValueError): metrics_check(observation(1, 'success'), c, 'measurement')
        m['statistics'].reverse()
        metrics_check(m, confirmed_config(), 'measurement')
        m['statistics'][0].update(confirmation_attempts=1, total_resp_command_attempts=2,
                                  replica_confirmation_replies=[0, 1, 0])
        with self.assertRaises(ValueError): metrics_check(m, confirmed_config(), 'measurement')

    def test_unknown_confirmation_keeps_whole_and_scheduled_latency(self):
        c = confirmed_config(2); c['load'] = {'kind': 'fixed_rate', 'batches_per_second': 100}
        m = observation(1, 'replication_shortfall'); op = m['statistics'][1]
        op['populations'][1]['scheduled_to_completion'] = exact_histogram([150])
        op['dispatch_lateness'] = exact_histogram([30])
        metrics_check(m, c, 'measurement')
        op['populations'][1]['scheduled_to_completion'] = exact_histogram([120])
        with self.assertRaises(ValueError): metrics_check(m, c, 'measurement')


if __name__ == '__main__': unittest.main()
