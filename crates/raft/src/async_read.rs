//! Bounded asynchronous ReadIndex waiters, serviced by the existing Raft owner.
//! Registration never acquires the peer/persistence mutex. A sender completes
//! only after its exact context is confirmed and the unified apply covers it.

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use kv9_common::Error;
use tokio::sync::oneshot;

use crate::driver::{BarrierPhase, ReadIndexError};
use crate::work::WorkSignal;

type ReadResult = std::result::Result<u64, ReadIndexError>;
const MAX_REQUESTS: usize = 128;
const TURN_REQUESTS: usize = 64;

fn failed(message: &str) -> ReadIndexError {
    ReadIndexError::Failed(Error::Raft(message.into()))
}

fn expired(start: Instant, confirmed: bool) -> ReadIndexError {
    ReadIndexError::Unconfirmed {
        phase: if confirmed {
            BarrierPhase::ApplyCatchUp
        } else {
            BarrierPhase::QuorumConfirmation
        },
        waited: start.elapsed(),
    }
}

struct Request {
    context: [u8; 24],
    sender: Option<oneshot::Sender<ReadResult>>,
    confirmed: Option<u64>,
    quorum_observed: Arc<AtomicBool>,
    started: Instant,
    deadline: Instant,
    owner: Weak<Inner>,
}

impl Request {
    fn abandoned(&self) -> bool {
        self.sender.as_ref().is_none_or(|sender| sender.is_closed())
    }

    fn finish(mut self, result: ReadResult) {
        if let Some(sender) = self.sender.take() {
            let _ = sender.send(result);
        }
    }
}

impl Drop for Request {
    fn drop(&mut self) {
        // A callback unwind/owner destruction cannot leave a live receiver parked.
        if let Some(sender) = self.sender.take() {
            let _ = sender.send(Err(failed("read owner dropped a pending request")));
        }
        if let Some(owner) = self.owner.upgrade() {
            owner
                .state
                .lock()
                .expect("async read queue poisoned")
                .in_flight -= 1;
        }
    }
}

#[derive(Default)]
struct State {
    queued: VecDeque<Request>,
    active: BTreeMap<[u8; 24], Request>,
    in_flight: usize,
    peak: usize,
    stopped: bool,
}

struct Inner {
    state: Mutex<State>,
    signal: Arc<WorkSignal>,
}

#[derive(Clone)]
pub(crate) struct AsyncReads(Arc<Inner>);

#[derive(Debug, Clone, Copy)]
pub struct AsyncReadSnapshot {
    pub limit: usize,
    pub queued: usize,
    pub active: usize,
    pub in_flight: usize,
    pub peak: usize,
    pub stopped: bool,
}

pub(crate) struct ReadTicket {
    receiver: oneshot::Receiver<ReadResult>,
    quorum_observed: Arc<AtomicBool>,
    started: Instant,
    deadline: Instant,
    signal: Arc<WorkSignal>,
    finished: bool,
}

impl ReadTicket {
    pub(crate) async fn wait(mut self) -> ReadResult {
        tokio::select! {
            biased;
            result = &mut self.receiver => {
                self.finished = true;
                result.unwrap_or_else(|_| Err(failed("read owner lost its sender")))
            },
            _ = tokio::time::sleep_until(self.deadline.into()) => {
                Err(expired(self.started, self.quorum_observed.load(Ordering::Acquire)))
            }
        }
    }
}

impl Drop for ReadTicket {
    fn drop(&mut self) {
        // Close before waking the owner; otherwise it can miss cancellation.
        if !self.finished {
            self.receiver.close();
            self.signal.notify();
        }
    }
}

impl AsyncReads {
    pub(crate) fn new(signal: Arc<WorkSignal>) -> Self {
        Self(Arc::new(Inner {
            state: Mutex::default(),
            signal,
        }))
    }

