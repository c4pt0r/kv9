//! Bounded, opt-in observations of existing quorum boundaries.
//!
//! No event, ticket, counter or capture result is a Raft receipt. Recording
//! reserves one immutable slot without a mutex, never waits for a protocol
//! lock, and returns no decision to consensus. Capture closes observation only.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use raft::prelude::{Message, MessageType};
use serde::Serialize;

pub const SAMPLE_EVERY: u64 = 256;
pub const MAX_EVENTS: usize = 65_536;
const CAPTURE_BUDGET: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    GroupSubmit,
    GroupAdmitted,
    GroupDeferred,
    GroupRefused,
    GroupClosed,
    QuorumConfirmed,
    CompletionEligible,
    MembersUncovered,
    OutboundOffered,
    OutboundAccepted,
    OutboundRejected,
    OutboundDequeued,
    RouteDiscarded,
    QueueAbandoned,
    InboundValidated,
    InboxOffered,
    InboxAdmitted,
    InboxRejected,
    InboxDrained,
    UnknownPeer,
    EncodingRejected,
    MaskedOutbound,
    DriverStep,
}
const STAGES: usize = Stage::DriverStep as usize + 1;
pub const STAGE_NAMES: [&str; STAGES] = [
    "group_submit",
    "group_admitted",
    "group_deferred",
    "group_refused",
    "group_closed",
    "quorum_confirmed",
    "completion_eligible",
    "members_uncovered",
    "outbound_offered",
    "outbound_accepted",
    "outbound_rejected",
    "outbound_dequeued",
    "route_discarded",
    "queue_abandoned",
    "inbound_validated",
    "inbox_offered",
    "inbox_admitted",
    "inbox_rejected",
    "inbox_drained",
    "unknown_peer",
    "encoding_rejected",
    "masked_outbound",
    "driver_step",
];

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Group,
    Heartbeat,
    HeartbeatResponse,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub struct Key {
    pub context: [u8; 24],
    // Group callbacks do not acquire the peer lock to invent a term. A reader
    // may join a group to a message term only when the context is unambiguous.
    pub term: Option<u64>,
    pub from: u64,
    pub to: u64,
    pub kind: Kind,
}

impl Key {
    pub(crate) fn message(message: &Message) -> Option<Self> {
        let kind = match message.get_msg_type() {
            MessageType::MsgHeartbeat => Kind::Heartbeat,
            MessageType::MsgHeartbeatResponse => Kind::HeartbeatResponse,
            _ => return None,
        };
        Some(Self {
            context: message.context.as_ref().try_into().ok()?,
            term: Some(message.term),
            from: message.from,
            to: message.to,
            kind,
        })
    }

    fn selected(self) -> bool {
        u64::from_be_bytes(self.context[16..].try_into().unwrap()) % SAMPLE_EVERY == 0
    }
}

