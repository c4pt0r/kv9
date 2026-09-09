use std::sync::Mutex;
use std::time::Duration;

use kv9_common::metrics::{Latency, LatencySnapshot, Outcome as MetricOutcome};
use serde::Serialize;

use crate::client::{CallReport, OperationKind, Outcome, Reason};

use super::recorder::PHASES;

const POPULATIONS: [&str; 12] = [
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

fn population(reason: Option<&Reason>) -> usize {
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

fn reason(outcome: &Outcome) -> Option<&Reason> {
    match outcome {
        Outcome::Success { .. } => None,
        Outcome::Refused { reason }
        | Outcome::UnknownWrite { reason }
        | Outcome::ReadFailure { reason }
        | Outcome::ClientRejected { reason } => Some(reason),
    }
}

fn attempt_outcome(kind: OperationKind, reason: Option<&Reason>) -> MetricOutcome {
    match reason {
        None => MetricOutcome::Success,
        Some(
            Reason::NotLeader { .. }
            | Reason::AdmissionCount
            | Reason::AdmissionBytes
            | Reason::AdmissionOversize,
        ) => MetricOutcome::Rejected,
        _ if kind == OperationKind::Get => MetricOutcome::Error,
        _ => MetricOutcome::Unconfirmed,
    }
}

struct State {
    logical_counts: [[[u64; POPULATIONS.len()]; 3]; PHASES.len()],
    attempt_counts: [[[u64; POPULATIONS.len()]; 3]; PHASES.len()],
    logical_rpc_codes: [[[u64; 17]; 3]; PHASES.len()],
    attempt_rpc_codes: [[[u64; 17]; 3]; PHASES.len()],
    // Exactly nineteen phase rows, allocated once on the heap. Separating
    // warmup/setup from measurement also applies to latency populations.
    logical: Vec<[Latency; 3]>,
    attempts: Vec<[Latency; 3]>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            logical_counts: [[[0; POPULATIONS.len()]; 3]; PHASES.len()],
            attempt_counts: [[[0; POPULATIONS.len()]; 3]; PHASES.len()],
            logical_rpc_codes: [[[0; 17]; 3]; PHASES.len()],
            attempt_rpc_codes: [[[0; 17]; 3]; PHASES.len()],
            logical: PHASES
                .iter()
                .map(|_| std::array::from_fn(|_| Latency::default()))
                .collect(),
            attempts: PHASES
                .iter()
                .map(|_| std::array::from_fn(|_| Latency::default()))
                .collect(),
        }
    }
}

#[derive(Default)]
pub struct Metrics(Mutex<State>);

#[derive(Serialize)]
pub struct MetricsSnapshot {
    pub populations: [&'static str; POPULATIONS.len()],
    pub phases: [&'static str; PHASES.len()],
    pub operations: [OperationKind; 3],
    pub logical_counts: [[[u64; POPULATIONS.len()]; 3]; PHASES.len()],
    pub attempt_counts: [[[u64; POPULATIONS.len()]; 3]; PHASES.len()],
    pub logical_rpc_codes: [[[u64; 17]; 3]; PHASES.len()],
    pub attempt_rpc_codes: [[[u64; 17]; 3]; PHASES.len()],
    pub logical_latency: Vec<[LatencySnapshot; 3]>,
    pub attempt_latency: Vec<[LatencySnapshot; 3]>,
}

impl Metrics {
    pub fn record(&self, phase: &str, report: &CallReport) -> Result<(), &'static str> {
        fn rpc_code(reason: Option<&Reason>) -> Result<Option<usize>, &'static str> {
            match reason {
                Some(Reason::RpcStatus { code }) if (0..17).contains(code) => {
                    Ok(Some(*code as usize))
                }
                Some(Reason::RpcStatus { .. }) => Err("invalid RPC code in workload report"),
                _ => Ok(None),
            }
        }
        if report.attempts.len() > crate::client::MAX_ATTEMPTS {
            return Err("attempt population exceeds the client bound");
        }
        let logical_code = rpc_code(reason(&report.outcome))?;
        // Bound and validate every code before any metric changes.
        for attempt in &report.attempts {
            rpc_code(attempt.failure.as_ref())?;
        }
        let phase = PHASES
            .iter()
            .position(|known| *known == phase)
            .ok_or("unknown metrics phase")?;
        let mut state = self
            .0
            .lock()
            .map_err(|_| "workload metrics lock poisoned")?;
        let kind = report.operation as usize;
        let outcome = match &report.outcome {
            Outcome::Success { .. } => MetricOutcome::Success,
            Outcome::Refused { .. } => MetricOutcome::Rejected,
            Outcome::UnknownWrite { .. } => MetricOutcome::Unconfirmed,
            Outcome::ReadFailure { .. } => MetricOutcome::Error,
            Outcome::ClientRejected { .. } => MetricOutcome::Released,
        };
        state.logical_counts[phase][kind][population(reason(&report.outcome))] += 1;
        if let Some(code) = logical_code {
            state.logical_rpc_codes[phase][kind][code] += 1;
        }
        state.logical[phase][kind].record(Duration::from_nanos(report.elapsed_ns), outcome);
        for attempt in &report.attempts {
            if let Some(code) = rpc_code(attempt.failure.as_ref())? {
                state.attempt_rpc_codes[phase][kind][code] += 1;
            }
            state.attempt_counts[phase][kind][population(attempt.failure.as_ref())] += 1;
            state.attempts[phase][kind].record(
                Duration::from_nanos(attempt.elapsed_ns),
                attempt_outcome(report.operation, attempt.failure.as_ref()),
            );
        }
        Ok(())
    }