    pub(crate) fn register(
        &self,
        context: [u8; 24],
        started: Instant,
        budget: Duration,
    ) -> std::result::Result<ReadTicket, ReadIndexError> {
        let deadline = started
            .checked_add(budget)
            .ok_or_else(|| failed("read deadline overflow"))?;
        let (sender, receiver) = oneshot::channel();
        let quorum_observed = Arc::new(AtomicBool::new(false));
        {
            let mut state = self.0.state.lock().expect("async read queue poisoned");
            if state.stopped {
                return Err(failed("read owner is stopped"));
            }
            if state.in_flight >= MAX_REQUESTS {
                return Err(failed("asynchronous read admission count limit reached"));
            }
            if Instant::now() >= deadline {
                return Err(expired(started, false));
            }
            // Contexts are minted by the driver's checked, process-unique counter.
            if state.active.contains_key(&context)
                || state.queued.iter().any(|r| r.context == context)
            {
                return Err(failed("duplicate asynchronous read context"));
            }
            state.in_flight += 1;
            state.peak = state.peak.max(state.in_flight);
            state.queued.push_back(Request {
                context,
                sender: Some(sender),
                confirmed: None,
                quorum_observed: quorum_observed.clone(),
                started,
                deadline,
                owner: Arc::downgrade(&self.0),
            });
        }
        self.0.signal.notify();
        Ok(ReadTicket {
            receiver,
            quorum_observed,
            started,
            deadline,
            signal: self.0.signal.clone(),
            finished: false,
        })
    }

    /// One owner only. Callbacks run without the queue lock. Deferred admission
    /// retains its context and absolute deadline; it does not create a quorum receipt.
    pub(crate) fn submit(&self, mut read_index: impl FnMut(Vec<u8>) -> kv9_common::Result<bool>) {
        // Snapshot the available prefix: requeued unready requests are not
        // retried repeatedly within this turn.
        let (count, retained) = {
            let state = self.0.state.lock().expect("async read queue poisoned");
            let count = state.queued.len().min(TURN_REQUESTS);
            (count, state.queued.len() > count)
        };
        let mut deferred = false;
        for _ in 0..count {
            let request = {
                let mut state = self.0.state.lock().expect("async read queue poisoned");
                if state.stopped {
                    break;
                }
                state.queued.pop_front()
            };
            let Some(request) = request else {
                break;
            };
            if request.abandoned() {
                continue;
            }
            if Instant::now() >= request.deadline {
                let error = expired(request.started, false);
                request.finish(Err(error));
                continue;
            }
            let admitted = read_index(request.context.to_vec());
            let admitted = match admitted {
                Ok(value) => value,
                Err(Error::NotLeader { leader }) => {
                    request.finish(Err(ReadIndexError::NotLeader { hint: leader }));
                    continue;
                }
                Err(error) => {
                    request.finish(Err(ReadIndexError::Failed(error)));
                    continue;
                }
            };
            let mut state = self.0.state.lock().expect("async read queue poisoned");
            if state.stopped {
                drop(state);
                request.finish(Err(failed("read owner stopped during admission")));
            } else if admitted {
                // No other owner inserts while this callback is running.
                let previous = state.active.insert(request.context, request);
                drop(state);
                assert!(
                    previous.is_none(),
                    "unique asynchronous read context replaced"
                );
            } else {
                deferred = true;
                state.queued.push_back(request);
            }
        }
        // A full prefix can retain uninspected requests. An unready short
        // prefix waits for real Raft work or its tick, avoiding an idle spin.
        if retained && !deferred {
            self.0.signal.notify();
        }
    }

    /// First exact-context confirmation wins. Other request contexts, including
    /// synchronous callers, remain the responsibility of their original path.
    pub(crate) fn confirm(&self, context: &[u8], index: u64) -> bool {
        let Ok(context) = <[u8; 24]>::try_from(context) else {
            return false;
        };
        let mut state = self.0.state.lock().expect("async read queue poisoned");
        let Some(request) = state.active.get_mut(&context) else {
            return false;
        };
        if request.confirmed.is_none() {
            request.confirmed = Some(index);
            request.quorum_observed.store(true, Ordering::Release);
        }
        true
    }

    /// Called after the whole pump iteration succeeds. Selection under the
    /// registry mutex is completion eligibility; a later stop preserves it.
    /// A partially applied failed Ready cannot authorize reads.
    pub(crate) fn complete(&self, applied: Option<u64>) {
        for (request, result) in self.select_completions(applied) {
            request.finish(result);
        }
    }

