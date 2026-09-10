"""RESP sizing and uncertain-batch accounting examples, independent of Rust."""
import copy
import unittest
from redis_batch_report import config_check, metrics_check, paired_configuration
from test_batch_benchmark_report import exact_histogram


def config():
    return dict(version=1, address='127.0.0.1:6379', deadline_ms=1500, run_id='paired', seed=71,
                workers=4, keys=256, batch_size=16, value_bytes=128, read_percent=50,
                warmup_calls=8, measure_ms=1000, max_calls=1000, load=dict(kind='closed_loop'))


def metrics():
    def population():
        return dict(calls=0, input_items=0, completed_before_cutoff=0, whole_call=exact_histogram([]),
                    client_call=exact_histogram([]), scheduled_to_completion=exact_histogram([]))
    def operation():
        return dict(populations=[population() for _ in range(3)], reasons=[0]*6,
                    dispatch_lateness=exact_histogram([]), connection_attempts=0,
                    connection_failures=0, command_attempts=0, data_failures=0)
    m = dict(operations=['mget', 'mset'], outcomes=['success', 'unknown_write', 'read_failure'],
             reasons=['success', 'deadline', 'io', 'server_error', 'protocol', 'data_integrity'],
             histogram_subdivisions=64, valid=True, statistics=[operation(), operation()])
    op = m['statistics'][1]
    op['populations'][1].update(calls=1, input_items=16, whole_call=exact_histogram([120]), client_call=exact_histogram([100]))
    op['reasons'][1] = 1
    op['command_attempts'] = 1
    return m


class Configuration(unittest.TestCase):
    def test_wire_sizes_match_concrete_resp_frames(self):
        c = config()
        _, sizes = config_check(c)
        def frame(parts):
            return b'*'+str(len(parts)).encode()+b'\r\n'+b''.join(b'$'+str(len(p)).encode()+b'\r\n'+p+b'\r\n' for p in parts)
        names = [(c['run_id']+':'+format(i, '016x')).encode() for i in range(c['batch_size'])]
        self.assertEqual(sizes['mget_request_bytes'], len(frame([b'MGET', *names])))
        self.assertEqual(sizes['mset_request_bytes'], len(frame([b'MSET', *[p for name in names for p in (name, bytes(c['value_bytes']))]])))
        self.assertEqual(sizes['mget_response_bytes'], len(frame([bytes(c['value_bytes'])]*c['batch_size'])))

    def test_bounds_refuse_invalid_or_unbounded_load(self):
        for key, value in [('workers', True), ('workers', 257), ('batch_size', 0), ('keys', 8), ('deadline_ms', 30001), ('max_calls', 0)]:
            c = config(); c[key] = value
            with self.assertRaises(ValueError): config_check(c)
        c = config(); c.update(batch_size=256, value_bytes=8192)
        with self.assertRaises(ValueError): config_check(c)
        c = config(); c['load'] = dict(kind='fixed_rate', batches_per_second=1001)
        with self.assertRaises(ValueError): config_check(c)

    def test_pairing_requires_identical_workload_and_deadline(self):
        c = config()
        native = {k:copy.deepcopy(v) for k, v in c.items() if k not in ('address', 'deadline_ms')}
        native.update(rpc_transport='tonic_stream', client=dict(version=1, peers=[dict(node_id=1,address='127.0.0.1:20160')],
                      keyspace_id=1, epoch_conf_ver=1, epoch_version=1, max_in_flight=4, max_attempts=6,
                      deadline_ms=1500, retry_backoff_ms=5))
        paired_configuration(c, native)
        for key in ('seed', 'batch_size', 'workers', 'read_percent', 'value_bytes'):
            altered = copy.deepcopy(native); altered[key] -= 1
            with self.assertRaises(ValueError): paired_configuration(c, altered)
        native['client']['deadline_ms'] = 1499
        with self.assertRaises(ValueError): paired_configuration(c, native)


class Accounting(unittest.TestCase):
    def test_lost_mset_reply_remains_one_whole_unknown(self):
        self.assertEqual(metrics_check(metrics(), config(), 'measurement'),
                         dict(calls=[0, 1], successes=0, successful_items=0, attempts=1))

    def test_connect_failure_is_visible_without_a_command_replay(self):
        m = metrics(); op = m['statistics'][1]
        op.update(connection_attempts=1, connection_failures=1, command_attempts=0)
        metrics_check(m, config(), 'measurement')
        op['command_attempts'] = 1
        with self.assertRaises(ValueError): metrics_check(m, config(), 'measurement')

    def test_wrong_outcomes_and_omitted_whole_items_are_rejected(self):
        m = metrics(); m['statistics'].reverse()
        with self.assertRaises(ValueError): metrics_check(m, config(), 'measurement')
        for field, value in [('input_items', 15), ('whole_call', exact_histogram([])), ('client_call', exact_histogram([121]))]:
            m = metrics(); m['statistics'][1]['populations'][1][field] = value
            with self.assertRaises(ValueError): metrics_check(m, config(), 'measurement')

    def test_fixed_rate_unknown_keeps_late_dispatch_in_latency(self):
        c = config(); c['load'] = dict(kind='fixed_rate', batches_per_second=100)
        m = metrics(); op = m['statistics'][1]
        op['populations'][1]['scheduled_to_completion'] = exact_histogram([150])
        op['dispatch_lateness'] = exact_histogram([30])
        metrics_check(m, c, 'measurement')
        op['populations'][1]['scheduled_to_completion'] = exact_histogram([120])
        with self.assertRaises(ValueError): metrics_check(m, c, 'measurement')


if __name__ == '__main__': unittest.main()
