//! Node-local admission for reserved, queued and running public backend work.
//!
//! Bytes mean the protobuf request's encoded length, not total process RSS or
//! transport/response memory. A reservation is owned by the actual backend job;
//! cancelling its RPC cannot release capacity while the job remains live.

use kv9_common::metrics::{Latency, NamedLatency, Outcome};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};
use std::time::Instant;

use kv9_common::{Error, Result};

const CLASS_COUNT: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublicApiLimits {
    pub max_requests: usize,
    pub max_encoded_bytes: usize,
}

impl Default for PublicApiLimits {
    fn default() -> Self {
        Self {
            max_requests: 64,
            max_encoded_bytes: 64 * 1024 * 1024,
        }
    }
}

impl PublicApiLimits {
    pub fn validate(self) -> Result<Self> {
        if self.max_requests == 0 || self.max_encoded_bytes == 0 {
            return Err(Error::Config(
                "public API count and byte limits must be positive".into(),
            ));
        }
        Ok(self)
    }

    pub(crate) fn from_env() -> Result<Self> {
        Self::parse(|name| match std::env::var(name) {
            Ok(value) => Ok(Some(value)),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(_) => Err(Error::Config(format!("invalid {name}"))),
        })
    }

    fn parse(mut get: impl FnMut(&str) -> Result<Option<String>>) -> Result<Self> {
        let defaults = Self::default();
        let mut value = |name: &str, default| match get(name)? {
            None => Ok(default),
            Some(text) => text
                .parse::<usize>()
                .map_err(|_| Error::Config(format!("{name} must be a positive integer"))),
        };
        Self {
            max_requests: value("KV9_PUBLIC_MAX_REQUESTS", defaults.max_requests)?,
            max_encoded_bytes: value("KV9_PUBLIC_MAX_ENCODED_BYTES", defaults.max_encoded_bytes)?,
        }
        .validate()
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum WorkClass {
    RawRead,
    RawWrite,
    MetadataRead,
    MetadataWrite,
    Transaction,
}

impl WorkClass {
    const ALL: [Self; CLASS_COUNT] = [
        Self::RawRead,
        Self::RawWrite,
        Self::MetadataRead,
        Self::MetadataWrite,
        Self::Transaction,
    ];

    fn timing_names(self) -> (&'static str, &'static str) {
        match self {
            Self::RawRead => ("public_raw_read_prepare_queue", "public_raw_read_backend"),
            Self::RawWrite => ("public_raw_write_prepare_queue", "public_raw_write_backend"),
            Self::MetadataRead => (
                "public_metadata_read_prepare_queue",
                "public_metadata_read_backend",
            ),
            Self::MetadataWrite => (
                "public_metadata_write_prepare_queue",
                "public_metadata_write_backend",
            ),
            Self::Transaction => (
                "public_transaction_prepare_queue",
                "public_transaction_backend",
            ),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::RawRead => "raw_read",
            Self::RawWrite => "raw_write",
            Self::MetadataRead => "metadata_read",
            Self::MetadataWrite => "metadata_write",
            Self::Transaction => "transaction",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refusal {
    RequestCount,
    EncodedBytes,
    RequestTooLarge,
}

impl Refusal {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::RequestCount => "request_count",
            Self::EncodedBytes => "encoded_bytes",
            Self::RequestTooLarge => "request_too_large",
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ClassCounters {
    pub admitted: u64,
    pub completed: u64,
    pub backend_errors: u64,
    pub released_before_execution: u64,
    pub backend_aborted: u64,
    pub refused_count: u64,
    pub refused_bytes: u64,
    pub request_too_large: u64,
}

#[derive(Debug, Default, Clone)]
struct State {
    in_flight: usize,
    running: usize,
    encoded_bytes: usize,
    peak_requests: usize,
    peak_encoded_bytes: usize,
    classes: [ClassCounters; CLASS_COUNT],
}

#[derive(Debug, Clone)]
pub struct AdmissionSnapshot {
    pub limits: PublicApiLimits,
    pub in_flight: usize,
    pub queued: usize,
    pub running: usize,
    pub encoded_bytes: usize,
    pub peak_requests: usize,
    pub peak_encoded_bytes: usize,
    pub classes: [ClassCounters; CLASS_COUNT],
    pub raw_get_completed_inline: u64,
    pub raw_get_blocking_submitted: u64,
    pub raw_batch_get_completed_inline: u64,
    pub raw_batch_get_blocking_submitted: u64,
}

#[derive(Clone, Copy)]
pub(crate) enum PreparedReadKind {
    Point,
    Batch,
}

#[derive(Default)]
struct WorkTiming {
    prepare_queue: Latency,
    backend: Latency,
}

pub struct PublicAdmission {
    limits: PublicApiLimits,
    state: Mutex<State>,
    timings: [WorkTiming; CLASS_COUNT],
    raw_get_completed_inline: AtomicU64,
    raw_get_blocking_submitted: AtomicU64,
    raw_batch_get_completed_inline: AtomicU64,
    raw_batch_get_blocking_submitted: AtomicU64,
}

impl PublicAdmission {
    pub fn new(limits: PublicApiLimits) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            limits: limits.validate()?,
            state: Mutex::new(State::default()),
            timings: std::array::from_fn(|_| WorkTiming::default()),
            raw_get_completed_inline: AtomicU64::new(0),
            raw_get_blocking_submitted: AtomicU64::new(0),
            raw_batch_get_completed_inline: AtomicU64::new(0),
            raw_batch_get_blocking_submitted: AtomicU64::new(0),
        }))
    }

    pub(crate) fn reserve(
        self: &Arc<Self>,
        class: WorkClass,
        bytes: usize,
    ) -> std::result::Result<Reservation, Refusal> {
        let mut state = self.state.lock().expect("public admission poisoned");
        let refusal = if bytes > self.limits.max_encoded_bytes {
            Some(Refusal::RequestTooLarge)
        } else if state.in_flight == self.limits.max_requests {
            Some(Refusal::RequestCount)
        } else if bytes > self.limits.max_encoded_bytes - state.encoded_bytes {
            Some(Refusal::EncodedBytes)
        } else {
            None
        };
        let counters = &mut state.classes[class as usize];
        if let Some(reason) = refusal {
            let counter = match reason {
                Refusal::RequestCount => &mut counters.refused_count,
                Refusal::EncodedBytes => &mut counters.refused_bytes,
                Refusal::RequestTooLarge => &mut counters.request_too_large,
            };
            *counter = counter.saturating_add(1);
            return Err(reason);
        }
        counters.admitted = counters.admitted.saturating_add(1);
        // Both additions are bounded by the checks above, even at usize::MAX.
        state.in_flight += 1;
        state.encoded_bytes += bytes;
        state.peak_requests = state.peak_requests.max(state.in_flight);
        state.peak_encoded_bytes = state.peak_encoded_bytes.max(state.encoded_bytes);
        Ok(Reservation {
            owner: self.clone(),
            class,
            bytes,
            started: false,
            outcome: None,
            reserved_at: Instant::now(),
            started_at: None,
            finished_at: None,
        })
    }

    pub(crate) fn latency_snapshots(&self) -> Vec<NamedLatency> {
        let mut snapshots = Vec::with_capacity(CLASS_COUNT * 2);
        for class in WorkClass::ALL {
            let (prepare, backend) = class.timing_names();
            let timing = &self.timings[class as usize];
            snapshots.push(NamedLatency::new(prepare, &timing.prepare_queue));
            snapshots.push(NamedLatency::new(backend, &timing.backend));
        }
        snapshots
    }

    pub fn snapshot(&self) -> AdmissionSnapshot {
        let state = self.state.lock().expect("public admission poisoned");
        AdmissionSnapshot {
            limits: self.limits,
            in_flight: state.in_flight,
            queued: state.in_flight - state.running,
            running: state.running,
            encoded_bytes: state.encoded_bytes,
            peak_requests: state.peak_requests,
            peak_encoded_bytes: state.peak_encoded_bytes,
            classes: state.classes,
            raw_get_completed_inline: self.raw_get_completed_inline.load(Ordering::Relaxed),
            raw_get_blocking_submitted: self.raw_get_blocking_submitted.load(Ordering::Relaxed),
            raw_batch_get_completed_inline: self
                .raw_batch_get_completed_inline
                .load(Ordering::Relaxed),
            raw_batch_get_blocking_submitted: self
                .raw_batch_get_blocking_submitted
                .load(Ordering::Relaxed),
        }
    }

    pub(crate) fn record_prepared_read(&self, kind: PreparedReadKind, completed_inline: bool) {
        let counter = match (kind, completed_inline) {
            (PreparedReadKind::Point, true) => &self.raw_get_completed_inline,
            (PreparedReadKind::Point, false) => &self.raw_get_blocking_submitted,
            (PreparedReadKind::Batch, true) => &self.raw_batch_get_completed_inline,
            (PreparedReadKind::Batch, false) => &self.raw_batch_get_blocking_submitted,
        };
        let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            Some(value.saturating_add(1))
        });
    }
}