    fn select_completions(&self, applied: Option<u64>) -> Vec<(Request, ReadResult)> {
        let mut state = self.0.state.lock().expect("async read queue poisoned");
        let keys: Vec<_> = state
            .active
            .iter()
            .filter_map(|(key, request)| {
                (request.abandoned()
                    || Instant::now() >= request.deadline
                    || request
                        .confirmed
                        .is_some_and(|index| applied.is_some_and(|at| at >= index)))
                .then_some(*key)
            })
            .collect();
        keys.into_iter()
            .map(|key| {
                let request = state.active.remove(&key).expect("selected request");
                let result = if let Some(index) = request
                    .confirmed
                    .filter(|index| applied.is_some_and(|at| at >= *index))
                {
                    Ok(index)
                } else {
                    Err(expired(request.started, request.confirmed.is_some()))
                };
                (request, result)
            })
            .collect()
    }

    pub(crate) fn close(&self) {
        let (queued, active) = {
            let mut state = self.0.state.lock().expect("async read queue poisoned");
            state.stopped = true;
            (
                std::mem::take(&mut state.queued),
                std::mem::take(&mut state.active),
            )
        };
        for request in queued.into_iter().chain(active.into_values()) {
            request.finish(Err(failed("read owner is stopped")));
        }
    }

