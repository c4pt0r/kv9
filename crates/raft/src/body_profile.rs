//! Bounded observation of batch offer to the original request-body queue poll.
//! Observer state never authorizes admission, routing, delivery or consensus.
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Instant;

use kv9_common::metrics::{Latency, LatencySnapshot, Outcome};
use raft::eraftpb::MessageType;
use serde::Serialize;
use tokio_stream::{wrappers::ReceiverStream, Stream};

use crate::grpc::pb;

pub const SAMPLE_EVERY: u64 = 64;
pub const MESSAGE_KINDS: [&str; 7] = [
    "heartbeat",
    "heartbeat_response",
    "append",
    "append_response",
    "read_index",
    "read_index_response",
    "other",
];
const CLASSES: [&str; 8] = [
    "heartbeat",
    "heartbeat_response",
    "append",
    "append_response",
    "read_index",
    "read_index_response",
    "other",
    "mixed",
];
pub(crate) type Composition = [u64; 7];

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
    offered: AtomicU64,
    offered_messages: [AtomicU64; 7],
    yielded_messages: [AtomicU64; 7],
    abandoned_messages: [AtomicU64; 7],
    abandoned: AtomicU64,
    exhausted: AtomicBool,
    latency: Latency,
}

fn add(counter: &AtomicU64, value: u64, exhausted: &AtomicBool) {
    if value != 0
        && counter
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |old| {
                old.checked_add(value)
            })
            .is_err()
    {
        exhausted.store(true, Ordering::Relaxed);
    }
}

fn add_messages(target: &[AtomicU64; 7], counts: &Composition, exhausted: &AtomicBool) {
    for (counter, count) in target.iter().zip(counts) {
        add(counter, *count, exhausted);
    }
}

#[derive(Debug, Serialize)]
pub struct BatchSnapshot {
    pub class: &'static str,
    pub offered_batches: u64,
    pub selected_from_offers: u64,
    pub offered_message_counts: Composition,
    pub yielded_sample_message_counts: Composition,
    pub abandoned_sample_message_counts: Composition,
    pub abandoned_unknown_duration: u64,
    pub counter_exhausted: bool,
    pub latency: LatencySnapshot,
}

#[derive(Clone, Copy)]
pub(crate) enum Event {
    RouteChanged,
    RouteWatchClosed,
    ConnectFailedOrTimedOut,
    OutboundQueueClosed,
    BatchChannelClosed,
    RpcResolvedBeforeBatch,
    RpcResolvedDuringOffer,
    ProgressBudgetExpired,
}
const EVENTS: [&str; 8] = [
    "route_changed",
    "route_watch_closed",
    "connect_failed_or_timed_out",
    "outbound_queue_closed",
    "batch_channel_closed",
    "rpc_resolved_before_batch",
    "rpc_resolved_during_offer",
    "progress_budget_expired",
];

#[derive(Debug, Serialize)]
pub struct EventSnapshot {
    pub event: &'static str,
    pub count: u64,
}

/// Sequential independent counter/histogram observations, not an atomic ledger.
#[derive(Debug, Serialize)]
pub struct BodySnapshot {
    pub version: u32,
    pub stage: &'static str,
    pub sample_every: u64,
    pub message_kinds: [&'static str; 7],
    pub classes: [BatchSnapshot; 8],
    pub session_events: [EventSnapshot; 8],
    pub event_counter_exhausted: bool,
}

#[derive(Default)]
pub struct BodyProfile {
    cells: [Arc<Cell>; 8],
    events: [AtomicU64; 8],
    event_exhausted: AtomicBool,
}

impl BodyProfile {
    pub(crate) fn event(&self, event: Event) {
        add(&self.events[event as usize], 1, &self.event_exhausted);
    }