impl AdmissionSnapshot {
    pub(crate) fn status_lines(&self) -> String {
        use std::fmt::Write;
        let mut text = format!(
            "public_rpc_limit_requests={}\npublic_rpc_limit_encoded_bytes={}\npublic_rpc_in_flight={}\npublic_rpc_queued={}\npublic_rpc_running={}\npublic_rpc_encoded_bytes={}\npublic_rpc_peak_requests={}\npublic_rpc_peak_encoded_bytes={}\n",
            self.limits.max_requests, self.limits.max_encoded_bytes, self.in_flight,
            self.queued, self.running, self.encoded_bytes, self.peak_requests, self.peak_encoded_bytes,
        );
        writeln!(
            text,
            "public_raw_get_completed_inline={}\npublic_raw_get_blocking_submitted={}",
            self.raw_get_completed_inline, self.raw_get_blocking_submitted
        )
        .expect("writing to String");
        writeln!(
            text,
            "public_raw_batch_get_completed_inline={}\npublic_raw_batch_get_blocking_submitted={}",
            self.raw_batch_get_completed_inline, self.raw_batch_get_blocking_submitted
        )
        .expect("writing to String");
        for class in WorkClass::ALL {
            let c = self.classes[class as usize];
            writeln!(text, "public_rpc_{}=admitted={},completed={},backend_errors={},released_before_execution={},backend_aborted={},refused_count={},refused_bytes={},request_too_large={}",
                class.label(), c.admitted, c.completed, c.backend_errors, c.released_before_execution,
                c.backend_aborted, c.refused_count, c.refused_bytes, c.request_too_large)
                .expect("writing to String");
        }
        text
    }
}