    pub fn snapshot(&self) -> Result<MetricsSnapshot, &'static str> {
        let state = self
            .0
            .lock()
            .map_err(|_| "workload metrics lock poisoned")?;
        Ok(MetricsSnapshot {
            populations: POPULATIONS,
            phases: PHASES,
            operations: [
                OperationKind::Get,
                OperationKind::Put,
                OperationKind::Delete,
            ],
            logical_counts: state.logical_counts,
            attempt_counts: state.attempt_counts,
            logical_rpc_codes: state.logical_rpc_codes,
            attempt_rpc_codes: state.attempt_rpc_codes,
            logical_latency: state
                .logical
                .iter()
                .map(|row| std::array::from_fn(|i| row[i].snapshot()))
                .collect(),
            attempt_latency: state
                .attempts
                .iter()
                .map(|row| std::array::from_fn(|i| row[i].snapshot()))
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{Attempt, Stop, Value};

    #[test]
    fn refused_and_unknown_calls_remain_in_counts_and_latency_populations() {
        let metrics = Metrics::default();
        for (outcome, reason) in [
            (
                Outcome::Success {
                    value: Value::Applied { term: 7, index: 11 },
                },
                None,
            ),
            (
                Outcome::Refused {
                    reason: Reason::AdmissionCount,
                },
                Some(Reason::AdmissionCount),
            ),
            (
                Outcome::UnknownWrite {
                    reason: Reason::Deadline,
                },
                Some(Reason::Deadline),
            ),
            (
                Outcome::UnknownWrite {
                    reason: Reason::RpcStatus { code: 13 },
                },
                Some(Reason::RpcStatus { code: 13 }),
            ),
            (
                Outcome::UnknownWrite {
                    reason: Reason::RpcStatus { code: 14 },
                },
                Some(Reason::RpcStatus { code: 14 }),
            ),
        ] {
            let report = CallReport {
                operation: OperationKind::Put,
                elapsed_ns: 100,
                attempts: vec![Attempt {
                    ordinal: 1,
                    node_id: 1,
                    elapsed_ns: 80,
                    failure: reason,
                }],
                stop: Stop::Terminal,
                outcome,
            };
            metrics.record("measure", &report).unwrap();
            let mut warmup = report.clone();
            warmup.elapsed_ns = 10_000;
            metrics.record("warmup", &warmup).unwrap();
        }
        let snapshot = metrics.snapshot().unwrap();
        assert_eq!(snapshot.logical_counts[2][1].iter().sum::<u64>(), 5);
        assert_eq!(snapshot.attempt_counts[2][1].iter().sum::<u64>(), 5);
        assert_eq!(snapshot.logical_rpc_codes[2][1][13], 1);
        assert_eq!(snapshot.logical_rpc_codes[2][1][14], 1);
        assert_eq!(snapshot.attempt_rpc_codes[2][1][13], 1);
        assert_eq!(snapshot.attempt_rpc_codes[2][1][14], 1);
        assert_eq!(
            snapshot.logical_latency[2][1]
                .outcomes
                .iter()
                .map(|h| h.count)
                .sum::<u64>(),
            5
        );
        assert_eq!(
            snapshot.logical_latency[2][1]
                .outcomes
                .iter()
                .map(|h| h.sum_ns)
                .sum::<u64>(),
            500
        );
        assert_eq!(
            snapshot.attempt_latency[2][1]
                .outcomes
                .iter()
                .map(|h| h.sum_ns)
                .sum::<u64>(),
            400
        );
        assert_eq!(snapshot.logical_counts[2][1][9], 1);
        assert_eq!(snapshot.logical_counts[2][1][2], 1);
    }
}
