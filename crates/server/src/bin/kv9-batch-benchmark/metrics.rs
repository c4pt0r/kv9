use super::model::Histogram;
use kv9_server::client::{CallReport, OperationKind, Outcome, Reason};
use serde::Serialize;
use serde_json::{json, Value};

pub const OUTCOMES: [&str; 5] = [
    "success",
    "refused",
    "unknown_write",
    "read_failure",
    "client_rejected",
];
pub const REASONS: [&str; 12] = [
    "success",
    "not_leader",
    "admission_count",
    "admission_bytes",
    "admission_oversize",
    "read_quorum_unconfirmed",
    "read_apply_unconfirmed",
    "rpc_status",
    "protocol",
    "deadline",
    "client_capacity",
    "client_input",
];

fn reason_index(reason: Option<&Reason>) -> usize {
    match reason {
        None => 0,
        Some(Reason::NotLeader { .. }) => 1,
        Some(Reason::AdmissionCount) => 2,
        Some(Reason::AdmissionBytes) => 3,
        Some(Reason::AdmissionOversize) => 4,
        Some(Reason::ReadQuorumUnconfirmed) => 5,
        Some(Reason::ReadApplyUnconfirmed) => 6,
        Some(Reason::RpcStatus { .. }) => 7,
        Some(Reason::Protocol) => 8,
        Some(Reason::Deadline) => 9,
        Some(Reason::ClientCapacity) => 10,
        Some(Reason::ClientInput) => 11,
    }
}

#[derive(Clone, Default, Serialize)]
pub struct Population {
    pub calls: u64,
    pub input_items: u64,
    pub completed_before_cutoff: u64,
    pub whole_call: Histogram,
    pub sdk_call: Histogram,
    pub scheduled_to_completion: Histogram,
}

#[derive(Clone, Serialize)]
pub struct OperationMetrics {
    pub populations: [Population; 5],
    pub reasons: [u64; 12],
    pub rpc_codes: [u64; 17],
    pub attempt_reasons: [u64; 12],
    pub attempt_rpc_codes: [u64; 17],
    pub attempts: [Histogram; 3],
    pub dispatch_lateness: Histogram,
    pub data_failures: u64,
}

impl Default for OperationMetrics {
    fn default() -> Self {
        Self {
            populations: std::array::from_fn(|_| Population::default()),
            reasons: [0; 12],
            rpc_codes: [0; 17],
            attempt_reasons: [0; 12],
            attempt_rpc_codes: [0; 17],
            attempts: std::array::from_fn(|_| Histogram::default()),
            dispatch_lateness: Histogram::default(),
            data_failures: 0,
        }
    }
}

#[derive(Clone, Default, Serialize)]
pub struct Metrics {
    pub operations: [OperationMetrics; 2],
}

pub struct Sample<'a> {
    pub report: &'a CallReport,
    pub items: usize,
    /// Includes input construction, SDK call and response-content validation.
    pub whole_call_ns: u64,
    pub before_cutoff: bool,
    pub scheduled_to_completion_ns: Option<u64>,
    pub dispatch_lateness_ns: Option<u64>,
    pub data_valid: bool,
}

impl Metrics {
    pub fn record(&mut self, sample: Sample<'_>) {
        let report = sample.report;
        let kind = match report.operation {
            OperationKind::BatchGet => 0,
            OperationKind::BatchPut => 1,
            OperationKind::Get | OperationKind::Put | OperationKind::Delete => {
                panic!("point operation in batch benchmark")
            }
        };
        let (outcome, reason) = match &report.outcome {
            Outcome::Success { .. } => (0, None),
            Outcome::Refused { reason } => (1, Some(reason)),
            Outcome::UnknownWrite { reason } => (2, Some(reason)),
            Outcome::ReadFailure { reason } => (3, Some(reason)),
            Outcome::ClientRejected { reason } => (4, Some(reason)),
        };
        let op = &mut self.operations[kind];
        let population = &mut op.populations[outcome];
        population.calls += 1;
        population.input_items += sample.items as u64;
        population.completed_before_cutoff += u64::from(sample.before_cutoff);
        population.whole_call.add(sample.whole_call_ns);
        population.sdk_call.add(report.elapsed_ns);
        if let Some(nanos) = sample.scheduled_to_completion_ns {
            population.scheduled_to_completion.add(nanos);
        }
        if let Some(nanos) = sample.dispatch_lateness_ns {
            op.dispatch_lateness.add(nanos);
        }
        op.reasons[reason_index(reason)] += 1;
        if let Some(Reason::RpcStatus { code }) = reason {
            if let Some(count) = usize::try_from(*code)
                .ok()
                .and_then(|i| op.rpc_codes.get_mut(i))
            {
                *count += 1;
            } else {
                op.data_failures += 1;
            }
        }
        for attempt in &report.attempts {
            let reason = attempt.failure.as_ref();
            let outcome = match reason {
                None => 0,
                Some(
                    Reason::NotLeader { .. }
                    | Reason::AdmissionCount
                    | Reason::AdmissionBytes
                    | Reason::AdmissionOversize,
                ) => 1,
                _ => 2,
            };
            op.attempts[outcome].add(attempt.elapsed_ns);
            op.attempt_reasons[reason_index(reason)] += 1;
            if let Some(Reason::RpcStatus { code }) = reason {
                if let Some(count) = usize::try_from(*code)
                    .ok()
                    .and_then(|i| op.attempt_rpc_codes.get_mut(i))
                {
                    *count += 1;
                } else {
                    op.data_failures += 1;
                }
            }
        }
        op.data_failures += u64::from(!sample.data_valid);
    }

