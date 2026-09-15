//! Bounded, default-off write-stage observations. Exact sampled positions join
//! group preparation/application/publication to terminal receipt inspection.
//! There are no keys, values, caller identifiers, I/O or protocol decisions here.
//! Recording never allocates or waits for the observer lock. Loss is explicit.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, TryLockError};
use std::time::{Duration, Instant};

use kv9_common::AppliedPosition;
use serde::Serialize;

pub const CAPACITY: usize = 512;
pub const SAMPLE_STRIDE: u64 = 16;
static LAST_TRACE_INSTANCE: AtomicU64 = AtomicU64::new(0);

fn next_instance(last: &AtomicU64) -> Option<u64> {
    last.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
        .ok()
        .map(|previous| previous + 1)
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct GroupRecord {
    pub sequence: u64,
    pub group_sequence: u64,
    pub term: u64,
    pub index: u64,
    pub commands: u64,
    pub encoded_bytes: u64,
    pub prepare_started_ns: u64,
    pub locks_acquired_ns: u64,
    pub apply_started_ns: u64,
    pub apply_finished_ns: u64,
    /// None on an unsuccessful apply: no success receipt was published.
    pub receipts_inserted_ns: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InspectionOutcome {
    #[default]
    Applied,
    Manifest,
    FenceRejected,
    Replaced,
    Failed,
    Unconfirmed,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct InspectionRecord {
    pub sequence: u64,
    pub term: u64,
    pub index: u64,
    pub inspect_started_ns: u64,
    pub inspect_finished_ns: u64,
    /// The original waiter's existing elapsed sample, taken before inspection.
    pub registration_age_ns: u64,
    pub outcome: InspectionOutcome,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct GroupTiming {
    pub commands: u64,
    pub encoded_bytes: u64,
    pub prepare_started_ns: u64,
    pub locks_acquired_ns: u64,
    pub apply_started_ns: u64,
    pub apply_finished_ns: u64,
    pub receipts_inserted_ns: Option<u64>,
}

struct Ring<T> {
    rows: [T; CAPACITY],
    total: u64,
}
impl<T: Default + Copy> Default for Ring<T> {
    fn default() -> Self {
        Self {
            rows: [T::default(); CAPACITY],
            total: 0,
        }
    }
}
impl<T: Copy> Ring<T> {
    fn push(&mut self, make: impl FnOnce(u64) -> T) -> bool {
        let Some(next) = self.total.checked_add(1) else {
            return false;
        };
        self.rows[(self.total % CAPACITY as u64) as usize] = make(next);
        self.total = next;
        true
    }
}

#[derive(Default)]
struct State {
    groups_seen: u64,
    group_commands_seen: u64,
    terminal_inspections_seen: u64,
    saturated: bool,
    groups: Ring<GroupRecord>,
    inspections: Ring<InspectionRecord>,
}

pub(crate) struct WriteStageTrace {
    instance: u64,
    started: Instant,
    state: Mutex<State>,
    dropped_recording_calls: AtomicU64,
    invalid: AtomicBool,
}
impl Default for WriteStageTrace {
    fn default() -> Self {
        let instance = next_instance(&LAST_TRACE_INSTANCE);
        Self {
            instance: instance.unwrap_or(0),
            started: Instant::now(),
            state: Mutex::new(State::default()),
            dropped_recording_calls: AtomicU64::new(0),
            invalid: AtomicBool::new(instance.is_none()),
        }
    }
}

fn selected(index: u64) -> bool {
    index != 0 && index.is_multiple_of(SAMPLE_STRIDE)
}
fn add(into: &mut u64, amount: u64, saturated: &mut bool) {
    match into.checked_add(amount) {
        Some(next) => *into = next,
        None => {
            *into = u64::MAX;
            *saturated = true;
        }
    }
}

impl WriteStageTrace {
    pub(crate) fn now(&self) -> u64 {
        self.nanos(self.started.elapsed())
    }
    fn nanos(&self, elapsed: Duration) -> u64 {
        let nanos = elapsed.as_nanos();
        if nanos > u128::from(u64::MAX) {
            self.invalid.store(true, Ordering::Relaxed);
        }
        nanos.min(u128::from(u64::MAX)) as u64
    }
    fn record_lock(&self) -> Option<MutexGuard<'_, State>> {
        match self.state.try_lock() {
            Ok(state) => Some(state),
            Err(error) => {
                if matches!(error, TryLockError::Poisoned(_)) {
                    self.invalid.store(true, Ordering::Relaxed);
                }
                if self
                    .dropped_recording_calls
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
                    .is_err()
                {
                    self.invalid.store(true, Ordering::Relaxed);
                }
                None
            }
        }
    }
    pub(crate) fn record_group(
        &self,
        timing: GroupTiming,
        positions: impl Iterator<Item = AppliedPosition>,
    ) {
        let Some(mut guard) = self.record_lock() else {
            return;
        };
        let state = &mut *guard;
        add(&mut state.groups_seen, 1, &mut state.saturated);
        add(
            &mut state.group_commands_seen,
            timing.commands,
            &mut state.saturated,
        );
        let group_sequence = state.groups_seen;
        // Store each ACTUAL selected member, not every number between endpoints.
        // This remains exact for gaps, term changes and replacement attempts.
        for at in positions.filter(|at| selected(at.index)) {
            state.saturated |= !state.groups.push(|sequence| GroupRecord {
                sequence,
                group_sequence,
                term: at.term,
                index: at.index,
                commands: timing.commands,
                encoded_bytes: timing.encoded_bytes,
                prepare_started_ns: timing.prepare_started_ns,
                locks_acquired_ns: timing.locks_acquired_ns,
                apply_started_ns: timing.apply_started_ns,
                apply_finished_ns: timing.apply_finished_ns,
                receipts_inserted_ns: timing.receipts_inserted_ns,
            });
        }
    }
    pub(crate) fn record_inspection(
        &self,
        at: AppliedPosition,
        started: u64,
        finished: u64,
        registration_age: Duration,
        outcome: InspectionOutcome,
    ) {
        let registration_age_ns = self.nanos(registration_age);
        let Some(mut guard) = self.record_lock() else {
            return;
        };
        let state = &mut *guard;
        add(
            &mut state.terminal_inspections_seen,
            1,
            &mut state.saturated,
        );
        if selected(at.index) {
            state.saturated |= !state.inspections.push(|sequence| InspectionRecord {
                sequence,
                term: at.term,
                index: at.index,
                inspect_started_ns: started,
                inspect_finished_ns: finished,
                registration_age_ns,
                outcome,
            });
        }
    }

    pub(crate) fn snapshot(&self) -> TraceSnapshot {
        let capture_started_ns = self.now();
        // Copy only fixed arrays under the leaf lock; allocation/serialization
        // happens after release. A busy snapshot supplies no partial row set.
        let copied = match self.state.try_lock() {
            Ok(s) => Some((
                s.groups_seen,
                s.group_commands_seen,
                s.terminal_inspections_seen,
                s.saturated,
                s.groups.total,
                s.groups.rows,
                s.inspections.total,
                s.inspections.rows,
            )),
            Err(TryLockError::WouldBlock) => None,
            Err(TryLockError::Poisoned(_)) => {
                self.invalid.store(true, Ordering::Relaxed);
                None
            }
        };
        let capture_finished_ns = self.now();
        let rows_available = copied.is_some();
        let (
            groups_seen,
            group_commands_seen,
            terminal_inspections_seen,
            saturated,
            group_total,
            groups,
            inspection_total,
            inspections,
        ) = copied.unwrap_or((
            0,
            0,
            0,
            false,
            0,
            [GroupRecord::default(); CAPACITY],
            0,
            [InspectionRecord::default(); CAPACITY],
        ));
        TraceSnapshot {
            schema_version: 1,
            trace_instance: self.instance,
            clock: "nanoseconds_since_this_driver_trace_creation",
            scope: "sampled_actual_group_members_and_terminal_async_receipt_inspections",
            snapshot_consistency: "coherent_rows_independent_loss_and_clock_observations",
            capture_started_ns,
            capture_finished_ns,
            sample_stride: SAMPLE_STRIDE,
            capacity_per_ring: CAPACITY,
            rows_available,
            valid: rows_available && !saturated && !self.invalid.load(Ordering::Relaxed),
            dropped_recording_calls: self.dropped_recording_calls.load(Ordering::Relaxed),
            groups_seen,
            group_commands_seen,
            terminal_inspections_seen,
            groups: RingSnapshot::new(group_total, groups),
            inspections: RingSnapshot::new(inspection_total, inspections),
        }
    }
}

#[derive(Serialize)]
pub struct RingSnapshot<T> {
    pub total_recorded: u64,
    pub overwritten: u64,
    pub rows: Vec<T>,
}
impl<T: Copy> RingSnapshot<T> {
    fn new(total: u64, rows: [T; CAPACITY]) -> Self {
        let retained = total.min(CAPACITY as u64);
        Self {
            total_recorded: total,
            overwritten: total - retained,
            rows: (total - retained..total)
                .map(|i| rows[(i % CAPACITY as u64) as usize])
                .collect(),
        }
    }
}

#[derive(Serialize)]
pub struct TraceSnapshot {
    pub schema_version: u32,
    /// Unique within the enclosing process lifetime; never a cluster authority.
    pub trace_instance: u64,
    pub clock: &'static str,
    pub scope: &'static str,
    pub snapshot_consistency: &'static str,
    pub capture_started_ns: u64,
    pub capture_finished_ns: u64,
    pub sample_stride: u64,
    pub capacity_per_ring: usize,
    pub rows_available: bool,
    pub valid: bool,
    pub dropped_recording_calls: u64,
    pub groups_seen: u64,
    pub group_commands_seen: u64,
    pub terminal_inspections_seen: u64,
    pub groups: RingSnapshot<GroupRecord>,
    pub inspections: RingSnapshot<InspectionRecord>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn driver_recreation_has_a_distinct_trace_origin_and_exhaustion_never_wraps() {
        let first = WriteStageTrace::default().snapshot();
        let second = WriteStageTrace::default().snapshot();
        assert!(first.trace_instance != 0 && second.trace_instance > first.trace_instance);
        let counter = AtomicU64::new(u64::MAX - 1);
        assert_eq!(next_instance(&counter), Some(u64::MAX));
        assert_eq!(next_instance(&counter), None);
        assert_eq!(next_instance(&counter), None);
        assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
    }

    #[test]
    fn exact_sampled_members_keep_gaps_terms_failed_apply_and_inspection_outcomes() {
        let trace = WriteStageTrace::default();
        let positions = [
            AppliedPosition { term: 3, index: 16 },
            AppliedPosition { term: 4, index: 48 },
            AppliedPosition { term: 4, index: 49 },
        ];
        trace.record_group(
            GroupTiming {
                commands: 3,
                prepare_started_ns: 10,
                apply_started_ns: 20,
                apply_finished_ns: 40,
                ..Default::default()
            },
            positions.into_iter(),
        );
        trace.record_inspection(
            positions[1],
            50,
            60,
            Duration::from_nanos(9),
            InspectionOutcome::Replaced,
        );
        let s = trace.snapshot();
        assert!(s.valid);
        assert_eq!((s.groups_seen, s.group_commands_seen), (1, 3));
        assert_eq!(
            s.groups
                .rows
                .iter()
                .map(|r| (r.term, r.index))
                .collect::<Vec<_>>(),
            [(3, 16), (4, 48)]
        );
        assert!(s
            .groups
            .rows
            .iter()
            .all(|r| r.receipts_inserted_ns.is_none()));
        assert_eq!(s.inspections.rows[0].outcome, InspectionOutcome::Replaced);
        assert_eq!(s.inspections.rows[0].registration_age_ns, 9);
    }

    #[test]
    fn bounded_rings_report_every_overwrite_in_sequence_order() {
        let trace = WriteStageTrace::default();
        for n in 1..=CAPACITY as u64 + 7 {
            let at = AppliedPosition {
                term: 1,
                index: n * SAMPLE_STRIDE,
            };
            trace.record_group(
                GroupTiming {
                    commands: 1,
                    receipts_inserted_ns: Some(9),
                    ..Default::default()
                },
                std::iter::once(at),
            );
            trace.record_inspection(at, 10, 11, Duration::ZERO, InspectionOutcome::Applied);
        }
        let s = trace.snapshot();
        assert!(s.valid);
        assert_eq!(s.groups.overwritten, 7);
        assert_eq!(s.inspections.overwritten, 7);
        assert_eq!(s.groups.rows.len(), CAPACITY);
        assert_eq!(s.inspections.rows.len(), CAPACITY);
        for (n, (g, i)) in s.groups.rows.iter().zip(&s.inspections.rows).enumerate() {
            assert_eq!(g.sequence, n as u64 + 8);
            assert_eq!((g.sequence, g.index), (i.sequence, i.index));
        }
    }

    #[test]
    fn contention_poisoning_and_overflow_never_wait_or_fabricate_complete_rows() {
        let trace = WriteStageTrace::default();
        let held = trace.state.lock().unwrap();
        trace.record_group(GroupTiming::default(), std::iter::empty());
        assert!(!trace.snapshot().rows_available);
        drop(held);
        assert_eq!(trace.snapshot().dropped_recording_calls, 1);
        let _ = std::panic::catch_unwind(|| {
            let _held = trace.state.lock().unwrap();
            panic!("observer poison");
        });
        trace.record_group(GroupTiming::default(), std::iter::empty());
        assert!(!trace.snapshot().valid);
        let trace = WriteStageTrace::default();
        assert_eq!(trace.nanos(Duration::from_secs(u64::MAX)), u64::MAX);
        assert!(!trace.snapshot().valid);
        let trace = WriteStageTrace::default();
        trace.state.lock().unwrap().groups_seen = u64::MAX;
        trace.record_group(GroupTiming::default(), std::iter::empty());
        assert!(!trace.snapshot().valid);
    }
}