    /// Every nonempty offered batch belongs to exactly one fixed class. Select
    /// offers 1,65,129,... independently per class; exhaustion stops observation.
    pub(crate) fn offer(&self, batch: pb::BatchRaftMessage, counts: Composition) -> ObservedBatch {
        let mut nonzero = counts.iter().enumerate().filter(|(_, count)| **count != 0);
        let class = match (nonzero.next(), nonzero.next()) {
            (Some((kind, _)), None) => kind,
            _ => 7,
        };
        let cell = &self.cells[class];
        let previous = cell
            .offered
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |old| {
                old.checked_add(1)
            });
        let sample = match previous {
            Ok(previous) => {
                add_messages(&cell.offered_messages, &counts, &cell.exhausted);
                (previous % SAMPLE_EVERY == 0).then(|| Sample {
                    cell: cell.clone(),
                    counts,
                    started: Instant::now(),
                    finished: false,
                })
            }
            Err(_) => {
                cell.exhausted.store(true, Ordering::Relaxed);
                None
            }
        };
        ObservedBatch { batch, sample }
    }

    pub fn snapshot(&self) -> BodySnapshot {
        BodySnapshot {
            version: 1,
            stage: "batch_offer_to_body_poll",
            sample_every: SAMPLE_EVERY,
            message_kinds: MESSAGE_KINDS,
            classes: std::array::from_fn(|index| {
                let cell = &self.cells[index];
                let offered = cell.offered.load(Ordering::Relaxed);
                BatchSnapshot {
                    class: CLASSES[index],
                    offered_batches: offered,
                    selected_from_offers: offered.div_ceil(SAMPLE_EVERY),
                    offered_message_counts: std::array::from_fn(|i| {
                        cell.offered_messages[i].load(Ordering::Relaxed)
                    }),
                    yielded_sample_message_counts: std::array::from_fn(|i| {
                        cell.yielded_messages[i].load(Ordering::Relaxed)
                    }),
                    abandoned_sample_message_counts: std::array::from_fn(|i| {
                        cell.abandoned_messages[i].load(Ordering::Relaxed)
                    }),
                    abandoned_unknown_duration: cell.abandoned.load(Ordering::Relaxed),
                    counter_exhausted: cell.exhausted.load(Ordering::Relaxed),
                    latency: cell.latency.snapshot(),
                }
            }),
            session_events: std::array::from_fn(|i| EventSnapshot {
                event: EVENTS[i],
                count: self.events[i].load(Ordering::Relaxed),
            }),
            event_counter_exhausted: self.event_exhausted.load(Ordering::Relaxed),
        }
    }
}

struct Sample {
    cell: Arc<Cell>,
    counts: Composition,
    started: Instant,
    finished: bool,
}
impl Sample {
    fn yielded(mut self) {
        // Stop before any observer lock/counter work. This is ReceiverStream's
        // Ready boundary, not a socket flush or a replica acknowledgement.
        let elapsed = self.started.elapsed();
        self.finished = true;
        add_messages(
            &self.cell.yielded_messages,
            &self.counts,
            &self.cell.exhausted,
        );
        self.cell.latency.record(elapsed, Outcome::Success);
    }
}
impl Drop for Sample {
    fn drop(&mut self) {
        if !self.finished {
            add(&self.cell.abandoned, 1, &self.cell.exhausted);
            add_messages(
                &self.cell.abandoned_messages,
                &self.counts,
                &self.cell.exhausted,
            );
        }
        // Teardown takes no clock or lock and invents no completion duration.
        // Session events are separate populations, not reasons assigned to drops.
    }
}

pub(crate) struct ObservedBatch {
    batch: pb::BatchRaftMessage,
    sample: Option<Sample>,
}