    pub fn merge(&mut self, other: &Self) {
        fn counters<const N: usize>(a: &mut [u64; N], b: &[u64; N]) {
            for (a, b) in a.iter_mut().zip(b) {
                *a += b;
            }
        }
        for (a, b) in self.operations.iter_mut().zip(&other.operations) {
            for (a, b) in a.populations.iter_mut().zip(&b.populations) {
                a.calls += b.calls;
                a.input_items += b.input_items;
                a.completed_before_cutoff += b.completed_before_cutoff;
                a.whole_call.merge(&b.whole_call);
                a.sdk_call.merge(&b.sdk_call);
                a.scheduled_to_completion.merge(&b.scheduled_to_completion);
            }
            counters(&mut a.reasons, &b.reasons);
            counters(&mut a.rpc_codes, &b.rpc_codes);
            counters(&mut a.attempt_reasons, &b.attempt_reasons);
            counters(&mut a.attempt_rpc_codes, &b.attempt_rpc_codes);
            for (a, b) in a.attempts.iter_mut().zip(&b.attempts) {
                a.merge(b);
            }
            a.dispatch_lateness.merge(&b.dispatch_lateness);
            a.data_failures += b.data_failures;
        }
    }

    pub fn calls(&self) -> u64 {
        self.operations
            .iter()
            .flat_map(|op| &op.populations)
            .map(|p| p.calls)
            .sum()
    }

    pub fn data_failures(&self) -> u64 {
        self.operations.iter().map(|op| op.data_failures).sum()
    }

    pub fn valid(&self) -> bool {
        self.operations.iter().all(|op| {
            op.dispatch_lateness.valid
                && op.attempts.iter().all(|h| h.valid)
                && op.populations.iter().all(|p| {
                    p.whole_call.valid && p.sdk_call.valid && p.scheduled_to_completion.valid
                })
        })
    }

    pub fn report(&self) -> Value {
        fn histogram(h: &Histogram) -> Value {
            json!({"raw": h, "mean_ns": if h.valid && h.count > 0 { Some(h.sum_ns as f64 / h.count as f64) } else { None },
                "p50": h.quantile(50), "p95": h.quantile(95), "p99": h.quantile(99)})
        }
        json!({"operations": ["batch_get", "batch_put"], "outcomes": OUTCOMES,
            "reasons": REASONS, "attempt_outcomes": ["success", "refused", "failed_or_unknown"],
            "histogram_subdivisions": 64, "valid": self.valid(),
            "statistics": self.operations.iter().map(|op| json!({
                "populations": op.populations.iter().map(|p| json!({"calls":p.calls,"input_items":p.input_items,
                    "completed_before_cutoff":p.completed_before_cutoff,"whole_call":histogram(&p.whole_call),
                    "sdk_call":histogram(&p.sdk_call),"scheduled_to_completion":histogram(&p.scheduled_to_completion)})).collect::<Vec<_>>(),
                "reasons":op.reasons,"rpc_codes":op.rpc_codes,"attempt_reasons":op.attempt_reasons,
                "attempt_rpc_codes":op.attempt_rpc_codes,"attempts":op.attempts.iter().map(histogram).collect::<Vec<_>>(),
                "dispatch_lateness":histogram(&op.dispatch_lateness),"data_failures":op.data_failures,
            })).collect::<Vec<_>>()})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kv9_server::client::{Attempt, Stop, Value as ClientValue};

    #[test]
    fn unknown_whole_batches_and_retry_attempts_keep_separate_populations() {
        let mut metrics = Metrics::default();
        let report = CallReport {
            operation: OperationKind::BatchPut,
            elapsed_ns: 70,
            attempts: vec![
                Attempt {
                    ordinal: 1,
                    node_id: 1,
                    elapsed_ns: 20,
                    failure: Some(Reason::NotLeader { leader: Some(2) }),
                },
                Attempt {
                    ordinal: 2,
                    node_id: 2,
                    elapsed_ns: 40,
                    failure: Some(Reason::Deadline),
                },
            ],
            stop: Stop::Terminal,
            outcome: Outcome::UnknownWrite {
                reason: Reason::Deadline,
            },
        };
        metrics.record(Sample {
            report: &report,
            items: 16,
            whole_call_ns: 80,
            before_cutoff: false,
            scheduled_to_completion_ns: Some(95),
            dispatch_lateness_ns: Some(15),
            data_valid: true,
        });
        let p = &metrics.operations[1].populations[2];
        assert_eq!(
            (p.calls, p.input_items, p.completed_before_cutoff),
            (1, 16, 0)
        );
        assert_eq!(
            (
                p.whole_call.sum_ns,
                p.sdk_call.sum_ns,
                p.scheduled_to_completion.sum_ns
            ),
            (80, 70, 95)
        );
        assert_eq!(metrics.operations[1].attempts[0].count, 0);
        assert_eq!(metrics.operations[1].attempts[1].count, 1);
        assert_eq!(metrics.operations[1].attempts[2].count, 1);
        let mut success = report.clone();
        success.outcome = Outcome::Success {
            value: ClientValue::Applied { term: 2, index: 9 },
        };
        success.attempts[1].failure = None;
        let mut other = Metrics::default();
        other.record(Sample {
            report: &success,
            items: 16,
            whole_call_ns: 90,
            before_cutoff: true,
            scheduled_to_completion_ns: None,
            dispatch_lateness_ns: None,
            data_valid: true,
        });
        metrics.merge(&other);
        assert_eq!(metrics.calls(), 2);
        assert_eq!(metrics.operations[1].populations[0].input_items, 16);
        assert_eq!(metrics.operations[1].populations[2].input_items, 16);
        assert_eq!(
            metrics.operations[1]
                .attempts
                .iter()
                .map(|h| h.count)
                .sum::<u64>(),
            4
        );
    }
}