/// Retaining the actual route allocation prevents pointer reuse while any
/// recorded observation names it. The id is LOCAL to this trace, never wire
/// identity, a connection id, an endpoint generation or protocol authority.
#[derive(Clone)]
pub(crate) struct Route {
    id: usize,
    _keep_alive: Arc<dyn Send + Sync>,
}
impl Route {
    pub(crate) fn new<T: Send + Sync + 'static>(route: &Arc<T>) -> Self {
        Self {
            id: Arc::as_ptr(route) as usize,
            _keep_alive: route.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Event {
    pub at_ns: u64,
    pub stage: Stage,
    pub key: Key,
    pub local_ticket: Option<u64>,
    pub local_route: Option<usize>,
    pub members: Option<usize>,
    pub index: Option<u64>,
}
struct RetainedEvent {
    event: Event,
    _route: Option<Route>,
}
#[derive(Default)]
struct Counts {
    offered: AtomicU64,
    selected: AtomicU64,
    recorded: AtomicU64,
    contended: AtomicU64,
    poisoned: AtomicU64,
    full: AtomicU64,
    exhausted: AtomicU64,
}
#[derive(Debug, Serialize)]
pub struct CountSnapshot {
    pub offered: u64,
    pub selected: u64,
    pub recorded: u64,
    pub contended: u64,
    pub poisoned: u64,
    pub full: u64,
    pub exhausted: u64,
}
impl Counts {
    fn snapshot(&self) -> CountSnapshot {
        CountSnapshot {
            offered: self.offered.load(Ordering::Relaxed),
            selected: self.selected.load(Ordering::Relaxed),
            recorded: self.recorded.load(Ordering::Relaxed),
            contended: self.contended.load(Ordering::Relaxed),
            poisoned: self.poisoned.load(Ordering::Relaxed),
            full: self.full.load(Ordering::Relaxed),
            exhausted: self.exhausted.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Snapshot {
    pub schema_version: u32,
    pub node: u64,
    pub region: u64,
    pub process_id: u32,
    pub sample_every: u64,
    pub maximum_events: usize,
    pub captured_at_ns: u64,
    pub counters_valid: bool,
    pub clock: &'static str,
    pub scope: &'static str,
    pub stage_names: Vec<&'static str>,
    pub stage_counts: Vec<CountSnapshot>,
    pub events: Vec<Event>,
}

pub struct QuorumTrace {
    node: u64,
    region: u64,
    started: Instant,
    events: Box<[OnceLock<RetainedEvent>]>,
    reserved: AtomicUsize,
    counts: [Counts; STAGES],
    counter_overflow: AtomicBool,
    closed: AtomicBool,
    active: AtomicUsize,
    capacity: usize,
}
struct Active<'a>(&'a AtomicUsize);
impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl QuorumTrace {
    pub(crate) fn new(node: u64, region: u64) -> Arc<Self> {
        Self::with_capacity(node, region, MAX_EVENTS)
    }

    fn with_capacity(node: u64, region: u64, capacity: usize) -> Arc<Self> {
        assert!(capacity <= MAX_EVENTS);
        Arc::new(Self {
            node,
            region,
            started: Instant::now(),
            // Every reservation owns a different cell. Initializing an owned
            // OnceLock cannot wait for another writer; capture only reads it.
            events: (0..capacity).map(|_| OnceLock::new()).collect(),
            reserved: AtomicUsize::new(0),
            counts: std::array::from_fn(|_| Counts::default()),
            counter_overflow: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            active: AtomicUsize::new(0),
            capacity,
        })
    }

    fn enter(&self) -> Option<Active<'_>> {
        // SeqCst establishes the capture cut across the two atomics. An
        // entrant that observes closed after increment makes no count/event
        // update; any entrant that proceeds is visible to capture's wait.
        if self.closed.load(Ordering::SeqCst) {
            return None;
        }
        self.active.fetch_add(1, Ordering::SeqCst);
        let active = Active(&self.active);
        if self.closed.load(Ordering::SeqCst) {
            return None;
        }
        Some(active)
    }

    fn increment(&self, counter: &AtomicU64) {
        if counter.fetch_add(1, Ordering::Relaxed) == u64::MAX {
            // Wrap is never advertised as a valid count. No protocol state
            // depends on these counters and capture refuses valid accounting.
            self.counter_overflow.store(true, Ordering::Relaxed);
        }
    }

    fn record(&self, mut event: Event, route: Option<Route>, mint_ticket: bool) -> Option<u64> {
        let _active = self.enter()?;
        let count = &self.counts[event.stage as usize];
        self.increment(&count.offered);
        if !event.key.selected() {
            return None;
        }
        self.increment(&count.selected);
        let Ok(at) = self.started.elapsed().as_nanos().try_into() else {
            self.increment(&count.exhausted);
            return None;
        };
        event.at_ns = at;
        // The cursor never wraps or resets. Each failed strong CAS implies
        // another reservation advanced it; at most capacity such advances
        // exist over this recorder's lifetime. No lock holder can stall us.
        let mut slot = self.reserved.load(Ordering::Relaxed);
        loop {
            if slot == self.capacity {
                self.increment(&count.full);
                return None;
            }
            match self.reserved.compare_exchange(
                slot,
                slot + 1,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => slot = actual,
            }
        }
        if mint_ticket {
            // A ticket names its unique originating event slot. Gaps from
            // non-ticket events are intentional; zero remains the sentinel.
            event.local_ticket = Some(slot as u64 + 1);
        }
        let ticket = event.local_ticket;
        if self.events[slot]
            .set(RetainedEvent {
                event,
                _route: route,
            })
            .is_err()
        {
            // Unreachable under exclusive slot ownership. Fail accounting
            // closed instead of turning an observer invariant into Raft panic.
            self.counter_overflow.store(true, Ordering::Relaxed);
            self.increment(&count.exhausted);
            return None;
        }
        self.increment(&count.recorded);
        // Zero is a non-ticket success sentinel, never a serialized ticket.
        Some(ticket.unwrap_or(0))
    }

    fn event(stage: Stage, key: Key) -> Event {
        Event {
            at_ns: 0,
            stage,
            key,
            local_ticket: None,
            local_route: None,
            members: None,
            index: None,
        }
    }

    pub(crate) fn message(&self, stage: Stage, message: &Message) {
        if let Some(key) = Key::message(message) {
            self.key(stage, key);
        }
    }

    pub(crate) fn key(&self, stage: Stage, key: Key) {
        self.record(Self::event(stage, key), None, false);
    }

    pub(crate) fn group(
        &self,
        stage: Stage,
        context: [u8; 24],
        members: usize,
        index: Option<u64>,
    ) {
        let key = Key {
            context,
            term: None,
            from: self.node,
            to: self.node,
            kind: Kind::Group,
        };
        let mut event = Self::event(stage, key);
        event.members = Some(members);
        event.index = index;
        self.record(event, None, false);
    }

    pub(crate) fn queue(
        self: &Arc<Self>,
        stage: Stage,
        key: Key,
        route: Option<Route>,
    ) -> Option<Ticket> {
        let mut event = Self::event(stage, key);
        event.local_route = route.as_ref().map(|r| r.id);
        let id = self.record(event, route.clone(), true)?;
        Some(Ticket {
            point: Point {
                trace: self.clone(),
                key,
                route,
                id,
            },
            finished: false,
        })
    }

    /// One finite prefix; later observations are outside the capture domain.
    /// Run only after timed client work, since serialization perturbs scheduling.
    /// A timeout or incomplete slot yields no usable snapshot and never an
    /// error to the database. Capture cannot reopen or drain protocol queues.
    pub fn capture(&self) -> Option<Snapshot> {
        if self.closed.swap(true, Ordering::SeqCst) {
            return None;
        }
        let deadline = Instant::now() + CAPTURE_BUDGET;
        while self.active.load(Ordering::SeqCst) != 0 {
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::yield_now();
        }
        let events = self.events[..self.reserved.load(Ordering::Relaxed)]
            .iter()
            .map(|slot| slot.get().map(|retained| retained.event.clone()))
            .collect::<Option<Vec<_>>>()?;
        let captured_at_ns = self.started.elapsed().as_nanos().try_into().ok()?;
        Some(Snapshot {
            schema_version: 1, node: self.node, region: self.region, process_id: std::process::id(),
            sample_every: SAMPLE_EVERY, maximum_events: self.capacity, captured_at_ns,
            counters_valid: !self.counter_overflow.load(Ordering::Relaxed),
            clock: "process_local_monotonic_instant",
            scope: "Finite observation prefix; capture is not process shutdown. Ticket ids and retained route allocations are local only. Missing/duplicate/lost context matches are unknown; no cross-process clock subtraction. Group term is unobserved. Stage offers count observer invocations, including repeated contexts; ticket continuation stages cover selected tickets only.",
            stage_names: STAGE_NAMES.to_vec(),
            stage_counts: self.counts.iter().map(Counts::snapshot).collect(),
            events,
        })
    }
}

#[derive(Clone)]
pub(crate) struct Point {
    trace: Arc<QuorumTrace>,
    key: Key,
    route: Option<Route>,
    id: u64,
}
impl Point {
    pub(crate) fn record(&self, stage: Stage) {
        let mut event = QuorumTrace::event(stage, self.key);
        event.local_ticket = Some(self.id);
        event.local_route = self.route.as_ref().map(|r| r.id);
        self.trace.record(event, self.route.clone(), false);
    }
}

pub(crate) struct Ticket {
    pub(crate) point: Point,
    finished: bool,
}
impl Ticket {
    pub(crate) fn finish(&mut self, stage: Stage) {
        self.point.record(stage);
        self.finished = true;
    }
}
impl Drop for Ticket {
    fn drop(&mut self) {
        if !self.finished {
            self.point.record(Stage::QueueAbandoned);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn heartbeat(sequence: u64) -> Message {
        let mut context = [7; 24];
        context[16..].copy_from_slice(&sequence.to_be_bytes());
        let mut message = Message {
            from: 1,
            to: 2,
            term: 3,
            context: context.to_vec().into(),
            ..Default::default()
        };
        message.set_msg_type(MessageType::MsgHeartbeat);
        message
    }

    fn conserved(snapshot: &Snapshot) {
        assert!(snapshot.counters_valid);
        assert_eq!(snapshot.stage_names, STAGE_NAMES);
        for (i, count) in snapshot.stage_counts.iter().enumerate() {
            assert_eq!(
                count.recorded,
                snapshot
                    .events
                    .iter()
                    .filter(|e| e.stage as usize == i)
                    .count() as u64
            );
            assert_eq!(
                count.selected,
                count.recorded + count.contended + count.poisoned + count.full + count.exhausted
            );
            assert!(count.offered >= count.selected);
        }
    }

    #[test]
    fn trace_samples_exact_contexts_and_preserves_repeated_emissions() {
        let trace = QuorumTrace::with_capacity(1, 0, 12);
        let key = Key::message(&heartbeat(0)).unwrap();
        for sequence in 0..=256 {
            trace.message(Stage::InboundValidated, &heartbeat(sequence));
        }
        let mut first = trace.queue(Stage::OutboundOffered, key, None).unwrap();
        let mut second = trace.queue(Stage::OutboundOffered, key, None).unwrap();
        first.finish(Stage::OutboundDequeued);
        second.finish(Stage::RouteDiscarded);
        drop(first);
        drop(second);
        let snapshot = trace.capture().unwrap();
        conserved(&snapshot);
        assert_eq!(
            snapshot.stage_counts[Stage::InboundValidated as usize].offered,
            257
        );
        assert_eq!(
            snapshot.stage_counts[Stage::InboundValidated as usize].recorded,
            2
        );
        let starts: Vec<_> = snapshot
            .events
            .iter()
            .filter(|e| e.stage == Stage::OutboundOffered)
            .collect();
        assert_eq!(starts[0].key, starts[1].key);
        assert_ne!(starts[0].local_ticket, starts[1].local_ticket);
        assert_eq!(
            snapshot.stage_counts[Stage::QueueAbandoned as usize].recorded,
            0
        );
        assert!(trace.capture().is_none());
    }

    #[test]
    fn trace_full_slots_never_reuse_ticket_ids() {
        let trace = QuorumTrace::with_capacity(1, 0, 3);
        let message = heartbeat(0);
        let first = trace
            .queue(Stage::InboxOffered, Key::message(&message).unwrap(), None)
            .unwrap();
        trace.message(Stage::DriverStep, &message);
        let last = trace
            .queue(Stage::InboxOffered, Key::message(&message).unwrap(), None)
            .unwrap();
        assert_eq!((first.point.id, last.point.id), (1, 3));
        assert!(trace
            .queue(Stage::InboxOffered, Key::message(&message).unwrap(), None)
            .is_none());
        drop(first);
        drop(last);
        let snapshot = trace.capture().unwrap();
        conserved(&snapshot);
        assert_eq!(snapshot.stage_counts[Stage::InboxOffered as usize].full, 1);
        assert_eq!(
            snapshot.stage_counts[Stage::QueueAbandoned as usize].full,
            2
        );
        assert_eq!(snapshot.events.len(), 3);
        assert_eq!(trace.reserved.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn trace_counter_overflow_is_invalid_and_incomplete_slots_fail_closed() {
        let trace = QuorumTrace::with_capacity(1, 0, 2);
        trace.counts[Stage::DriverStep as usize]
            .offered
            .store(u64::MAX, Ordering::Relaxed);
        trace.message(Stage::DriverStep, &heartbeat(0));
        assert!(!trace.capture().unwrap().counters_valid);
        let trace = QuorumTrace::with_capacity(1, 0, 2);
        // Simulate an observer that unwound after reservation but before
        // initialization. Capture must not silently omit that reservation.
        trace.reserved.store(1, Ordering::Relaxed);
        assert!(trace.capture().is_none());
        assert!(trace.closed.load(Ordering::SeqCst));
    }

    #[test]
    fn trace_concurrent_writers_keep_all_events_and_unique_ticket_chains() {
        let trace = QuorumTrace::with_capacity(1, 0, 2048);
        let barrier = Arc::new(std::sync::Barrier::new(8));
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let trace = &trace;
                let barrier = &barrier;
                scope.spawn(move || {
                    let key = Key::message(&heartbeat(0)).unwrap();
                    barrier.wait();
                    for _ in 0..64 {
                        let mut ticket = trace.queue(Stage::OutboundOffered, key, None).unwrap();
                        ticket.point.record(Stage::OutboundAccepted);
                        ticket.finish(Stage::OutboundDequeued);
                    }
                });
            }
        });
        let snapshot = trace.capture().unwrap();
        conserved(&snapshot);
        assert_eq!(snapshot.events.len(), 1536);
        let mut chains = std::collections::BTreeMap::<_, Vec<_>>::new();
        for event in &snapshot.events {
            chains
                .entry(event.local_ticket.unwrap())
                .or_default()
                .push(event.stage);
        }
        assert_eq!(chains.len(), 512);
        for stages in chains.values() {
            assert_eq!(
                stages,
                &[
                    Stage::OutboundOffered,
                    Stage::OutboundAccepted,
                    Stage::OutboundDequeued
                ]
            );
        }
        for counts in &snapshot.stage_counts {
            assert_eq!(counts.selected, counts.recorded);
            assert_eq!(
                counts.contended + counts.poisoned + counts.full + counts.exhausted,
                0
            );
        }
    }

    #[test]
    fn trace_concurrent_saturation_has_exact_capacity_and_loss_accounting() {
        let trace = QuorumTrace::with_capacity(1, 0, 128);
        let barrier = Arc::new(std::sync::Barrier::new(8));
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let trace = &trace;
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    for _ in 0..256 {
                        trace.message(Stage::DriverStep, &heartbeat(0));
                    }
                });
            }
        });
        let snapshot = trace.capture().unwrap();
        conserved(&snapshot);
        assert_eq!(snapshot.events.len(), 128);
        let counts = &snapshot.stage_counts[Stage::DriverStep as usize];
        assert_eq!(
            (
                counts.offered,
                counts.selected,
                counts.recorded,
                counts.full
            ),
            (2048, 2048, 128, 1920)
        );
        assert_eq!(counts.contended + counts.poisoned + counts.exhausted, 0);
    }

    #[test]
    fn trace_retains_actual_route_allocation_and_records_abandonment() {
        let trace = QuorumTrace::with_capacity(1, 0, 4);
        let actual = Arc::new(123);
        let weak = Arc::downgrade(&actual);
        let route = Route::new(&actual);
        drop(actual);
        let ticket = trace
            .queue(
                Stage::OutboundOffered,
                Key::message(&heartbeat(0)).unwrap(),
                Some(route),
            )
            .unwrap();
        drop(ticket);
        let snapshot = trace.capture().unwrap();
        conserved(&snapshot);
        assert!(weak.upgrade().is_some());
        assert_eq!(
            snapshot.events[0].local_route,
            snapshot.events[1].local_route
        );
        assert_eq!(snapshot.events[1].stage, Stage::QueueAbandoned);
        drop(trace);
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn trace_capture_is_a_finite_prefix_not_a_queue_drain() {
        let trace = QuorumTrace::with_capacity(1, 0, 8);
        let ticket = trace
            .queue(
                Stage::InboxOffered,
                Key::message(&heartbeat(0)).unwrap(),
                None,
            )
            .unwrap();
        let snapshot = trace.capture().unwrap();
        drop(ticket); // After the cut: not a recorded queue abandonment.
        trace.message(Stage::DriverStep, &heartbeat(0));
        conserved(&snapshot);
        assert_eq!(snapshot.events.len(), 1);
        assert_eq!(
            trace.counts[Stage::QueueAbandoned as usize]
                .offered
                .load(Ordering::Relaxed),
            0
        );
    }

    #[test]
    fn trace_capture_conserves_concurrent_observations() {
        let trace = QuorumTrace::with_capacity(1, 0, 128);
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let writers: Vec<_> = (0..2)
            .map(|_| {
                let trace = trace.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    for _ in 0..1000 {
                        trace.message(Stage::DriverStep, &heartbeat(0));
                    }
                })
            })
            .collect();
        barrier.wait();
        let snapshot = trace.capture().unwrap();
        for writer in writers {
            writer.join().unwrap();
        }
        conserved(&snapshot);
        assert!(snapshot
            .events
            .iter()
            .all(|e| e.at_ns <= snapshot.captured_at_ns));
        assert_eq!(
            trace.counts[Stage::DriverStep as usize]
                .offered
                .load(Ordering::Relaxed),
            snapshot.stage_counts[Stage::DriverStep as usize].offered
        );
    }
}