/// Not Clone: only the job that owns this reservation can return its budget.
pub(crate) struct Reservation {
    owner: Arc<PublicAdmission>,
    class: WorkClass,
    bytes: usize,
    started: bool,
    outcome: Option<bool>,
    reserved_at: Instant,
    started_at: Option<Instant>,
    finished_at: Option<Instant>,
}

impl Reservation {
    pub(crate) fn start(&mut self) {
        self.start_at(Instant::now());
    }

    fn start_at(&mut self, now: Instant) {
        assert!(!self.started, "a reservation starts only once");
        self.owner
            .state
            .lock()
            .expect("public admission poisoned")
            .running += 1;
        self.started = true;
        self.started_at = Some(now);
        self.owner.timings[self.class as usize]
            .prepare_queue
            .record(
                now.saturating_duration_since(self.reserved_at),
                Outcome::Success,
            );
    }

    pub(crate) fn finish(self, failed: bool) {
        self.finish_at(failed, Instant::now());
    }

    fn finish_at(mut self, failed: bool, now: Instant) {
        assert!(self.started, "only a started backend job can finish");
        self.outcome = Some(failed);
        self.finished_at = Some(now);
        // Drop releases gauges and records this outcome in one locked update.
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        let mut state = self.owner.state.lock().expect("public admission poisoned");
        state.in_flight -= 1;
        state.encoded_bytes -= self.bytes;
        if self.started {
            state.running -= 1;
        }
        let c = &mut state.classes[self.class as usize];
        match self.outcome {
            Some(failed) => {
                c.completed = c.completed.saturating_add(1);
                if failed {
                    c.backend_errors = c.backend_errors.saturating_add(1);
                }
            }
            None if self.started => c.backend_aborted = c.backend_aborted.saturating_add(1),
            None => c.released_before_execution = c.released_before_execution.saturating_add(1),
        }
        drop(state);
        // Observer locks are leaves; no accounting lock is held during recording.
        let ended = self.finished_at.unwrap_or_else(Instant::now);
        let timing = &self.owner.timings[self.class as usize];
        if let Some(started) = self.started_at {
            let outcome = match self.outcome {
                Some(false) => Outcome::Success,
                Some(true) => Outcome::Error,
                None => Outcome::Aborted,
            };
            timing
                .backend
                .record(ended.saturating_duration_since(started), outcome);
        } else {
            timing.prepare_queue.record(
                ended.saturating_duration_since(self.reserved_at),
                Outcome::Released,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admission_checks_aggregate_bytes_count_and_oversized_requests() {
        let budget = PublicAdmission::new(PublicApiLimits {
            max_requests: 2,
            max_encoded_bytes: 10,
        })
        .unwrap();
        let first = budget.reserve(WorkClass::RawRead, 6).unwrap();
        assert!(
            matches!(
                budget.reserve(WorkClass::RawWrite, 5),
                Err(Refusal::EncodedBytes)
            ),
            "aggregate encoded byte limit was bypassed"
        );
        let second = budget.reserve(WorkClass::Transaction, 4).unwrap();
        assert!(
            matches!(
                budget.reserve(WorkClass::MetadataRead, 0),
                Err(Refusal::RequestCount)
            ),
            "request count limit was bypassed"
        );
        assert!(matches!(
            budget.reserve(WorkClass::MetadataWrite, 11),
            Err(Refusal::RequestTooLarge)
        ));
        let full = budget.snapshot();
        assert_eq!(
            (
                full.in_flight,
                full.queued,
                full.running,
                full.encoded_bytes
            ),
            (2, 2, 0, 10)
        );
        drop(first);
        let replacement = budget.reserve(WorkClass::RawWrite, 6).unwrap();
        drop((replacement, second));
        let empty = budget.snapshot();
        assert_eq!((empty.in_flight, empty.encoded_bytes), (0, 0));
        assert_eq!((empty.peak_requests, empty.peak_encoded_bytes), (2, 10));
        assert_eq!(
            empty
                .classes
                .iter()
                .map(|c| c.released_before_execution)
                .sum::<u64>(),
            3
        );
    }

    #[test]
    fn admission_limits_do_not_overflow_at_machine_maximum() {
        let budget = PublicAdmission::new(PublicApiLimits {
            max_requests: usize::MAX,
            max_encoded_bytes: usize::MAX,
        })
        .unwrap();
        let held = budget.reserve(WorkClass::RawRead, usize::MAX).unwrap();
        assert!(matches!(
            budget.reserve(WorkClass::RawRead, 1),
            Err(Refusal::EncodedBytes)
        ));
        let zero = budget.reserve(WorkClass::RawRead, 0).unwrap();
        drop((held, zero));
        assert_eq!(budget.snapshot().encoded_bytes, 0);
    }

    #[test]
    fn admission_saturating_counters_do_not_control_capacity() {
        let budget = PublicAdmission::new(PublicApiLimits {
            max_requests: 1,
            max_encoded_bytes: 1,
        })
        .unwrap();
        budget.state.lock().unwrap().classes[0].admitted = u64::MAX;
        budget.state.lock().unwrap().classes[0].completed = u64::MAX;
        for counter in [
            &budget.raw_get_completed_inline,
            &budget.raw_get_blocking_submitted,
            &budget.raw_batch_get_completed_inline,
            &budget.raw_batch_get_blocking_submitted,
        ] {
            counter.store(u64::MAX, Ordering::Relaxed);
        }
        for _ in 0..2 {
            let mut held = budget.reserve(WorkClass::RawRead, 1).unwrap();
            held.start();
            for kind in [PreparedReadKind::Point, PreparedReadKind::Batch] {
                budget.record_prepared_read(kind, true);
                budget.record_prepared_read(kind, false);
            }
            held.finish(false);
        }
        let state = budget.snapshot();
        assert_eq!(
            (state.in_flight, state.running, state.encoded_bytes),
            (0, 0, 0)
        );
        assert_eq!(
            (state.classes[0].admitted, state.classes[0].completed),
            (u64::MAX, u64::MAX)
        );
        assert_eq!(
            [
                state.raw_get_completed_inline,
                state.raw_get_blocking_submitted,
                state.raw_batch_get_completed_inline,
                state.raw_batch_get_blocking_submitted
            ],
            [u64::MAX; 4]
        );
        assert_eq!(state.status_lines().lines().count(), 17);
    }

    #[test]
    fn admission_configuration_rejects_zero_invalid_and_overflow() {
        assert_eq!(
            PublicApiLimits::parse(|_| Ok(None)).unwrap(),
            PublicApiLimits::default()
        );
        for name in ["KV9_PUBLIC_MAX_REQUESTS", "KV9_PUBLIC_MAX_ENCODED_BYTES"] {
            for value in ["0", "-1", "wat", "184467440737095516160"] {
                assert!(
                    PublicApiLimits::parse(|key| Ok((key == name).then(|| value.into()))).is_err()
                );
            }
        }
        let limits = PublicApiLimits::parse(|key| {
            Ok(Some(
                if key.ends_with("REQUESTS") { "3" } else { "42" }.into(),
            ))
        })
        .unwrap();
        assert_eq!(
            limits,
            PublicApiLimits {
                max_requests: 3,
                max_encoded_bytes: 42
            }
        );
    }
    #[test]
    fn admission_timing_separates_queue_execution_and_unsubmitted_release() {
        use std::time::Duration;
        let budget = PublicAdmission::new(PublicApiLimits::default()).unwrap();
        for (failed, outcome) in [(false, Outcome::Success), (true, Outcome::Error)] {
            let mut held = budget.reserve(WorkClass::RawRead, 3).unwrap();
            let base = held.reserved_at;
            held.start_at(base + Duration::from_millis(7));
            held.finish_at(failed, base + Duration::from_millis(19));
            let h = budget.timings[0].backend.snapshot();
            assert_eq!(
                h.outcomes[outcome as usize].sum_ns, 12_000_000,
                "backend timing included preparation or queue time"
            );
        }
        let mut unsubmitted = budget.reserve(WorkClass::RawRead, 3).unwrap();
        unsubmitted.finished_at = Some(unsubmitted.reserved_at + Duration::from_millis(5));
        drop(unsubmitted);
        let mut aborted = budget.reserve(WorkClass::RawRead, 3).unwrap();
        let base = aborted.reserved_at;
        aborted.start_at(base + Duration::from_millis(3));
        aborted.finished_at = Some(base + Duration::from_millis(14));
        drop(aborted);
        let queued = budget.timings[0].prepare_queue.snapshot();
        assert_eq!(
            queued.outcomes[Outcome::Success as usize].sum_ns,
            17_000_000
        );
        assert_eq!(
            queued.outcomes[Outcome::Released as usize].sum_ns,
            5_000_000
        );
        let backend = budget.timings[0].backend.snapshot();
        assert_eq!(
            backend.outcomes[Outcome::Aborted as usize].sum_ns,
            11_000_000
        );
        assert_eq!(backend.outcomes.iter().map(|h| h.count).sum::<u64>(), 3);
        assert_eq!(budget.snapshot().in_flight, 0);
    }
}