pub(crate) struct ObservedStream {
    inner: ReceiverStream<ObservedBatch>,
}
impl ObservedStream {
    pub(crate) fn new(inner: ReceiverStream<ObservedBatch>) -> Self {
        Self { inner }
    }
}
impl Stream for ObservedStream {
    type Item = pb::BatchRaftMessage;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match Pin::new(&mut self.get_mut().inner).poll_next(cx) {
            Poll::Ready(Some(mut offered)) => {
                if let Some(sample) = offered.sample.take() {
                    sample.yielded();
                }
                Poll::Ready(Some(offered.batch))
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::sync::atomic::AtomicUsize;
    use std::task::{Wake, Waker};
    use tokio::sync::mpsc;

    fn batch(id: u64) -> pb::BatchRaftMessage {
        pb::BatchRaftMessage {
            msgs: vec![pb::RaftEnvelope {
                from_node: 1,
                to_node: 2,
                region_id: id,
                raft_message: vec![0, 127, 255],
                epoch_conf_ver: 8,
                epoch_version: 9,
            }],
            root_digest: vec![7; 32],
            flushed_unix_nanos: 0,
        }
    }
    const HEARTBEAT: Composition = [1, 0, 0, 0, 0, 0, 0];

    #[test]
    fn mixed_batches_keep_composition_and_independent_fixed_sampling() {
        let profile = BodyProfile::default();
        for n in 0..130 {
            let mut value = batch(n);
            value.msgs.push(value.msgs[0].clone());
            value.msgs.push(value.msgs[0].clone());
            let mut offered = profile.offer(value, [1, 0, 2, 0, 0, 0, 0]);
            assert_eq!(offered.sample.is_some(), n % 64 == 0);
            if n == 0 || n == 128 {
                offered.sample.take().unwrap().yielded();
            }
        }
        let mut heartbeat = profile.offer(batch(0), HEARTBEAT);
        heartbeat.sample.take().unwrap().yielded();
        let s = profile.snapshot();
        let row = &s.classes[7];
        assert_eq!(row.offered_batches, 130);
        assert_eq!(row.selected_from_offers, 3);
        assert_eq!(row.offered_message_counts, [130, 0, 260, 0, 0, 0, 0]);
        assert_eq!(row.yielded_sample_message_counts, [2, 0, 4, 0, 0, 0, 0]);
        assert_eq!(row.abandoned_sample_message_counts, [1, 0, 2, 0, 0, 0, 0]);
        assert_eq!(row.abandoned_unknown_duration, 1);
        assert_eq!(row.latency.outcomes[Outcome::Success as usize].count, 2);
        assert!(row.latency.outcomes.iter().skip(1).all(|h| h.count == 0));
        assert_eq!(s.classes[0].selected_from_offers, 1);
        assert_eq!(s.classes[1].offered_batches, 0);
    }

    #[derive(Default)]
    struct WakeCount(AtomicUsize);
    impl Wake for WakeCount {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn stream_preserves_pending_wakeup_fifo_payload_and_eof() {
        let profile = BodyProfile::default();
        let (tx, rx) = mpsc::channel(16);
        let mut stream = ObservedStream::new(ReceiverStream::new(rx));
        let wake = Arc::new(WakeCount::default());
        let waker = Waker::from(wake.clone());
        let mut cx = Context::from_waker(&waker);
        assert!(Pin::new(&mut stream).poll_next(&mut cx).is_pending());
        let first = batch(11);
        let second = batch(12);
        assert!(tx.try_send(profile.offer(first.clone(), HEARTBEAT)).is_ok());
        assert!(wake.0.load(Ordering::Relaxed) > 0);
        assert!(tx
            .try_send(profile.offer(second.clone(), HEARTBEAT))
            .is_ok());
        assert_eq!(stream.size_hint(), stream.inner.size_hint());
        assert_eq!(
            Pin::new(&mut stream).poll_next(&mut cx),
            Poll::Ready(Some(first))
        );
        assert_eq!(
            Pin::new(&mut stream).poll_next(&mut cx),
            Poll::Ready(Some(second))
        );
        assert!(Pin::new(&mut stream).poll_next(&mut cx).is_pending());
        drop(tx);
        assert_eq!(Pin::new(&mut stream).poll_next(&mut cx), Poll::Ready(None));
        assert_eq!(Pin::new(&mut stream).poll_next(&mut cx), Poll::Ready(None));
        let row = &profile.snapshot().classes[0];
        assert_eq!(row.offered_batches, 2);
        assert_eq!(row.latency.outcomes[Outcome::Success as usize].count, 1);
        assert_eq!(row.abandoned_unknown_duration, 0);
    }

    #[test]
    fn full_channel_pending_send_and_receiver_teardown_preserve_unknown_samples() {
        let profile = BodyProfile::default();
        let (tx, rx) = mpsc::channel(16);
        let stream = ObservedStream::new(ReceiverStream::new(rx));
        for id in 0..16 {
            assert!(tx.try_send(profile.offer(batch(id), HEARTBEAT)).is_ok());
        }
        assert_eq!(tx.capacity(), 0);
        // Unsampled offers cancelled before polling still count as offers.
        for id in 16..64 {
            drop(profile.offer(batch(id), HEARTBEAT));
        }
        let mut send = Box::pin(tx.send(profile.offer(batch(64), HEARTBEAT)));
        let mut cx = Context::from_waker(Waker::noop());
        assert!(send.as_mut().poll(&mut cx).is_pending());
        assert_eq!(profile.snapshot().classes[0].abandoned_unknown_duration, 0);
        drop(send);
        assert_eq!(profile.snapshot().classes[0].abandoned_unknown_duration, 1);
        drop(stream);
        assert!(tx.is_closed());
        let row = &profile.snapshot().classes[0];
        assert_eq!(row.offered_batches, 65);
        assert_eq!(row.selected_from_offers, 2);
        assert_eq!(row.abandoned_unknown_duration, 2);
        assert_eq!(row.abandoned_sample_message_counts, [2, 0, 0, 0, 0, 0, 0]);
        assert!(row.latency.outcomes.iter().all(|h| h.count == 0));
    }

    #[test]
    fn closed_channel_keeps_ownership_until_error_is_dropped() {
        let profile = BodyProfile::default();
        let (tx, rx) = mpsc::channel(16);
        drop(ObservedStream::new(ReceiverStream::new(rx)));
        let error = tx
            .try_send(profile.offer(batch(0), HEARTBEAT))
            .err()
            .unwrap();
        profile.event(Event::BatchChannelClosed);
        assert_eq!(profile.snapshot().classes[0].abandoned_unknown_duration, 0);
        drop(error);
        let s = profile.snapshot();
        assert_eq!(s.classes[0].abandoned_unknown_duration, 1);
        assert!(s.classes[0].latency.outcomes.iter().all(|h| h.count == 0));
        assert_eq!(
            s.session_events[Event::BatchChannelClosed as usize].count,
            1
        );
    }

    #[test]
    fn observer_exhaustion_never_wraps_or_prevents_batch_handoff() {
        let profile = BodyProfile::default();
        profile.cells[0].offered.store(u64::MAX, Ordering::Relaxed);
        let offered = profile.offer(batch(9), HEARTBEAT);
        assert!(offered.sample.is_none());
        assert_eq!(offered.batch, batch(9));
        assert_eq!(profile.snapshot().classes[0].offered_batches, u64::MAX);
        assert!(profile.snapshot().classes[0].counter_exhausted);
        let other = profile.offer(batch(1), [0, 1, 0, 0, 0, 0, 0]);
        profile.cells[1]
            .abandoned
            .store(u64::MAX, Ordering::Relaxed);
        profile.cells[1].abandoned_messages[1].store(u64::MAX, Ordering::Relaxed);
        drop(other);
        profile.events[0].store(u64::MAX, Ordering::Relaxed);
        profile.event(Event::RouteChanged);
        let s = profile.snapshot();
        assert_eq!(s.classes[1].abandoned_unknown_duration, u64::MAX);
        assert_eq!(s.classes[1].abandoned_sample_message_counts[1], u64::MAX);
        assert!(s.classes[1].counter_exhausted);
        assert_eq!(s.session_events[0].count, u64::MAX);
        assert!(s.event_counter_exhausted);
    }

    #[test]
    fn message_kind_inventory_is_fixed() {
        for (message_type, expected) in [
            (MessageType::MsgHeartbeat, 0),
            (MessageType::MsgHeartbeatResponse, 1),
            (MessageType::MsgAppend, 2),
            (MessageType::MsgAppendResponse, 3),
            (MessageType::MsgReadIndex, 4),
            (MessageType::MsgReadIndexResp, 5),
            (MessageType::MsgSnapshot, 6),
            (MessageType::MsgHup, 6),
        ] {
            let mut msg = raft::prelude::Message::default();
            msg.set_msg_type(message_type);
            assert_eq!(kind(&msg), expected);
        }
    }
}
