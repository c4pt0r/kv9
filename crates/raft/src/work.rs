//! Bounded notification and inbound ownership for the event-driven Raft pump.
//!
//! Producers publish work before notifying. The owner consumes the coalesced
//! notification before draining, and checks the same mutex-protected predicate
//! when parking. A notification during a drain therefore survives that drain.
//! Notifications do not advance Raft time and grant no persistence authority.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use protobuf::Message as _;
use raft::eraftpb::Message;

/// Completion notifications are hints to recheck exact receipts, never receipts
/// themselves. A waiter observes the generation BEFORE inspecting application
/// state, then waits only if it is unchanged. This closes publication between
/// the receipt lookup and parking without allocating a per-request wait queue.
pub(crate) struct CompletionSignal {
    generation: Mutex<Option<u64>>,
    changed: Condvar,
    #[cfg(test)]
    park_observer: Mutex<Option<std::sync::mpsc::Sender<()>>>,
}

impl Default for CompletionSignal {
    fn default() -> Self {
        Self {
            generation: Mutex::new(Some(0)),
            changed: Condvar::new(),
            #[cfg(test)]
            park_observer: Mutex::new(None),
        }
    }
}

impl CompletionSignal {
    pub(crate) fn observe(&self) -> kv9_common::Result<u64> {
        self.generation
            .lock()
            .expect("completion signal poisoned")
            .ok_or_else(Self::exhausted)
    }

    /// Call only AFTER publishing observable state and releasing its locks.
    /// Exhaustion is terminal and wakes everyone; generations never wrap.
    pub(crate) fn publish(&self) -> kv9_common::Result<()> {
        let mut generation = self.generation.lock().expect("completion signal poisoned");
        *generation = generation.and_then(|value| value.checked_add(1));
        self.changed.notify_all();
        generation.map(|_| ()).ok_or_else(Self::exhausted)
    }

    pub(crate) fn wait(&self, observed: u64, remaining: Duration) -> kv9_common::Result<()> {
        let generation = self.generation.lock().expect("completion signal poisoned");
        #[cfg(test)]
        if *generation == Some(observed) && !remaining.is_zero() {
            if let Some(observer) = self.park_observer.lock().unwrap().take() {
                let _ = observer.send(());
            }
        }
        let (generation, _) = self
            .changed
            .wait_timeout_while(generation, remaining, |current| *current == Some(observed))
            .expect("completion signal poisoned");
        generation.map(|_| ()).ok_or_else(Self::exhausted)
    }

    fn exhausted() -> kv9_common::Error {
        kv9_common::Error::Raft("completion notification generation exhausted".into())
    }

    #[cfg(test)]
    pub(crate) fn observe_next_park(&self) -> std::sync::mpsc::Receiver<()> {
        let (tx, rx) = std::sync::mpsc::channel();
        *self.park_observer.lock().unwrap() = Some(tx);
        rx
    }
}

#[derive(Debug, Default)]
struct State {
    pending: bool,
    stopped: bool,
}

// One experimental budget, bounded by the owner's existing tick deadline.
// Polling changes scheduling only; the mutex predicate remains authoritative.
const OWNER_POLL_BUDGET: Duration = Duration::from_micros(32);

#[cfg(test)]
type PollObserver = (std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>);

#[derive(Debug, Default)]
pub struct WorkSignal {
    state: Mutex<State>,
    changed: Condvar,
    poll_hint: AtomicBool,
    #[cfg(test)]
    park_observer: Mutex<Option<std::sync::mpsc::Sender<()>>>,
    #[cfg(test)]
    poll_observer: Mutex<Option<PollObserver>>,
}

impl WorkSignal {
    pub fn notify(&self) {
        let mut state = self.state.lock().expect("work signal poisoned");
        if !state.stopped {
            state.pending = true;
            self.poll_hint.store(true, Ordering::Release);
            self.changed.notify_one();
        }
    }

