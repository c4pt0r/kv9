"""Independent report controls for exclusive proposal refusals and legacy evidence."""
import copy
import unittest

from workload_report import (LEGACY_POPULATIONS, OPERATIONS, OUTCOMES, PHASES,
                             POPULATIONS, metrics_check, reason_population)


def empty_metrics(populations):
    def latency():
        return dict(valid=True, outcomes=[dict(
            outcome=outcome, buckets=[0] * 65, count=0, sum_ns=0,
            min_ns=None, max_ns=None, count_saturated=False, sum_saturated=False,
            duration_clamped=False, p50=None, p95=None, p99=None) for outcome in OUTCOMES])
    result = dict(populations=list(populations), phases=PHASES, operations=OPERATIONS)
    for level in ('logical', 'attempt'):
        result[f'{level}_counts'] = [[[0] * len(populations) for _ in OPERATIONS] for _ in PHASES]
        result[f'{level}_rpc_codes'] = [[[0] * 17 for _ in OPERATIONS] for _ in PHASES]
        result[f'{level}_latency'] = [[latency() for _ in OPERATIONS] for _ in PHASES]
    return result


def add_refusal(metrics, op=1):
    for level in ('logical', 'attempt'):
        metrics[f'{level}_counts'][2][op][12] = 1
        h = metrics[f'{level}_latency'][2][op]['outcomes'][5]
        h.update(count=1, sum_ns=1, min_ns=1, max_ns=1,
                 **{f'p{p}': dict(lower_ns=1, upper_ns=1) for p in (50, 95, 99)})
        h['buckets'][1] = 1


class ProposalReportTests(unittest.TestCase):
    def test_old_evidence_requires_its_exact_width_and_vocabulary(self):
        for vocabulary in (LEGACY_POPULATIONS, POPULATIONS):
            metrics = empty_metrics(vocabulary)
            metrics_check(metrics)
            for field in ('logical_counts', 'attempt_counts'):
                wrong_width = copy.deepcopy(metrics)
                wrong_width[field][2][1].append(0)
                with self.assertRaises(ValueError):
                    metrics_check(wrong_width)
        for vocabulary in (POPULATIONS[::-1], POPULATIONS + ['future']):
            with self.assertRaisesRegex(ValueError, 'vocabulary'):
                metrics_check(empty_metrics(vocabulary))

    def test_refusal_counts_must_have_rejected_latency_and_cannot_be_reads(self):
        metrics = empty_metrics(POPULATIONS)
        add_refusal(metrics)
        metrics_check(metrics)
        for level in ('logical', 'attempt'):
            unknown = copy.deepcopy(metrics)
            outcomes = unknown[f'{level}_latency'][2][1]['outcomes']
            outcomes[5], outcomes[6] = outcomes[6], outcomes[5]
            outcomes[5]['outcome'], outcomes[6]['outcome'] = 'rejected', 'unconfirmed'
            with self.assertRaisesRegex(ValueError, 'counts disagree'):
                metrics_check(unknown)
        read = empty_metrics(POPULATIONS)
        add_refusal(read, op=0)
        with self.assertRaisesRegex(ValueError, 'read cannot'):
            metrics_check(read)

    def test_reason_requires_exact_payload_and_declared_vocabulary(self):
        for reason in ('request_count', 'encoded_bytes', 'request_too_large', 'expired', 'stopped'):
            value = dict(kind='proposal_refused', reason=reason)
            self.assertEqual(reason_population(value), 12)
            with self.assertRaises(ValueError):
                reason_population(value, LEGACY_POPULATIONS)
        for value in (dict(kind='proposal_refused'), dict(kind='proposal_refused', reason='future'),
                      dict(kind='proposal_refused', reason='expired', code=8),
                      dict(kind='proposal_refused', reason=[])):
            with self.assertRaises(ValueError):
                reason_population(value)


if __name__ == '__main__':
    unittest.main()
