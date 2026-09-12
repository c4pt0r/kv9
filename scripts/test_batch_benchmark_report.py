"""Independent arithmetic examples and failure-population controls."""
import copy
import unittest

from batch_benchmark_report import (OPERATIONS, OUTCOMES, REASONS, bucket_bounds, config_check,
                                    dominance_check, histogram_check, metrics_check, report_check,
                                    strict_json)


def exact_histogram(samples):
    counts = [0] * 3776
    for sample in samples:
        # Deliberately search the mathematical intervals, without the producer's
        # bit-index formula, so integer boundary controls are independent.
        slot = next(i for i in range(3776) if bucket_bounds(i)[0] <= sample <= bucket_bounds(i)[1])
        counts[slot] += 1
    ordered = sorted(samples)
    result = {'raw': {'valid': True, 'count': len(samples), 'sum_ns': sum(samples),
                      'min_ns': min(samples) if samples else None,
                      'max_ns': max(samples) if samples else None, 'buckets': counts},
              'mean_ns': sum(samples) / len(samples) if samples else None}
    for p in (50, 95, 99):
        if not ordered:
            result['p' + str(p)] = None
        else:
            sample = ordered[(len(samples) * p + 99) // 100 - 1]
            lo, hi = next(bucket_bounds(i) for i in range(3776) if bucket_bounds(i)[0] <= sample <= bucket_bounds(i)[1])
            result['p' + str(p)] = {'lower_ns': lo, 'upper_ns': hi}
    return result


class ReadApiConfigurations(unittest.TestCase):
    def config(self):
        return dict(version=1, client=dict(version=1,
                    peers=[dict(node_id=1, address='127.0.0.1:12345')],
                    keyspace_id=7, epoch_conf_ver=1, epoch_version=1,
                    max_in_flight=1, max_attempts=1, deadline_ms=1000, retry_backoff_ms=1),
                    rpc_transport='tonic_stream', run_id='paired', seed=71, workers=1,
                    keys=2, batch_size=1, value_bytes=128, read_percent=100,
                    warmup_calls=0, measure_ms=1, max_calls=100,
                    load=dict(kind='closed_loop'))

    def test_legacy_schema_and_explicit_single_key_apis_have_equal_payload_sizes(self):
        legacy = self.config()
        expected = config_check(legacy)
        for api in ('batch_get', 'point_get'):
            selected = dict(legacy, version=2, read_api=api)
            self.assertEqual(config_check(selected), expected)
        for extra in (None, 'batch_get', 'point_get'):
            with self.assertRaises(ValueError):
                config_check(dict(legacy, read_api=extra))

    def test_missing_null_unknown_or_unmatched_point_selector_is_rejected(self):
        original = dict(self.config(), version=2, read_api='point_get')
        config_check(original)
        missing = dict(original)
        del missing['read_api']
        cases = [missing] + [dict(original, read_api=value) for value in (None, 'unknown', False)]
        cases += [dict(original, batch_size=2), dict(original, read_percent=50)]
        for invalid in cases:
            with self.assertRaises(ValueError):
                config_check(invalid)

    def test_point_labels_apply_only_to_the_selected_measured_and_warmup_phases(self):
        metrics, _ = FailureAccounting().case()
        # Zero populations isolate the label contract from unrelated call mix.
        metrics['statistics'][1] = copy.deepcopy(metrics['statistics'][0])
        config = dict(self.config(), version=2, read_api='point_get')
        for phase in ('initialization', 'warmup', 'measurement', 'verification'):
            selected = ['get', 'batch_put'] if phase in ('warmup', 'measurement') else OPERATIONS
            metrics['operations'] = selected
            metrics_check(metrics, config, phase)
            metrics['operations'] = OPERATIONS if selected != OPERATIONS else ['get', 'batch_put']
            with self.assertRaisesRegex(ValueError, 'metric vocabulary differs: operations'):
                metrics_check(metrics, config, phase)


class ExplicitWriteApis(unittest.TestCase):
    def config(self):
        return dict(ReadApiConfigurations().config(), version=3,
                    read_api='point_get', write_api='point_put', read_percent=50)

    def test_v3_mixed_and_pure_selectors_keep_actual_point_put_wire_shape(self):
        c = self.config()
        for read in ('point_get', 'batch_get'):
            for write in ('point_put', 'batch_put'):
                for percent in (0, 50, 100):
                    selected = dict(c, read_api=read, write_api=write, read_percent=percent)
                    _, sizes = config_check(selected)
                    # Context10 + key25 + value131. BatchPut adds the three-byte
                    # wrapper for its one nested156-byte KeyValue message.
                    self.assertEqual(sizes['put_request_bytes'], 166 if write == 'point_put' else 169)
                    self.assertEqual(sizes['get_request_bytes'], 35)
                    self.assertEqual(sizes['get_response_bytes'], 136)
                    self.assertEqual(sizes['maximum_input_items_in_flight'], 1)

    def test_malformed_missing_and_multikey_selectors_are_rejected_in_every_version(self):
        c = self.config()
        for name in ('read_api', 'write_api'):
            missing = dict(c)
            del missing[name]
            cases = [missing] + [dict(c, **{name: value}) for value in
                                (None, False, 1, [], {}, 'unknown', 'get', 'put')]
            for invalid in cases:
                with self.subTest(invalid=invalid):
                    with self.assertRaises(ValueError):
                        config_check(invalid)
        for read, write in (('point_get', 'batch_put'), ('batch_get', 'point_put'),
                            ('point_get', 'point_put')):
            for percent in (0, 50, 100):
                with self.assertRaises(ValueError):
                    config_check(dict(c, read_api=read, write_api=write,
                                      batch_size=2, read_percent=percent))
        config_check(dict(c, read_api='batch_get', write_api='batch_put', batch_size=2))
        for version in (1, 2):
            legacy = ReadApiConfigurations().config()
            if version == 2:
                legacy.update(version=2, read_api='point_get')
            before = copy.deepcopy(legacy)
            config_check(legacy)
            self.assertEqual(legacy, before)
            for selector in (None, 'point_put', 'batch_put'):
                with self.assertRaises(ValueError):
                    config_check(dict(legacy, write_api=selector))
        with self.assertRaises(ValueError):
            config_check(dict(ReadApiConfigurations().config(), version=2,
                              read_api='point_get', read_percent=50))

    def test_labels_apply_only_to_warmup_and_measurement_for_all_api_pairs(self):
        metrics, _ = FailureAccounting().case()
        metrics['statistics'][1] = copy.deepcopy(metrics['statistics'][0])
        for read, read_label in (('point_get', 'get'), ('batch_get', 'batch_get')):
            for write, write_label in (('point_put', 'put'), ('batch_put', 'batch_put')):
                c = dict(self.config(), read_api=read, write_api=write)
                for phase in ('initialization', 'warmup', 'measurement', 'verification'):
                    labels = [read_label, write_label] if phase in ('warmup', 'measurement') else OPERATIONS
                    metrics['operations'] = labels
                    metrics_check(metrics, c, phase)
                    metrics['operations'] = ['get', 'put'] if labels != ['get', 'put'] else OPERATIONS
                    with self.assertRaisesRegex(ValueError, 'metric vocabulary differs: operations'):
                        metrics_check(metrics, c, phase)

    def test_uncertain_point_write_keeps_single_item_attempts_and_rejects_read_outcomes(self):
        metrics, c = FailureAccounting().case()
        c.update(version=3, read_api='point_get', write_api='point_put', batch_size=1)
        metrics['operations'] = ['get', 'put']
        metrics['statistics'][1]['populations'][2]['input_items'] = 1
        self.assertEqual(metrics_check(metrics, c, 'measurement'),
                         dict(calls=[0, 1], successes=0, successful_items=0, attempts=2))
        wrong = copy.deepcopy(metrics)
        wrong['statistics'][1]['populations'][2], wrong['statistics'][1]['populations'][3] = (
            wrong['statistics'][1]['populations'][3], wrong['statistics'][1]['populations'][2])
        with self.assertRaisesRegex(ValueError, 'read failure in write population'):
            metrics_check(wrong, c, 'measurement')
        wrong = copy.deepcopy(metrics)
        wrong['statistics'][1]['data_failures'] = 1
        with self.assertRaises(ValueError):
            metrics_check(wrong, c, 'measurement')
        wrong = copy.deepcopy(metrics)
        op = wrong['statistics'][1]
        op['attempt_reasons'][1] = 0
        op['attempt_reasons'][9] = 2
        op['attempts'][1] = exact_histogram([])
        op['attempts'][2] = exact_histogram([20, 60])
        with self.assertRaises(ValueError):
            metrics_check(wrong, c, 'measurement')

    def report(self, c):
        metrics, _ = FailureAccounting().case()
        metrics['statistics'][1] = copy.deepcopy(metrics['statistics'][0])

        def phase(name, counts):
            result = copy.deepcopy(metrics)
            if name in ('warmup', 'measurement'):
                result['operations'] = ['get' if c.get('read_api') == 'point_get' else 'batch_get',
                                        'put' if c.get('write_api') == 'point_put' else 'batch_put']
            for op, n in zip(result['statistics'], counts):
                op['populations'][0].update(calls=n, input_items=n, completed_before_cutoff=n,
                                           whole_call=exact_histogram([10] * n),
                                           sdk_call=exact_histogram([9] * n))
                op['reasons'][0] = op['attempt_reasons'][0] = n
                op['attempts'][0] = exact_histogram([5] * n)
            return result

        build = dict(profile='debug', dirty=True)
        stat = '1 (fixture) S ' + ' '.join('1' if i == 18 else '0' for i in range(21))
        model = ('bounded_native_api_performance' if c['version'] == 3 else
                 'bounded_native_point_get_diagnostic' if c.get('read_api') == 'point_get' else
                 'bounded_native_batch_performance')
        measured = [1, 0] if c['read_percent'] == 100 else [0, 1]
        report = dict(version=c['version'], workload_model=model, full_history_recorded=False,
                      independently_checked=False, complete=True, failure=None, configuration=c,
                      config_sha256='0' * 64, build=build, build_sha256='0' * 64,
                      wire_sizes=config_check(c)[1], process_id=1, runtime_threads=2,
                      measurement_start_unix_ns=1, stop_reason='duration', measured_issued=1,
                      measured_completed=1, offered_slots=None, dropped_slots=0, task_failures=0,
                      cohort_elapsed_ns=1_000_000, completed_batches_per_second=1000.0,
                      successful_batches_per_second=1000.0, successful_input_items_per_second=1000.0,
                      timing_eligible=False, proc_stat_before=stat, proc_stat_after=stat,
                      stages=dict(initialization=dict(start_ns=0, end_ns=1000),
                                  warmup=dict(start_ns=1000, end_ns=1000),
                                  measurement=dict(start_ns=1000, end_ns=1_001_000),
                                  drain=dict(start_ns=1_001_000, end_ns=1_001_000),
                                  verification=dict(start_ns=1_001_000, end_ns=1_002_000)),
                      workers=[dict(worker=0, issued=1, dropped_slots=0, last_terminal_ns=10,
                                    stopped_ns=1_000_000, stopped_for_failure=False)],
                      metrics={name: phase(name, counts) for name, counts in
                               [('initialization', [3, 3]), ('warmup', [0, 0]),
                                ('measurement', measured), ('verification', [3, 0])]})
        return report, build

    def test_complete_v3_report_recomputes_model_size_and_selected_labels(self):
        c = dict(self.config(), read_percent=0)
        report, build = self.report(c)
        self.assertEqual(report_check(report, c, build)['calls'], 1)
        for name, value in [('version', 2), ('workload_model', 'bounded_native_batch_performance')]:
            altered = copy.deepcopy(report)
            altered[name] = value
            with self.assertRaises(ValueError):
                report_check(altered, c, build)
        altered = copy.deepcopy(report)
        altered['wire_sizes']['put_request_bytes'] = 169
        with self.assertRaisesRegex(ValueError, 'wire size'):
            report_check(altered, c, build)
        for phase in ('initialization', 'warmup', 'measurement', 'verification'):
            altered = copy.deepcopy(report)
            altered['metrics'][phase]['operations'] = (OPERATIONS if phase in ('warmup', 'measurement')
                                                       else ['get', 'put'])
            with self.assertRaisesRegex(ValueError, 'metric vocabulary'):
                report_check(altered, c, build)
        # Legacy reports retain their original model, metric and wire-size shape.
        for version, read in ((1, None), (2, 'batch_get'), (2, 'point_get')):
            legacy = ReadApiConfigurations().config()
            legacy['version'] = version
            if read is not None:
                legacy['read_api'] = read
            original, identity = self.report(legacy)
            self.assertEqual(report_check(original, legacy, identity)['calls'], 1)
            self.assertNotIn('write_api', original['configuration'])


class Histograms(unittest.TestCase):
    def test_independent_sorted_samples_and_exact_small_values(self):
        for values in ([], [0], [1], [63, 64, 65, 127, 128, 129], [1000] * 100 + [1_000_000], [(1 << 64) - 1]):
            histogram_check(exact_histogram(values))

    def test_bucket_partition_covers_unsigned_integer_space(self):
        next_value = 0
        for i in range(3776):
            lo, hi = bucket_bounds(i)
            self.assertEqual(lo, next_value)
            self.assertGreaterEqual(hi, lo)
            next_value = hi + 1
        self.assertEqual(next_value, 1 << 64)

    def test_sum_and_extrema_must_be_realizable_inside_buckets(self):
        original = exact_histogram([128, 129, 130, 131])
        for field, value in [('min_ns', 127), ('max_ns', 132), ('sum_ns', 4 * 128), ('sum_ns', 4 * 131)]:
            altered = copy.deepcopy(original)
            altered['raw'][field] = value
            if field == 'sum_ns':
                altered['mean_ns'] = value / 4
            with self.assertRaises(ValueError):
                histogram_check(altered)
        histogram_check(original)

    def test_boolean_quantile_and_counter_substitutions_are_rejected(self):
        original = exact_histogram([1])
        for path in [('raw', 'count'), ('raw', 'sum_ns'), ('p99', 'lower_ns')]:
            altered = copy.deepcopy(original)
            altered[path[0]][path[1]] = True
            with self.assertRaises(ValueError):
                histogram_check(altered)

    def test_duplicate_keys_and_nonfinite_json_are_rejected(self):
        for text in ('{"count":1,"count":2}', '{"count":NaN}', '{"count":Infinity}'):
            with self.assertRaises(ValueError):
                strict_json(text)

    def test_paired_latencies_require_distribution_containment(self):
        a, b = exact_histogram([10, 20, 50]), exact_histogram([10, 30, 40])
        histogram_check(a)
        histogram_check(b)
        # Equal counts/sums and compatible extrema alone cannot pair these
        # samples such that every containing duration is at least its child.
        with self.assertRaises(ValueError):
            dominance_check(a['raw'], b['raw'])
        dominance_check(exact_histogram([10, 30, 50])['raw'], b['raw'])
        dominance_check(exact_histogram([])['raw'], exact_histogram([])['raw'])

    def test_exact_extrema_detect_containment_failures_inside_shared_buckets(self):
        for whole, scheduled in (([129, 130, 132], [128, 131, 132]),
                                 ([128, 130, 133], [129, 130, 132])):
            a, b = exact_histogram(scheduled), exact_histogram(whole)
            histogram_check(a)
            histogram_check(b)
            self.assertEqual(a['raw']['buckets'], b['raw']['buckets'])
            self.assertEqual(a['raw']['sum_ns'], b['raw']['sum_ns'])
            with self.assertRaises(ValueError):
                dominance_check(a['raw'], b['raw'])
            dominance_check(b['raw'], b['raw'])


class FailureAccounting(unittest.TestCase):
    def case(self):
        def population():
            return dict(calls=0, input_items=0, completed_before_cutoff=0,
                        whole_call=exact_histogram([]), sdk_call=exact_histogram([]),
                        scheduled_to_completion=exact_histogram([]))
        def operation():
            return dict(populations=[population() for _ in OUTCOMES], reasons=[0] * 12,
                        rpc_codes=[0] * 17, attempt_reasons=[0] * 12, attempt_rpc_codes=[0] * 17,
                        attempts=[exact_histogram([]) for _ in range(3)],
                        dispatch_lateness=exact_histogram([]), data_failures=0)
        m = dict(operations=OPERATIONS, outcomes=OUTCOMES, reasons=REASONS,
                 attempt_outcomes=['success', 'refused', 'failed_or_unknown'],
                 histogram_subdivisions=64, valid=True, statistics=[operation(), operation()])
        # One uncertain atomic write of 16 items. A NotLeader refusal precedes
        # its only possibly effectful attempt, which times out after the cutoff.
        op = m['statistics'][1]
        op['populations'][2].update(calls=1, input_items=16,
                                    whole_call=exact_histogram([110]), sdk_call=exact_histogram([100]))
        op['reasons'][9] = 1
        op['attempt_reasons'][1] = op['attempt_reasons'][9] = 1
        op['attempts'][1] = exact_histogram([20])
        op['attempts'][2] = exact_histogram([60])
        c = dict(batch_size=16, load=dict(kind='closed_loop'), client=dict(max_attempts=2))
        return m, c

    def test_uncertain_batch_and_safe_retry_keep_whole_populations(self):
        m, c = self.case()
        result = metrics_check(m, c, 'measurement')
        self.assertEqual(result, dict(calls=[0, 1], successes=0, successful_items=0, attempts=2))

    def test_omitted_failed_latency_or_item_is_rejected(self):
        for field, value in [('whole_call', exact_histogram([])), ('input_items', 15),
                             ('sdk_call', exact_histogram([]))]:
            m, c = self.case()
            m['statistics'][1]['populations'][2][field] = value
            with self.assertRaises(ValueError):
                metrics_check(m, c, 'measurement')

    def test_uncertain_retry_is_rejected_even_with_balanced_attempt_histograms(self):
        m, c = self.case()
        # Replace the safe refusal with an earlier deadline. Keep the total
        # attempt count and durations unchanged; population balance alone is weak.
        op = m['statistics'][1]
        op['attempt_reasons'][1] = 0
        op['attempt_reasons'][9] = 2
        op['attempts'][1] = exact_histogram([])
        op['attempts'][2] = exact_histogram([20, 60])
        with self.assertRaises(ValueError):
            metrics_check(m, c, 'measurement')

    def test_fixed_rate_failure_keeps_dispatch_time(self):
        m, c = self.case()
        c['load'] = dict(kind='fixed_rate', batches_per_second=10)
        op = m['statistics'][1]
        op['populations'][2]['scheduled_to_completion'] = exact_histogram([150])
        op['dispatch_lateness'] = exact_histogram([40])
        metrics_check(m, c, 'measurement')
        op['populations'][2]['scheduled_to_completion'] = exact_histogram([110])
        with self.assertRaises(ValueError):
            metrics_check(m, c, 'measurement')


if __name__ == '__main__':
    unittest.main()