    pub(crate) fn stop(&self) {
        let mut state = self.state.lock().expect("work signal poisoned");
        state.stopped = true;
        self.poll_hint.store(true, Ordering::Release);
        self.changed.notify_all();
    }

    /// Consume BEFORE reading work queues. No work/peer lock nests inside this
    /// guard. Returns false once the owner must stop.
    pub(crate) fn begin_turn(&self) -> bool {
        let mut state = self.state.lock().expect("work signal poisoned");
        state.pending = false;
        self.poll_hint.store(false, Ordering::Relaxed);
        !state.stopped
    }

    pub(crate) fn wait_until(&self, deadline: Instant) {
        #[cfg(test)]
        if let Some((entered, resume)) = self.poll_observer.lock().unwrap().take() {
            let _ = entered.send(());
            let _ = resume.recv_timeout(Duration::from_secs(5));
        }
        // Do not hold the signal/queue/peer mutex while polling. A stale false
        // hint only spends this finite budget; a stale true hint only skips it.
        // Neither can bypass the original atomic predicate-to-park sequence.
        let poll_deadline = deadline.min(Instant::now() + OWNER_POLL_BUDGET);
        while !self.poll_hint.load(Ordering::Acquire) {
            if Instant::now() >= poll_deadline {
                break;
            }
            std::hint::spin_loop();
        }
        let mut state = self.state.lock().expect("work signal poisoned");
        while !state.pending && !state.stopped {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            #[cfg(test)]
            if let Some(observer) = self.park_observer.lock().unwrap().take() {
                let _ = observer.send(());
            }
            state = self
                .changed
                .wait_timeout(state, remaining)
                .expect("work signal poisoned")
                .0;
        }
    }

    #[cfg(test)]
    pub(crate) fn observe_next_park(&self) -> std::sync::mpsc::Receiver<()> {
        let (tx, rx) = std::sync::mpsc::channel();
        *self.park_observer.lock().unwrap() = Some(tx);
        rx
    }
}

/// Tick deadlines stay on a monotonic time axis, independent of traffic.
/// A delayed owner delivers one coalesced tick and starts a new interval;
/// it never emits a burst of artificial election ticks to repay a long stall.
/// Progress assumes bounded service time and eventual owner scheduling.
pub(crate) struct TickDeadline {
    next: Instant,
    period: Duration,
}

impl TickDeadline {
    pub(crate) fn new(now: Instant, period: Duration) -> Self {
        assert!(!period.is_zero());
        Self {
            next: now + period,
            period,
        }
    }

    pub(crate) fn due(&mut self, now: Instant) -> bool {
        if now < self.next {
            return false;
        }
        self.next = now + self.period;
        true
    }

    pub(crate) fn next(&self) -> Instant {
        self.next
    }
}

const MAX_INBOX_MESSAGES: usize = 4096;
const MAX_INBOX_BYTES: usize = 64 * 1024 * 1024;
const DRAIN_MESSAGES: usize = 256;
const DRAIN_BYTES: usize = 1024 * 1024;

#[derive(Default)]
struct InboxState {
    messages: VecDeque<(Message, usize)>,
    bytes: usize,
    signal: Option<Arc<WorkSignal>>,
}

/// A shared best-effort transport inbox. Limits count queued messages and their
/// protobuf encoded weights, not total process memory or allocated capacities.
/// A bounded prefix is drained per turn; one legal large message may exceed the
/// per-turn byte target. Refusal means the transport must drop/retry, never ACK
/// consensus on its own. Raft's ordinary retransmission remains responsible.
#[derive(Clone, Default)]
pub struct RaftInbox(Arc<Mutex<InboxState>>);

#[derive(Debug)]
pub struct InboxFull;

impl RaftInbox {
    pub fn send(&self, message: Message) -> Result<(), InboxFull> {
        let bytes = message.compute_size() as usize;
        let signal = {
            let mut state = self.0.lock().expect("raft inbox poisoned");
            if state.messages.len() >= MAX_INBOX_MESSAGES
                || bytes > MAX_INBOX_BYTES.saturating_sub(state.bytes)
            {
                return Err(InboxFull);
            }
            state.bytes += bytes;
            state.messages.push_back((message, bytes));
            state.signal.clone()
        };
        if let Some(signal) = signal {
            signal.notify();
        }
        Ok(())
    }

