//! Opt-in, bounded observations of the existing write path. These values never
//! decide admission, commitment, application, receipt identity or deadlines.
//! Recording allocates nothing; only snapshots allocate. Observer locks are
//! leaves, and poisoning invalidates observations without failing database work.

use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;

const BUCKETS: usize = 65;
const JOINT_BUCKETS: usize = 9;

fn add(value: &mut u64, amount: u64, saturated: &mut bool) {
    let (sum, overflow) = value.overflowing_add(amount);
    *value = if overflow { u64::MAX } else { sum };
    *saturated |= overflow;
}

#[derive(Clone)]
pub(crate) struct Distribution {
    buckets: [u64; BUCKETS],
    pub(crate) count: u64,
    sum: u64,
    max: u64,
    saturated: bool,
}

impl Default for Distribution {
    fn default() -> Self {
        Self {
            buckets: [0; BUCKETS],
            count: 0,
            sum: 0,
            max: 0,
            saturated: false,
        }
    }
}

impl Distribution {
    pub(crate) fn record(&mut self, value: u64) {
        let bucket = (u64::BITS - value.leading_zeros()) as usize;
        add(&mut self.buckets[bucket], 1, &mut self.saturated);
        add(&mut self.count, 1, &mut self.saturated);
        add(&mut self.sum, value, &mut self.saturated);
        self.max = self.max.max(value);
    }

    pub(crate) fn record_duration(&mut self, duration: Duration) {
        let nanos = duration.as_nanos();
        self.saturated |= nanos > u128::from(u64::MAX);
        self.record(nanos.min(u128::from(u64::MAX)) as u64);
    }

    fn merge(&mut self, other: &Self) {
        self.saturated |= other.saturated;
        if other.count == 0 {
            return;
        }
        for (into, amount) in self.buckets.iter_mut().zip(other.buckets) {
            add(into, amount, &mut self.saturated);
        }
        add(&mut self.count, other.count, &mut self.saturated);
        add(&mut self.sum, other.sum, &mut self.saturated);
        self.max = self.max.max(other.max);
    }

    fn snapshot(&self, name: &'static str, unit: &'static str) -> DistributionSnapshot {
        DistributionSnapshot {
            name,
            unit,
            buckets: self.buckets.to_vec(),
            count: self.count,
            sum: self.sum,
            max: (self.count != 0).then_some(self.max),
            saturated: self.saturated,
        }
    }
}

#[derive(Serialize)]
pub struct DistributionSnapshot {
    pub name: &'static str,
    pub unit: &'static str,
    /// Zero is its own bucket; bucket i>0 is [2^(i-1), 2^i-1].
    pub buckets: Vec<u64>,
    pub count: u64,
    pub sum: u64,
    pub max: Option<u64>,
    pub saturated: bool,
}

/// One owner service pass, including a pass with no queued requests. A request
/// inspected repeatedly contributes repeatedly to inspection_age_ns. Ages start
/// at registration after proposal, not at client invocation or reservation.
#[derive(Default)]
pub(crate) struct ServiceObservation {
    pub extracted: usize,
    pub inspected: usize,
    pub resolved: usize,
    pub canceled: usize,
    pub expired: usize,
    pub pending: usize,
    pub closed: usize,
    pub inspection_age: Distribution,
    pub resolved_age: Distribution,
}

/// Partial successful groups remain observable when a later group fails. A
/// group's samples are recorded only after its apply call returned success.
#[derive(Default)]
pub(crate) struct PumpObservation {
    pub committed_taken: usize,
    pub applied_commands: usize,
    pub group_commands: Distribution,
    pub group_bytes: Distribution,
}

const METRICS: [(&str, &str); 18] = [
    ("committed_entries_taken_per_pump", "entries"),
    ("applied_commands_per_pump", "commands"),
    ("successful_apply_group_commands", "commands"),
    ("successful_apply_group_encoded_bytes", "bytes"),
    ("async_requests_extracted_per_service", "requests"),
    ("async_requests_inspected_per_service", "requests"),
    ("async_requests_resolved_per_service", "requests"),
    ("async_requests_canceled_per_service", "requests"),
    ("async_requests_expired_per_service", "requests"),
    ("async_requests_requeued_per_service", "requests"),
    ("async_requests_closed_during_service", "requests"),
    ("async_request_inspection_age", "nanoseconds"),
    ("async_request_resolved_age", "nanoseconds"),
    ("receipt_ring_length_at_lookup", "receipts"),
    ("receipt_linear_probes_per_lookup", "receipts"),
    ("receipt_hit_slots_behind_tail", "receipts"),
    ("receipt_hit_index_distance_from_tail", "log_indexes"),
    ("receipt_upper_bound_skipped_ring_length", "receipts"),
];