    pub(crate) fn snapshot(&self) -> AsyncReadSnapshot {
        let state = self.0.state.lock().expect("async read queue poisoned");
        AsyncReadSnapshot {
            limit: MAX_REQUESTS,
            queued: state.queued.len(),
            active: state.active.len(),
            in_flight: state.in_flight,
            peak: state.peak,
            stopped: state.stopped,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn context(id: u64) -> [u8; 24] {
        let mut context = [3; 24];
        context[16..].copy_from_slice(&id.to_be_bytes());
        context
    }
    fn register(queue: &AsyncReads, id: u64) -> ReadTicket {
        queue
            .register(context(id), Instant::now(), Duration::from_secs(5))
            .unwrap()
    }

    #[tokio::test]
    async fn exact_first_confirmation_and_apply_coverage_are_both_required() {
        let queue = AsyncReads::new(Arc::default());
        let mut first = register(&queue, 1);
        let mut second = register(&queue, 2);
        queue.submit(|_| Ok(true));
        assert!(!queue.confirm(&context(9), 1));
        assert!(queue.confirm(&context(1), 7));
        assert!(queue.confirm(&context(1), 3));
        queue.complete(Some(6));
        assert!(
            matches!(
                first.receiver.try_recv(),
                Err(oneshot::error::TryRecvError::Empty)
            ),
            "confirmation was downgraded or apply coverage bypassed"
        );
        queue.complete(Some(7));
        assert_eq!(first.wait().await.unwrap(), 7);
        assert!(
            matches!(
                second.receiver.try_recv(),
                Err(oneshot::error::TryRecvError::Empty)
            ),
            "one context authorized another reader"
        );
        queue.close();
        assert!(matches!(
            second.wait().await,
            Err(ReadIndexError::Failed(_))
        ));
        assert_eq!(queue.snapshot().in_flight, 0);
    }

    #[tokio::test]
    async fn deadlines_keep_quorum_and_apply_phases_without_renewal() {
        for confirmed in [false, true] {
            let queue = AsyncReads::new(Arc::default());
            let mut ticket = register(&queue, 1);
            queue.submit(|_| Ok(true));
            if confirmed {
                queue.confirm(&context(1), 9);
            }
            ticket.deadline = Instant::now();
            assert!(
                matches!(ticket.wait().await, Err(ReadIndexError::Unconfirmed { phase, .. }) if phase == if confirmed { BarrierPhase::ApplyCatchUp } else { BarrierPhase::QuorumConfirmation })
            );
            assert_eq!(
                queue.snapshot().in_flight,
                1,
                "receiver expiry released owner storage"
            );
            queue.complete(Some(0));
            assert_eq!(queue.snapshot().in_flight, 0);
        }
    }

    #[tokio::test]
    async fn cancellation_never_submits_and_capacity_is_bounded_until_owner_cleanup() {
        let queue = AsyncReads::new(Arc::default());
        let tickets: Vec<_> = (0..MAX_REQUESTS as u64)
            .map(|id| register(&queue, id))
            .collect();
        assert!(
            matches!(
                queue.register(context(999), Instant::now(), Duration::from_secs(5)),
                Err(ReadIndexError::Failed(_))
            ),
            "read capacity was bypassed"
        );
        drop(tickets);
        assert_eq!(queue.snapshot().in_flight, MAX_REQUESTS);
        queue.submit(|_| panic!("cancelled read reached Raft"));
        assert_eq!(
            queue.snapshot().in_flight,
            MAX_REQUESTS - TURN_REQUESTS,
            "cancelled requests bypassed the turn budget"
        );
        queue.submit(|_| panic!("cancelled read reached Raft"));
        assert_eq!(queue.snapshot().in_flight, 0);
    }

    #[tokio::test]
    async fn stop_reaches_unclaimed_suffix_while_one_callback_is_blocked() {
        let queue = AsyncReads::new(Arc::default());
        let first = register(&queue, 1);
        let second = register(&queue, 2);
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let worker_queue = queue.clone();
        let worker = std::thread::spawn(move || {
            let mut calls = 0;
            worker_queue.submit(|_| {
                calls += 1;
                assert!(
                    worker_queue.0.state.try_lock().is_ok(),
                    "callback holds the registry guard"
                );
                entered_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                Ok(true)
            });
            calls
        });
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        queue.close();
        assert!(
            matches!(second.wait().await, Err(ReadIndexError::Failed(_))),
            "unclaimed suffix survived stop"
        );
        assert_eq!(
            queue.snapshot().in_flight,
            1,
            "claimed storage was released before callback completion"
        );
        release_tx.send(()).unwrap();
        assert_eq!(
            worker.join().unwrap(),
            1,
            "stop admitted an unclaimed suffix"
        );
        assert!(matches!(first.wait().await, Err(ReadIndexError::Failed(_))));
        assert_eq!(queue.snapshot().in_flight, 0);
    }

    #[tokio::test]
    async fn callback_unwind_and_owner_destruction_complete_live_receivers() {
        let queue = AsyncReads::new(Arc::default());
        let ticket = register(&queue, 1);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            queue.submit(|_| panic!("injected read admission unwind"));
        }))
        .is_err());
        assert!(matches!(
            ticket.wait().await,
            Err(ReadIndexError::Failed(_))
        ));
        assert_eq!(queue.snapshot().in_flight, 0);
        let ticket = register(&queue, 2);
        drop(queue);
        assert!(matches!(
            ticket.wait().await,
            Err(ReadIndexError::Failed(_))
        ));
    }

    #[test]
    fn deferred_admission_has_a_finite_turn_and_does_not_self_spin() {
        let signal = Arc::new(WorkSignal::default());
        let queue = AsyncReads::new(signal.clone());
        let tickets: Vec<_> = (0..MAX_REQUESTS as u64)
            .map(|id| register(&queue, id))
            .collect();
        assert!(signal.begin_turn());
        let mut calls = 0;
        queue.submit(|_| {
            calls += 1;
            Ok(false)
        });
        assert_eq!(
            calls, TURN_REQUESTS,
            "unready admission retried within the same turn"
        );
        assert_eq!(queue.snapshot().queued, MAX_REQUESTS);
        let parked = signal.observe_next_park();
        let sleeper_signal = signal.clone();
        let sleeper = std::thread::spawn(move || {
            sleeper_signal.wait_until(Instant::now() + Duration::from_secs(5))
        });
        let observed = parked.recv_timeout(Duration::from_secs(2));
        signal.notify();
        sleeper.join().unwrap();
        assert!(
            observed.is_ok(),
            "deferred admission spun without new Raft work"
        );
        drop(tickets);
        queue.close();
    }

    #[tokio::test]
    async fn selected_completion_survives_later_stop_and_late_observation() {
        let queue = AsyncReads::new(Arc::default());
        let mut selected = register(&queue, 1);
        let rejected = register(&queue, 2);
        queue.submit(|_| Ok(true));
        queue.confirm(&context(1), 7);
        let completions = queue.select_completions(Some(7));
        assert_eq!(completions.len(), 1);
        queue.close();
        assert!(matches!(
            rejected.wait().await,
            Err(ReadIndexError::Failed(_))
        ));
        assert_eq!(
            queue.snapshot().in_flight,
            1,
            "stop released a selected completion"
        );
        for (request, result) in completions {
            request.finish(result);
        }
        selected.deadline = Instant::now();
        assert_eq!(
            selected.wait().await.unwrap(),
            7,
            "later stop or deadline erased a known eligible result"
        );
        assert_eq!(queue.snapshot().in_flight, 0);
    }
}
