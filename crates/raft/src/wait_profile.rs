//! Diagnostic-only, bounded local transport waiting observations.
//! No observation is consulted by message admission, routing or Raft.
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use kv9_common::metrics::{Latency, LatencySnapshot, Outcome};
use raft::eraftpb::MessageType;
use serde::Serialize;

pub const SAMPLE_EVERY: u64 = 64;
const KINDS: [&str; 7] = [
    "heartbeat",
    "heartbeat_response",
    "append",
    "append_response",
    "read_index",
    "read_index_response",
    "other",
];

pub(crate) fn kind(message: &raft::prelude::Message) -> usize {
    match message.get_msg_type() {
        MessageType::MsgHeartbeat => 0,
        MessageType::MsgHeartbeatResponse => 1,
        MessageType::MsgAppend => 2,
        MessageType::MsgAppendResponse => 3,
        MessageType::MsgReadIndex => 4,
        MessageType::MsgReadIndexResp => 5,
        _ => 6,
    }
}

#[derive(Default)]
struct Cell {
    attempts: AtomicU64,
    abandoned: AtomicU64,
    exhausted: AtomicBool,
    latency: Latency,
}

/// Histograms are coherent independently. Counter loads and histograms are
/// sequential observations, not an atomic conservation snapshot.
#[derive(Debug, Serialize)]
pub struct WaitSnapshot {
    pub stage: &'static str,
    pub kind: &'static str,
    pub sample_every: u64,
    pub attempts: u64,
    pub selected_from_attempts: u64,
    pub abandoned_unknown_duration: u64,
    pub counter_exhausted: bool,
    pub latency: LatencySnapshot,
}

pub(crate) struct WaitSeries {
    stage: &'static str,
    kinds: &'static [&'static str],
    cells: Box<[Arc<Cell>]>,
}

impl WaitSeries {
    pub(crate) fn new(stage: &'static str) -> Self {
        Self::with_kinds(stage, &KINDS)
    }

    pub(crate) fn batch_channel() -> Self {
        Self::with_kinds("batch_channel_admission", &["batch"])
    }

    fn with_kinds(stage: &'static str, kinds: &'static [&'static str]) -> Self {
        Self {
            stage,
            kinds,
            cells: kinds.iter().map(|_| Arc::new(Cell::default())).collect(),
        }
    }

    /// Selection starts at attempt 1, then 65, 129, ... independently per kind.
    /// Exhaustion stops observation, not database work. The counter never wraps.
    pub(crate) fn sample(&self, kind: usize) -> Option<Sample> {
        let cell = &self.cells[kind];
        let previous =
            match cell
                .attempts
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                    value.checked_add(1)
                }) {
                Ok(previous) => previous,
                Err(_) => {
                    cell.exhausted.store(true, Ordering::Relaxed);
                    return None;
                }
            };
        (previous % SAMPLE_EVERY == 0).then(|| Sample {
            cell: cell.clone(),
            started: Instant::now(),
            finished: false,
        })
    }

    pub(crate) fn snapshots(&self) -> Vec<WaitSnapshot> {
        self.cells
            .iter()
            .zip(self.kinds.iter().copied())
            .map(|(cell, kind)| {
                let attempts = cell.attempts.load(Ordering::Relaxed);
                WaitSnapshot {
                    stage: self.stage,
                    kind,
                    sample_every: SAMPLE_EVERY,
                    attempts,
                    selected_from_attempts: attempts.div_ceil(SAMPLE_EVERY),
                    abandoned_unknown_duration: cell.abandoned.load(Ordering::Relaxed),
                    counter_exhausted: cell.exhausted.load(Ordering::Relaxed),
                    latency: cell.latency.snapshot(),
                }
            })
            .collect()
    }
}

pub(crate) struct Sample {
    cell: Arc<Cell>,
    started: Instant,
    finished: bool,
}

impl Sample {
    /// Reset immediately before the existing queue insertion, after its
    /// original application lock has been acquired. This takes no new lock.
    pub(crate) fn restart(&mut self) {
        self.started = Instant::now();
    }

    /// Finish outside application locks. Observer locks never call back into
    /// queue, route or driver code. The outcome describes this local boundary.
    pub(crate) fn finish(self, outcome: Outcome) {
        self.stop().record(outcome);
    }