#[derive(Clone)]
struct State {
    valid: bool,
    saturated: bool,
    distributions: [Distribution; METRICS.len()],
    successful_pumps: u64,
    failed_pumps: u64,
    lookup_hits: u64,
    lookup_misses: u64,
    /// Rows are applied command counts, columns are resolved waiter counts.
    /// Log2 buckets, with the final bucket containing all values >=128.
    applied_vs_resolved: [[u64; JOINT_BUCKETS]; JOINT_BUCKETS],
}

impl Default for State {
    fn default() -> Self {
        Self {
            valid: true,
            saturated: false,
            distributions: std::array::from_fn(|_| Distribution::default()),
            successful_pumps: 0,
            failed_pumps: 0,
            lookup_hits: 0,
            lookup_misses: 0,
            applied_vs_resolved: [[0; JOINT_BUCKETS]; JOINT_BUCKETS],
        }
    }
}

#[derive(Default)]
pub(crate) struct WriteDiagnostics(Mutex<State>);

impl WriteDiagnostics {
    fn update(&self, record: impl FnOnce(&mut State)) {
        let mut state = self.0.lock().unwrap_or_else(|error| {
            let mut state = error.into_inner();
            state.valid = false;
            state
        });
        record(&mut state);
    }

    pub(crate) fn record_pump(&self, pump: &PumpObservation, service: Option<&ServiceObservation>) {
        self.update(|state| {
            state.distributions[0].record(pump.committed_taken as u64);
            state.distributions[1].record(pump.applied_commands as u64);
            state.distributions[2].merge(&pump.group_commands);
            state.distributions[3].merge(&pump.group_bytes);
            if let Some(service) = service {
                add(&mut state.successful_pumps, 1, &mut state.saturated);
                for (metric, value) in state.distributions[4..11].iter_mut().zip([
                    service.extracted,
                    service.inspected,
                    service.resolved,
                    service.canceled,
                    service.expired,
                    service.pending,
                    service.closed,
                ]) {
                    metric.record(value as u64);
                }
                state.distributions[11].merge(&service.inspection_age);
                state.distributions[12].merge(&service.resolved_age);
                let joint_bucket = |value: usize| {
                    ((usize::BITS - value.leading_zeros()) as usize).min(JOINT_BUCKETS - 1)
                };
                add(
                    &mut state.applied_vs_resolved[joint_bucket(pump.applied_commands)]
                        [joint_bucket(service.resolved)],
                    1,
                    &mut state.saturated,
                );
            } else {
                add(&mut state.failed_pumps, 1, &mut state.saturated);
            }
        });
    }

    /// Count actual fallback linear probes, or zero probes for a bound rejection.
    /// The skipped ring length makes the absent scan explicit and accountable.
    pub(crate) fn record_bounded_lookup(
        &self,
        len: usize,
        slot: Option<usize>,
        tail_index: Option<u64>,
        requested: u64,
        upper_bound_miss: bool,
    ) {
        self.update(|state| {
            state.distributions[13].record(len as u64);
            if upper_bound_miss {
                state.distributions[14].record(0);
                state.distributions[17].record(len as u64);
            } else {
                state.distributions[14].record(slot.map_or(len, |slot| slot + 1) as u64);
            }
            if let Some(slot) = slot {
                add(&mut state.lookup_hits, 1, &mut state.saturated);
                state.distributions[15].record((len - 1 - slot) as u64);
                if let Some(distance) = tail_index.and_then(|tail| tail.checked_sub(requested)) {
                    state.distributions[16].record(distance);
                }
            } else {
                add(&mut state.lookup_misses, 1, &mut state.saturated);
            }
        });
    }

    pub(crate) fn snapshot(&self) -> DriverSnapshot {
        // Clone fixed arrays under the leaf lock; allocate only after release.
        let state = self
            .0
            .lock()
            .map(|state| state.clone())
            .unwrap_or_else(|e| {
                let mut state = e.into_inner().clone();
                state.valid = false;
                state
            });
        DriverSnapshot {
            valid: state.valid,
            saturated: state.saturated,
            lookup_algorithm: "upper_bound_then_first_match_linear_scan",
            successful_pumps: state.successful_pumps,
            failed_pumps: state.failed_pumps,
            lookup_hits: state.lookup_hits,
            lookup_misses: state.lookup_misses,
            distributions: state
                .distributions
                .iter()
                .zip(METRICS)
                .map(|(metric, (name, unit))| metric.snapshot(name, unit))
                .collect(),
            applied_vs_resolved: state.applied_vs_resolved,
        }
    }
}

