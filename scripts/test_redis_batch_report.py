"""RESP sizing and uncertain-batch accounting examples, independent of Rust."""
import copy
import unittest
from redis_batch_report import config_check, metrics_check, paired_configuration
from test_batch_benchmark_report import exact_histogram


def config():
    return dict(version=1, address='127.0.0.1:6379', deadline_ms=1500, run_id='paired', seed=71,
                workers=4, keys=256, batch_size=16, value_bytes=128, read_percent=50,
                warmup_calls=8, measure_ms=1000, max_calls=1000, load=dict(kind='closed_loop'))


def get_config():
    c = config()
    c.update(version=2, read_api='get', batch_size=1, read_percent=100)
    return c


def api_config():
    c = config()
    c.update(version=3, read_api='get', write_api='set', batch_size=1)
    return c


def native_config(c, read_api=None, write_api=None):
    native = {k: copy.deepcopy(v) for k, v in c.items()
              if k not in ('address', 'deadline_ms', 'read_api', 'write_api')}
    if c['version'] == 3:
        native.update(read_api=read_api, write_api=write_api)
    elif read_api is not None:
        native.update(version=2, read_api=read_api)
    native.update(rpc_transport='tonic_stream', client=dict(
        version=1, peers=[dict(node_id=1, address='127.0.0.1:20160')], keyspace_id=1,
        epoch_conf_ver=1, epoch_version=1, max_in_flight=c['workers'], max_attempts=6,
        deadline_ms=c['deadline_ms'], retry_backoff_ms=5))
    return native


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
    def test_v3_explicit_read_and_write_apis_and_legacy_shapes(self):
        for read_api in ('mget', 'get'):
            for write_api in ('mset', 'set'):
                for read_percent in (0, 50, 100):
                    c = api_config()
                    c.update(read_api=read_api, write_api=write_api, read_percent=read_percent)
                    with self.subTest(read=read_api, write=write_api, percent=read_percent):
                        config_check(c)
        legacy = config()
        _, sizes = config_check(legacy)
        v3_batch = copy.deepcopy(legacy)
        v3_batch.update(version=3, read_api='mget', write_api='mset')
        self.assertEqual(config_check(v3_batch)[1], sizes)
        self.assertNotIn('write_api', legacy)
        self.assertNotIn('write_api', get_config())
        self.assertEqual(len(config_check(get_config())[1]), 7)

    def test_v3_requires_both_selectors_and_point_cardinality(self):
        for field in ('read_api', 'write_api'):
            c = api_config(); del c[field]
            with self.subTest(missing=field), self.assertRaises(ValueError): config_check(c)
            for invalid in (None, False, [], 'unknown'):
                c = api_config(); c[field] = invalid
                with self.subTest(field=field, value=invalid), self.assertRaises(ValueError): config_check(c)
        for version in (1, 2, 4):
            c = api_config(); c['version'] = version
            with self.subTest(version=version), self.assertRaises(ValueError): config_check(c)
        for read, write in (('get', 'set'), ('get', 'mset'), ('mget', 'set')):
            c = api_config(); c.update(read_api=read, write_api=write, batch_size=2)
            with self.subTest(read=read, write=write), self.assertRaises(ValueError): config_check(c)
        c = api_config(); c.update(read_api='mget', write_api='mset', batch_size=2)
        config_check(c)

    def test_set_sizes_are_actual_set_frames(self):
        c = api_config()
        _, sizes = config_check(c)
        key = (c['run_id']+':'+format(0, '016x')).encode()
        value = bytes(c['value_bytes'])
        request = (b'*3\r\n$3\r\nSET\r\n$'+str(len(key)).encode()+b'\r\n'+key+
                   b'\r\n$'+str(len(value)).encode()+b'\r\n'+value+b'\r\n')
        self.assertEqual(sizes['set_request_bytes'], len(request))
        self.assertEqual(sizes['set_response_bytes'], len(b'+OK\r\n'))
        self.assertEqual(sizes['mset_request_bytes'], len(request)+1)
        self.assertEqual(len(sizes), 9)

    def test_v3_pairing_checks_both_apis_including_inactive_selectors(self):
        for read, native_read in (('mget', 'batch_get'), ('get', 'point_get')):
            for write, native_write in (('mset', 'batch_put'), ('set', 'point_put')):
                for percent in (0, 50, 100):
                    c = api_config(); c.update(read_api=read, write_api=write, read_percent=percent)
                    native = native_config(c, native_read, native_write)
                    pair = paired_configuration(c, native)
                    self.assertEqual(pair['read_api_pair'], dict(redis=read, native=native_read))
                    self.assertEqual(pair['write_api_pair'], dict(redis=write, native=native_write))
                    wrong = copy.deepcopy(native)
                    wrong['write_api'] = 'point_put' if native_write == 'batch_put' else 'batch_put'
                    with self.assertRaises(ValueError): paired_configuration(c, wrong)
                    wrong = copy.deepcopy(native)
                    wrong['read_api'] = 'point_get' if native_read == 'batch_get' else 'batch_get'
                    with self.assertRaises(ValueError): paired_configuration(c, wrong)
        self.assertNotIn('write_api_pair', paired_configuration(config(), native_config(config())))
        self.assertNotIn('write_api_pair', paired_configuration(get_config(), native_config(get_config(), 'point_get')))

    def test_explicit_get_sizes_and_legacy_schema(self):
        c = get_config()
        _, sizes = config_check(c)
        key = (c['run_id']+':'+format(0, '016x')).encode()
        request = b'*2\r\n$3\r\nGET\r\n$'+str(len(key)).encode()+b'\r\n'+key+b'\r\n'
        response = b'$128\r\n'+bytes(128)+b'\r\n'
        self.assertEqual(sizes['get_request_bytes'], len(request))
        self.assertEqual(sizes['get_response_bytes'], len(response))
        self.assertEqual(sizes['mget_request_bytes'], len(request)+1)
        self.assertEqual(sizes['mget_response_bytes'], len(response)+4)
        self.assertEqual(len(sizes), 7)
        legacy = config()
        _, original = config_check(legacy)
        self.assertEqual(len(original), 5)
        explicit = copy.deepcopy(legacy); explicit.update(version=2, read_api='mget')
        self.assertEqual(config_check(explicit)[1], original)

    def test_missing_null_ambiguous_or_nonpoint_api_is_rejected(self):
        c = get_config()
        for key, value in [('version', 1), ('version', True), ('version', 3),
                           ('read_api', None), ('read_api', 'GET'), ('read_api', False),
                           ('read_api', []), ('batch_size', 2), ('batch_size', True),
                           ('read_percent', 99)]:
            altered = copy.deepcopy(c); altered[key] = value
            with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                config_check(altered)
        del c['read_api']
        with self.assertRaises(ValueError): config_check(c)
        for malformed in [None, [], 'get']:
            with self.assertRaises(ValueError): config_check(malformed)

    def test_pairing_distinguishes_get_from_single_item_mget(self):
        c = get_config()
        native = native_config(c, 'point_get')
        self.assertEqual(paired_configuration(c, native)['read_api_pair'],
                         dict(redis='get', native='point_get'))
        wrong = copy.deepcopy(native); wrong['read_api'] = 'batch_get'
        with self.assertRaises(ValueError): paired_configuration(c, wrong)
        redis_mget = copy.deepcopy(c); redis_mget['read_api'] = 'mget'
        with self.assertRaises(ValueError): paired_configuration(redis_mget, native)
        paired_configuration(redis_mget, wrong)
        legacy = config()
        self.assertNotIn('read_api_pair', paired_configuration(legacy, native_config(legacy)))

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
    def test_mixed_v3_get_and_unknown_set_keep_one_command_each(self):
        c = api_config(); m = metrics()
        m['operations'] = ['get', 'set']
        write = m['statistics'][1]
        write['populations'][1]['input_items'] = 1
        read = m['statistics'][0]
        read['populations'][0].update(calls=1, input_items=1, completed_before_cutoff=1,
                                     whole_call=exact_histogram([90]), client_call=exact_histogram([80]))
        read['reasons'][0] = 1
        read['command_attempts'] = 1
        self.assertEqual(metrics_check(m, c, 'measurement'),
                         dict(calls=[1, 1], successes=1, successful_items=1, attempts=2))
        for change in ('replay', 'items', 'label', 'outcome'):
            bad = copy.deepcopy(m); op = bad['statistics'][1]
            if change == 'replay': op['command_attempts'] = 2
            elif change == 'items': op['populations'][1]['input_items'] = 2
            elif change == 'label': bad['operations'][1] = 'mset'
            else: op['populations'][1], op['populations'][2] = op['populations'][2], op['populations'][1]
            with self.subTest(change=change), self.assertRaises(ValueError): metrics_check(bad, c, 'measurement')

    def test_v3_selected_labels_are_confined_to_warmup_and_measurement(self):
        c = api_config(); empty = metrics()
        empty['statistics'] = [copy.deepcopy(empty['statistics'][0]) for _ in range(2)]
        for phase in ('initialization', 'verification'):
            metrics_check(empty, c, phase)
            for labels in (['get', 'mset'], ['mget', 'set'], ['get', 'set']):
                bad = copy.deepcopy(empty); bad['operations'] = labels
                with self.subTest(phase=phase, labels=labels), self.assertRaises(ValueError): metrics_check(bad, c, phase)
        for phase in ('warmup', 'measurement'):
            selected = copy.deepcopy(empty); selected['operations'] = ['get', 'set']
            metrics_check(selected, c, phase)
            with self.assertRaises(ValueError): metrics_check(empty, c, phase)

    def test_get_failure_has_point_labels_only_during_traffic_and_one_item(self):
        c = get_config(); m = metrics()
        m['operations'][0] = 'get'
        m['statistics'].reverse()
        read = m['statistics'][0]
        read['populations'][1], read['populations'][2] = read['populations'][2], read['populations'][1]
        read['populations'][2]['input_items'] = 1
        self.assertEqual(metrics_check(m, c, 'measurement'),
                         dict(calls=[1, 0], successes=0, successful_items=0, attempts=1))
        for field, value in [('input_items', 2), ('whole_call', exact_histogram([]))]:
            bad = copy.deepcopy(m); bad['statistics'][0]['populations'][2][field] = value
            with self.assertRaises(ValueError): metrics_check(bad, c, 'measurement')
        replay = copy.deepcopy(m); replay['statistics'][0]['command_attempts'] = 2
        with self.assertRaises(ValueError): metrics_check(replay, c, 'measurement')
        wrong_labels = copy.deepcopy(m); wrong_labels['operations'][0] = 'mget'
        with self.assertRaises(ValueError): metrics_check(wrong_labels, c, 'measurement')
        empty = metrics()
        empty['statistics'] = [copy.deepcopy(empty['statistics'][0]) for _ in range(2)]
        for phase in ('initialization', 'verification'):
            metrics_check(empty, c, phase)
            bad = copy.deepcopy(empty); bad['operations'][0] = 'get'
            with self.assertRaises(ValueError): metrics_check(bad, c, phase)
        empty['operations'][0] = 'get'
        metrics_check(empty, c, 'warmup')

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