    /// Capture the dequeue boundary without taking an observer lock. The
    /// bounded drain retains selected completions until its queue lock is gone.
    pub(crate) fn stop(self) -> CompletedSample {
        let elapsed = self.started.elapsed();
        CompletedSample {
            sample: self,
            elapsed,
        }
    }
}

pub(crate) struct CompletedSample {
    sample: Sample,
    elapsed: Duration,
}

impl CompletedSample {
    pub(crate) fn record(mut self, outcome: Outcome) {
        self.sample.finished = true;
        self.sample.cell.latency.record(self.elapsed, outcome);
    }
}

impl Drop for Sample {
    fn drop(&mut self) {
        if !self.finished
            && self
                .cell
                .abandoned
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_add(1))
                .is_err()
        {
            self.cell.exhausted.store(true, Ordering::Relaxed);
        }
        // Drop can occur during queue/task teardown. It takes no observer lock,
        // reads no clock and never invents a completed waiting duration.
    }
}

pub(crate) fn finish(sample: Option<Sample>, outcome: Outcome) {
    if let Some(sample) = sample {
        sample.finish(outcome);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sampling_preserves_kind_populations_and_unknown_drops() {
        let series = WaitSeries::new("test");
        for n in 0..130 {
            let sample = series.sample(0);
            assert_eq!(sample.is_some(), n % 64 == 0);
            if n == 64 {
                drop(sample);
            } else {
                finish(sample, Outcome::Success);
            }
        }
        finish(series.sample(1), Outcome::Rejected);
        let rows = series.snapshots();
        assert_eq!(rows[0].attempts, 130);
        assert_eq!(rows[0].selected_from_attempts, 3);
        assert_eq!(rows[0].abandoned_unknown_duration, 1);
        assert_eq!(rows[0].latency.outcomes[Outcome::Success as usize].count, 2);
        assert_eq!(rows[0].latency.outcomes[Outcome::Aborted as usize].count, 0);
        assert_eq!(
            rows[1].latency.outcomes[Outcome::Rejected as usize].count,
            1
        );
        assert_eq!(rows[2].attempts, 0);
    }

    #[test]
    fn exhausted_observer_counters_do_not_wrap_or_create_more_samples() {
        let series = WaitSeries::new("test");
        series.cells[0].attempts.store(u64::MAX, Ordering::Relaxed);
        assert!(series.sample(0).is_none());
        let sample = series.sample(1).unwrap();
        series.cells[1].abandoned.store(u64::MAX, Ordering::Relaxed);
        drop(sample);
        let rows = series.snapshots();
        assert_eq!(rows[0].attempts, u64::MAX);
        assert!(rows[0].counter_exhausted);
        assert_eq!(rows[1].abandoned_unknown_duration, u64::MAX);
        assert!(rows[1].counter_exhausted);
    }

    #[test]
    fn deferred_recording_keeps_the_dequeue_timestamp_and_drop_is_unknown() {
        let series = WaitSeries::new("test");
        let mut completed = series.sample(0).unwrap().stop();
        let elapsed = completed.elapsed.as_nanos() as u64;
        // Recording must use the stopped duration, even if the originating
        // clock differs by the time the application lock has been released.
        completed.sample.started -= Duration::from_secs(60);
        completed.record(Outcome::Success);
        drop(series.sample(1).unwrap().stop());
        let rows = series.snapshots();
        assert_eq!(
            rows[0].latency.outcomes[Outcome::Success as usize].sum_ns,
            elapsed
        );
        assert_eq!(rows[1].abandoned_unknown_duration, 1);
        assert!(rows[1].latency.outcomes.iter().all(|h| h.count == 0));
    }

    #[test]
    fn canceling_a_polled_wait_preserves_unknown_without_a_completed_latency() {
        let series = WaitSeries::batch_channel();
        let sample = series.sample(0);
        let mut future = Box::pin(async move {
            std::future::pending::<()>().await;
            finish(sample, Outcome::Success);
        });
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        assert!(std::future::Future::poll(future.as_mut(), &mut cx).is_pending());
        assert_eq!(series.snapshots()[0].abandoned_unknown_duration, 0);
        drop(future);
        let rows = series.snapshots();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, "batch");
        assert_eq!(rows[0].attempts, 1);
        assert_eq!(rows[0].abandoned_unknown_duration, 1);
        assert!(rows[0].latency.outcomes.iter().all(|h| h.count == 0));
    }
}
