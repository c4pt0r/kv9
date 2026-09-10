//! Bounded notification and inbound ownership for the event-driven Raft pump.
//!
//! Producers publish work before notifying. The owner consumes the coalesced
//! notification before draining, and checks the same mutex-protected predicate
//! when parking. A notification during a drain therefore survives that drain.
//! Notifications do not advance Raft time and grant no persistence authority.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use protobuf::Message as _;
use raft::eraftpb::Message;

#[derive(Debug, Default)]
struct State {
    pending: bool,
    stopped: bool,
}

#[derive(Debug, Default)]
pub struct WorkSignal {
    state: Mutex<State>,
    changed: Condvar,
    #[cfg(test)]
    park_observer: Mutex<Option<std::sync::mpsc::Sender<()>>>,
}

impl WorkSignal {
    pub fn notify(&self) {
        let mut state = self.state.lock().expect("work signal poisoned");
        if !state.stopped {
            state.pending = true;
            self.changed.notify_one();
        }
    }

    pub(crate) fn stop(&self) {
        let mut state = self.state.lock().expect("work signal poisoned");
        state.stopped = true;
        self.changed.notify_all();
    }

    /// Consume BEFORE reading work queues. No work/peer lock nests inside this
    /// guard. Returns false once the owner must stop.
    pub(crate) fn begin_turn(&self) -> bool {
        let mut state = self.state.lock().expect("work signal poisoned");
        state.pending = false;
        !state.stopped
    }

    pub(crate) fn wait_until(&self, deadline: Instant) {
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
