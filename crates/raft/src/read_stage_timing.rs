//! Opt-in observation of a successful asynchronous read's existing boundaries.
//! Timestamps travel with its reply, never act as a receipt and never decide
//! admission, confirmation, application, cancellation or deadline behavior.

use std::time::{Duration, Instant};

use kv9_common::metrics::{Latency, NamedLatency, Outcome};

pub(crate) const NAMES: [&str; 6] = [
    "raft_async_read_registration",
    "raft_async_read_owner_queue",
    "raft_async_read_quorum",
    "raft_async_read_completion",
    "raft_async_read_resume",
    "raft_async_read_total",
];

#[derive(Default)]
pub(crate) struct ReadStageMetrics {
    stages: [Latency; 6],
}

#[derive(Clone, Copy)]
pub(crate) struct Timeline {
    pub queued: Instant,
    pub issued: Option<Instant>,
    pub confirmed: Option<Instant>,
    pub sending: Option<Instant>,
}

impl Timeline {
    pub fn new(queued: Instant) -> Self {
        Self {
            queued,
            issued: None,
            confirmed: None,
            sending: None,
        }
    }

    fn durations(self, started: Instant, resumed: Instant) -> Option<[Duration; 6]> {
        let points = [
            started,
            self.queued,
            self.issued?,
            self.confirmed?,
            self.sending?,
            resumed,
        ];
        let mut stages = [Duration::ZERO; 6];
        for (stage, pair) in stages.iter_mut().zip(points.windows(2)) {
            *stage = pair[1].checked_duration_since(pair[0])?;
        }
        stages[5] = resumed.checked_duration_since(started)?;
        Some(stages)
    }
}

impl ReadStageMetrics {
    pub fn record(&self, timeline: Timeline, started: Instant, resumed: Instant) {
        let (durations, outcome) = match timeline.durations(started, resumed) {
            Some(durations) => (durations, Outcome::Success),
            // A malformed observation must be visible and cannot manufacture
            // successful samples. It must not change the database response.
            None => ([Duration::ZERO; 6], Outcome::Error),
        };
        for (metric, duration) in self.stages.iter().zip(durations) {
            metric.record(duration, outcome);
        }
    }

    pub fn snapshots(&self) -> Vec<NamedLatency> {
        NAMES
            .into_iter()
            .zip(&self.stages)
            .map(|(name, metric)| NamedLatency::new(name, metric))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matched_stages_conserve_the_exact_successful_wait() {
        let started = Instant::now();
        let at = |ns| started + Duration::from_nanos(ns);
        let metrics = ReadStageMetrics::default();
        metrics.record(
            Timeline {
                queued: at(2),
                issued: Some(at(5)),
                confirmed: Some(at(12)),
                sending: Some(at(23)),
            },
            started,
            at(36),
        );
        let snapshots = metrics.snapshots();
        let sums: Vec<_> = snapshots
            .iter()
            .map(|metric| {
                let success = &metric.latency.outcomes[Outcome::Success as usize];
                assert!(metric.latency.valid);
                assert_eq!(success.count, 1);
                success.sum_ns
            })
            .collect();
        assert_eq!(sums, [2, 3, 7, 11, 13, 36]);
        assert_eq!(sums[..5].iter().sum::<u64>(), sums[5]);
    }

    #[test]
    fn missing_or_reversed_boundaries_are_errors_without_success_samples() {
        let started = Instant::now();
        let at = |ns| started + Duration::from_nanos(ns);
        let good = Timeline {
            queued: at(1),
            issued: Some(at(2)),
            confirmed: Some(at(3)),
            sending: Some(at(4)),
        };
        let mut variants = [good; 6];
        variants[0].issued = None;
        variants[1].confirmed = None;
        variants[2].sending = None;
        variants[3].issued = Some(started);
        variants[4].confirmed = Some(at(1));
        variants[5].sending = Some(at(6));
        let metrics = ReadStageMetrics::default();
        for timeline in variants {
            metrics.record(timeline, started, at(5));
        }
        for metric in metrics.snapshots() {
            assert_eq!(metric.latency.outcomes[Outcome::Success as usize].count, 0);
            assert_eq!(metric.latency.outcomes[Outcome::Error as usize].count, 6);
        }
    }
}
