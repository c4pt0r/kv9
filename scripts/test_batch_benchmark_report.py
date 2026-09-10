"""Independent arithmetic examples and failure-population controls."""
import copy
import unittest

from batch_benchmark_report import (OPERATIONS, OUTCOMES, REASONS, bucket_bounds,
                                    histogram_check, metrics_check, strict_json)


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