#[derive(Serialize)]
pub struct DriverSnapshot {
    pub valid: bool,
    pub saturated: bool,
    pub lookup_algorithm: &'static str,
    pub successful_pumps: u64,
    pub failed_pumps: u64,
    pub lookup_hits: u64,
    pub lookup_misses: u64,
    pub distributions: Vec<DistributionSnapshot>,
    pub applied_vs_resolved: [[u64; JOINT_BUCKETS]; JOINT_BUCKETS],
}

/// Owned by PeerInner and recorded under its existing lock. A Ready sample is
/// taken only after Ready and LightReady persistence succeed, before publishing
/// their outputs. Entries to persist are not the same as committed entries.
#[derive(Default, Clone)]
pub(crate) struct ReadyDiagnostics {
    pub entries: Distribution,
    pub committed: Distribution,
    pub light_committed: Distribution,
}

impl ReadyDiagnostics {
    pub(crate) fn snapshots(&self) -> Vec<DistributionSnapshot> {
        [
            (&self.entries, "ready_entries_to_persist"),
            (&self.committed, "ready_committed_entries"),
            (&self.light_committed, "light_ready_committed_entries"),
        ]
        .into_iter()
        .map(|(metric, name)| metric.snapshot(name, "entries"))
        .collect()
    }
}

#[derive(Serialize)]
pub struct WriteDiagnosticsSnapshot {
    pub schema_version: u32,
    pub snapshot_consistency: &'static str,
    pub bucket_rule: &'static str,
    pub joint_bucket_rule: &'static str,
    pub driver: DriverSnapshot,
    pub ready: Vec<DistributionSnapshot>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histogram_merge_retains_zero_extremes_and_overflow_evidence() {
        let mut left = Distribution::default();
        left.record(0);
        left.record(1);
        let mut right = Distribution::default();
        right.record(u64::MAX);
        left.merge(&right);
        assert_eq!(left.count, 3);
        assert_eq!(left.buckets.iter().sum::<u64>(), 3);
        assert_eq!(
            (left.buckets[0], left.buckets[1], left.buckets[64]),
            (1, 1, 1)
        );
        assert_eq!(left.sum, u64::MAX);
        assert!(left.saturated);
        right.record_duration(Duration::from_secs(u64::MAX));
        assert!(right.saturated);
    }

    #[test]
    fn joint_counts_separate_apply_from_resolution_and_include_empty_pumps() {
        let metrics = WriteDiagnostics::default();
        metrics.record_pump(
            &PumpObservation::default(),
            Some(&ServiceObservation::default()),
        );
        metrics.record_pump(
            &PumpObservation {
                applied_commands: 128,
                ..Default::default()
            },
            Some(&ServiceObservation {
                extracted: 64,
                resolved: 64,
                ..Default::default()
            }),
        );
        metrics.record_pump(&PumpObservation::default(), None);
        let snapshot = metrics.snapshot();
        assert_eq!((snapshot.successful_pumps, snapshot.failed_pumps), (2, 1));
        assert_eq!(snapshot.applied_vs_resolved[0][0], 1);
        assert_eq!(snapshot.applied_vs_resolved[8][7], 1);
        assert_eq!(
            snapshot.applied_vs_resolved.iter().flatten().sum::<u64>(),
            2
        );
        assert_eq!(snapshot.distributions[0].count, 3);
        assert_eq!(snapshot.distributions[4].count, 2);
    }

    #[test]
    fn lookup_counts_measure_the_actual_scan_and_do_not_assume_dense_indexes() {
        let metrics = WriteDiagnostics::default();
        metrics.record_bounded_lookup(3, Some(0), Some(100), 1, false);
        metrics.record_bounded_lookup(3, Some(2), Some(100), 100, false);
        metrics.record_bounded_lookup(3, None, Some(100), 2, false);
        metrics.record_bounded_lookup(0, None, None, 1, false);
        let snapshot = metrics.snapshot();
        assert_eq!((snapshot.lookup_hits, snapshot.lookup_misses), (2, 2));
        assert_eq!(snapshot.distributions[14].sum, 7);
        assert_eq!(snapshot.distributions[15].sum, 2);
        assert_eq!(snapshot.distributions[16].sum, 99);
    }

    #[test]
    fn poisoned_observer_remains_nonfatal_and_marks_every_snapshot_invalid() {
        let metrics = WriteDiagnostics::default();
        let _ = std::panic::catch_unwind(|| {
            let _held = metrics.0.lock().unwrap();
            panic!("injected observer failure");
        });
        metrics.record_bounded_lookup(1, Some(0), Some(7), 7, false);
        let snapshot = metrics.snapshot();
        assert!(!snapshot.valid);
        assert_eq!(snapshot.lookup_hits, 1);
        assert!(!metrics.snapshot().valid);
    }
}