    pub(crate) fn set_signal(&self, signal: Arc<WorkSignal>) {
        {
            let mut state = self.0.lock().expect("raft inbox poisoned");
            state.signal = Some(signal.clone());
        }
        // Covers messages admitted before driver assembly.
        signal.notify();
    }

    pub(crate) fn drain(&self) -> Vec<Message> {
        let mut out = Vec::new();
        let mut bytes = 0;
        let signal = {
            let mut state = self.0.lock().expect("raft inbox poisoned");
            while out.len() < DRAIN_MESSAGES && bytes < DRAIN_BYTES {
                let Some((message, weight)) = state.messages.pop_front() else {
                    break;
                };
                state.bytes -= weight;
                bytes += weight;
                out.push(message);
            }
            if state.messages.is_empty() {
                None
            } else {
                state.signal.clone()
            }
        };
        if let Some(signal) = signal {
            signal.notify();
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_between_lookup_and_wait_is_retained_and_exhaustion_cannot_wrap() {
        let signal = Arc::new(CompletionSignal::default());
        let observed = signal.observe().unwrap();
        signal.publish().unwrap();
        let waiter = signal.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let task = std::thread::spawn(move || {
            tx.send(waiter.wait(observed, Duration::from_secs(5)))
                .unwrap();
        });
        let result = rx.recv_timeout(Duration::from_secs(1));
        // Unblock even a deliberately faulty implementation before reporting
        // the assertion, so the failure does not leave an owned waiter behind.
        signal.publish().unwrap();
        task.join().unwrap();
        assert!(result.unwrap().is_ok(), "publication before park was lost");
        *signal.generation.lock().unwrap() = Some(u64::MAX - 1);
        signal.publish().unwrap();
        assert_eq!(signal.observe().unwrap(), u64::MAX);
        assert!(signal.publish().is_err());
        assert!(signal.observe().is_err());
        assert!(signal.wait(u64::MAX, Duration::from_secs(5)).is_err());
        assert!(signal.publish().is_err(), "exhausted generation restarted");
    }

    #[test]
    fn one_completion_wakes_all_registered_waiters() {
        let signal = Arc::new(CompletionSignal::default());
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let mut tasks = Vec::new();
        for _ in 0..4 {
            let parked = signal.observe_next_park();
            let signal = signal.clone();
            let done = done_tx.clone();
            tasks.push(std::thread::spawn(move || {
                let observed = signal.observe().unwrap();
                done.send(signal.wait(observed, Duration::from_secs(5)))
                    .unwrap();
            }));
            parked.recv_timeout(Duration::from_secs(1)).unwrap();
        }
        signal.publish().unwrap();
        let results: Vec<_> = (0..4)
            .map(|_| done_rx.recv_timeout(Duration::from_secs(1)))
            .collect();
        for _ in 0..4 {
            signal.publish().unwrap();
        }
        for task in tasks {
            task.join().unwrap();
        }
        assert!(
            results
                .into_iter()
                .all(|result| result.is_ok_and(|waited| waited.is_ok())),
            "one or more registered completion waiters were left parked"
        );
    }

    #[test]
    fn notification_survives_drain_and_coalescing_and_cannot_restart_a_stopped_owner() {
        let signal = WorkSignal::default();
        signal.notify();
        signal.notify();
        assert!(signal.begin_turn());
        assert!(!signal.state.lock().unwrap().pending);
        signal.notify(); // producer publishes while the owner is draining
        assert!(signal.state.lock().unwrap().pending);
        signal.wait_until(Instant::now() + Duration::from_secs(60));
        assert!(signal.begin_turn());
        signal.stop();
        signal.notify();
        signal.wait_until(Instant::now() + Duration::from_secs(60));
        assert!(!signal.begin_turn());
    }

    #[test]
    fn traffic_does_not_accelerate_or_postpone_tick_deadlines() {
        let start = Instant::now();
        let period = Duration::from_millis(20);
        let mut clock = TickDeadline::new(start, period);
        for millis in 0..20 {
            assert!(!clock.due(start + Duration::from_millis(millis)));
        }
        assert!(clock.due(start + period));
        assert!(!clock.due(start + period));
        assert!(clock.due(start + Duration::from_secs(60)));
        assert!(!clock.due(start + Duration::from_secs(60)));
        assert_eq!(clock.next(), start + Duration::from_secs(60) + period);
    }

    #[test]
    fn owner_poll_false_hint_preserves_already_published_work() {
        let signal = WorkSignal::default();
        signal.notify();
        signal.poll_hint.store(false, Ordering::Relaxed);
        signal.wait_until(Instant::now() + Duration::from_secs(60));
        assert!(signal.state.lock().unwrap().pending);
        assert!(signal.begin_turn());
    }

    #[test]
    fn owner_poll_true_hint_cannot_bypass_the_park_predicate() {
        let signal = Arc::new(WorkSignal::default());
        signal.poll_hint.store(true, Ordering::Relaxed);
        let parked = signal.observe_next_park();
        let copy = signal.clone();
        let owner = std::thread::spawn(move || {
            copy.wait_until(Instant::now() + Duration::from_secs(60));
            copy.begin_turn()
        });
        parked.recv_timeout(Duration::from_secs(2)).unwrap();
        signal.stop();
        assert!(!owner.join().unwrap());
    }

    fn owner_poll_interleaving(stale_hint: bool, stop: bool) {
        let signal = Arc::new(WorkSignal::default());
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        *signal.poll_observer.lock().unwrap() = Some((entered_tx, resume_rx));
        let parked = signal.observe_next_park();
        let copy = signal.clone();
        let owner = std::thread::spawn(move || {
            copy.wait_until(Instant::now() + Duration::from_secs(60));
            copy.begin_turn()
        });
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        // The producer must acquire the authoritative mutex while the owner
        // is in its polling phase; the observer holds no such mutex.
        if stop {
            signal.stop();
        } else {
            signal.notify();
        }
        if stale_hint {
            signal.poll_hint.store(false, Ordering::Relaxed);
        }
        resume_tx.send(()).unwrap();
        assert_eq!(owner.join().unwrap(), !stop);
        assert!(matches!(
            parked.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
    }

    #[test]
    fn owner_poll_observes_publication_without_parking() {
        owner_poll_interleaving(false, false);
    }

    #[test]
    fn owner_poll_stale_hint_falls_back_without_losing_publication() {
        owner_poll_interleaving(true, false);
    }

    #[test]
    fn owner_poll_stale_hint_cannot_prevent_stop() {
        owner_poll_interleaving(true, true);
    }

    #[test]
    fn inbox_bounds_and_retained_prefix_force_another_turn() {
        let inbox = RaftInbox::default();
        let signal = Arc::new(WorkSignal::default());
        inbox.set_signal(signal.clone());
        for index in 0..MAX_INBOX_MESSAGES {
            let message = Message {
                index: index as u64,
                ..Default::default()
            };
            inbox.send(message).unwrap();
        }
        assert!(inbox.send(Message::default()).is_err());
        assert!(signal.begin_turn());
        let first = inbox.drain();
        assert_eq!(first.len(), DRAIN_MESSAGES);
        assert_eq!(first.last().unwrap().index, (DRAIN_MESSAGES - 1) as u64);
        assert!(signal.state.lock().unwrap().pending);
        let mut seen = first.len();
        while seen < MAX_INBOX_MESSAGES {
            assert!(signal.begin_turn());
            seen += inbox.drain().len();
        }
        assert!(signal.begin_turn());
        assert!(inbox.drain().is_empty());
        assert!(!signal.state.lock().unwrap().pending);
        assert_eq!(inbox.0.lock().unwrap().bytes, 0);
    }
}
