//! Per-owner proposal admission, atomic cancellation and bounded submission turns.
//!
//! No queue or receipt guard is held while calling Raft. The queue mutex orders
//! stop against claim; the receipt mutex orders cancellation against claim.
//! Count/byte reservations include dequeued work until submission actually returns.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::Instant;

use kv9_common::{Error, ProposalRefusal, Result};

use crate::work::WorkSignal;
use crate::ProposedAt;

const MAX_REQUESTS: usize = 128;
const MAX_BYTES: usize = 64 * 1024 * 1024;
const TURN_REQUESTS: usize = 64;
const TURN_BYTES: usize = 1024 * 1024;

enum Submission {
    Queued,
    Claimed,
    Cancelled,
    Finished(Option<Result<ProposedAt>>),
}

struct Receipt {
    state: Mutex<Submission>,
    changed: Condvar,
}

impl Receipt {
    fn finish(&self, result: Result<ProposedAt>) {
        let mut state = self.state.lock().expect("proposal receipt poisoned");
        if !matches!(*state, Submission::Cancelled) {
            *state = Submission::Finished(Some(result));
            self.changed.notify_one();
        }
    }
}

pub(crate) struct Ticket {
    receipt: Arc<Receipt>,
    deadline: Instant,
}

impl Ticket {
    pub(crate) fn wait(self) -> Result<ProposedAt> {
        let mut state = self
            .receipt
            .state
            .lock()
            .expect("proposal receipt poisoned");
        loop {
            if let Submission::Finished(result) = &mut *state {
                return result.take().expect("one submission waiter");
            }
            let remaining = self.deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return match *state {
                    Submission::Queued => {
                        *state = Submission::Cancelled;
                        Err(refused(ProposalRefusal::Expired))
                    }
                    // Once claim won, neither timeout nor stop can prove no effect.
                    Submission::Claimed => Err(Error::ProposalUnconfirmed),
                    Submission::Cancelled | Submission::Finished(_) => unreachable!(),
                };
            }
            state = self
                .receipt
                .changed
                .wait_timeout(state, remaining)
                .expect("proposal receipt poisoned")
                .0;
        }
    }
}

impl Drop for Ticket {
    fn drop(&mut self) {
        let mut state = self
            .receipt
            .state
            .lock()
            .expect("proposal receipt poisoned");
        if matches!(*state, Submission::Queued) {
            *state = Submission::Cancelled;
        }
    }
}

struct Reservation {
    queue: Weak<Inner>,
    bytes: usize,
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if let Some(queue) = self.queue.upgrade() {
            let mut state = queue.state.lock().expect("proposal queue poisoned");
            state.in_flight -= 1;
            state.bytes -= self.bytes;
        }
    }
}

struct Pending {
    receipt: Arc<Receipt>,
    data: Vec<u8>,
    expected_term: Option<u64>,
    deadline: Instant,
    reservation: Reservation,
}

impl Drop for Pending {
    fn drop(&mut self) {
        // A dropped claimed handoff may already have executed its effect.
        let mut state = self
            .receipt
            .state
            .lock()
            .expect("proposal receipt poisoned");
        if matches!(*state, Submission::Claimed) {
            *state = Submission::Finished(Some(Err(Error::ProposalUnconfirmed)));
            self.receipt.changed.notify_one();
        }
    }
}

#[derive(Default)]
struct State {
    pending: VecDeque<Pending>,
    in_flight: usize,
    bytes: usize,
    peak_requests: usize,
    peak_bytes: usize,
    stopped: bool,
}

struct Inner {
    state: Mutex<State>,
    signal: Arc<WorkSignal>,
    max_requests: usize,
    max_bytes: usize,
}

impl Drop for Inner {
    fn drop(&mut self) {
        for pending in self
            .state
            .get_mut()
            .expect("proposal queue poisoned")
            .pending
            .drain(..)
        {
            pending
                .receipt
                .finish(Err(refused(ProposalRefusal::Stopped)));
        }
    }
}

#[derive(Clone)]
pub(crate) struct ProposalQueue(Arc<Inner>);

/// Encoded command reservations, not total RSS or retained consensus-log bytes.
#[derive(Debug, Clone, Copy)]
pub struct ProposalQueueSnapshot {
    pub max_requests: usize,
    pub max_bytes: usize,
    pub queued: usize,
    pub in_flight: usize,
    pub encoded_bytes: usize,
    pub peak_requests: usize,
    pub peak_bytes: usize,
    pub stopped: bool,
}

impl ProposalQueue {
    pub(crate) fn new(signal: Arc<WorkSignal>) -> Self {
        Self::with_limits(signal, MAX_REQUESTS, MAX_BYTES)
    }

    fn with_limits(signal: Arc<WorkSignal>, max_requests: usize, max_bytes: usize) -> Self {
        assert!(max_requests > 0 && max_bytes > 0);
        Self(Arc::new(Inner {
            state: Mutex::new(State::default()),
            signal,
            max_requests,
            max_bytes,
        }))
    }

    pub(crate) fn enqueue(
        &self,
        data: Vec<u8>,
        expected_term: Option<u64>,
        deadline: Instant,
    ) -> Result<Ticket> {
        let receipt = Arc::new(Receipt {
            state: Mutex::new(Submission::Queued),
            changed: Condvar::new(),
        });
        {
            let mut state = self.0.state.lock().expect("proposal queue poisoned");
            if state.stopped {
                return Err(refused(ProposalRefusal::Stopped));
            }
            if Instant::now() >= deadline {
                return Err(refused(ProposalRefusal::Expired));
            }
            if data.len() > self.0.max_bytes {
                return Err(refused(ProposalRefusal::RequestTooLarge));
            }
            if state.in_flight >= self.0.max_requests {
                return Err(refused(ProposalRefusal::RequestCount));
            }
            if data.len() > self.0.max_bytes - state.bytes {
                return Err(refused(ProposalRefusal::EncodedBytes));
            }
            state.in_flight += 1;
            state.bytes += data.len();
            state.peak_requests = state.peak_requests.max(state.in_flight);
            state.peak_bytes = state.peak_bytes.max(state.bytes);
            state.pending.push_back(Pending {
                receipt: receipt.clone(),
                expected_term,
                deadline,
                reservation: Reservation {
                    queue: Arc::downgrade(&self.0),
                    bytes: data.len(),
                },
                data,
            });
        }
        self.0.signal.notify();
        Ok(Ticket { receipt, deadline })
    }

    /// Inspect a bounded prefix, including cancelled entries. One legal entry
    /// can cross the turn byte target. No wait is added to form a batch.
    pub(crate) fn drain(&self, mut submit: impl FnMut(Vec<u8>, Option<u64>) -> Result<ProposedAt>) {
        let mut bytes = 0;
        for _ in 0..TURN_REQUESTS {
            if bytes >= TURN_BYTES {
                break;
            }
            let next = {
                let mut queue = self.0.state.lock().expect("proposal queue poisoned");
                queue.pending.pop_front().map(|pending| {
                    let mut state = pending
                        .receipt
                        .state
                        .lock()
                        .expect("proposal receipt poisoned");
                    let claimed = if !matches!(*state, Submission::Queued) {
                        false
                    } else if queue.stopped || Instant::now() >= pending.deadline {
                        *state = Submission::Finished(Some(Err(refused(if queue.stopped {
                            ProposalRefusal::Stopped
                        } else {
                            ProposalRefusal::Expired
                        }))));
                        pending.receipt.changed.notify_one();
                        false
                    } else {
                        *state = Submission::Claimed;
                        true
                    };
                    drop(state);
                    (pending, claimed)
                })
            };
            let Some((mut pending, claimed)) = next else {
                break;
            };
            bytes += pending.reservation.bytes;
            if claimed {
                let result = submit(std::mem::take(&mut pending.data), pending.expected_term);
                pending.receipt.finish(result);
            }
            // Dropping this reservation occurs outside queue/receipt guards,
            // even if the caller abandoned its ticket or timed out after claim.
        }
        let retained = !self
            .0
            .state
            .lock()
            .expect("proposal queue poisoned")
            .pending
            .is_empty();
        if retained {
            self.0.signal.notify();
        }
    }

    pub(crate) fn close(&self) {
        let pending = {
            let mut state = self.0.state.lock().expect("proposal queue poisoned");
            state.stopped = true;
            std::mem::take(&mut state.pending)
        };
        for request in pending {
            request
                .receipt
                .finish(Err(refused(ProposalRefusal::Stopped)));
        }
    }

    pub(crate) fn snapshot(&self) -> ProposalQueueSnapshot {
        let state = self.0.state.lock().expect("proposal queue poisoned");
        ProposalQueueSnapshot {
            max_requests: self.0.max_requests,
            max_bytes: self.0.max_bytes,
            queued: state.pending.len(),
            in_flight: state.in_flight,
            encoded_bytes: state.bytes,
            peak_requests: state.peak_requests,
            peak_bytes: state.peak_bytes,
            stopped: state.stopped,
        }
    }
}

fn refused(reason: ProposalRefusal) -> Error {
    Error::ProposalRefused { reason }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LogIndex;
    use std::sync::mpsc;
    use std::time::Duration;

    fn later() -> Instant {
        Instant::now() + Duration::from_secs(5)
    }
    fn position(index: u64) -> ProposedAt {
        ProposedAt {
            term: 7,
            index: LogIndex(index),
        }
    }
    fn assert_refused(result: Result<ProposedAt>, expected: ProposalRefusal) {
        assert!(
            matches!(result, Err(Error::ProposalRefused { reason }) if reason == expected),
            "expected {expected:?}, got {result:?}"
        );
    }

    #[test]
    fn cancelled_and_expired_requests_never_reach_submission() {
        let queue = ProposalQueue::new(Arc::default());
        let mut cancelled = queue.enqueue(vec![1], None, later()).unwrap();
        cancelled.deadline = Instant::now();
        assert_refused(cancelled.wait(), ProposalRefusal::Expired);
        let dropped = queue.enqueue(vec![2], None, later()).unwrap();
        drop(dropped);
        let expired = queue.enqueue(vec![3], None, later()).unwrap();
        queue
            .0
            .state
            .lock()
            .unwrap()
            .pending
            .back_mut()
            .unwrap()
            .deadline = Instant::now();
        assert_eq!(
            queue.snapshot().in_flight,
            3,
            "cancelled queue storage was released early"
        );
        queue.drain(|_, _| panic!("cancelled or expired proposal reached Raft"));
        assert_refused(expired.wait(), ProposalRefusal::Expired);
        assert_eq!(queue.snapshot().in_flight, 0);
        assert_eq!(queue.snapshot().encoded_bytes, 0);
    }

    #[test]
    fn claimed_timeout_retains_capacity_until_real_submission_returns() {
        let queue = ProposalQueue::with_limits(Arc::default(), 1, 8);
        let mut ticket = queue.enqueue(vec![1; 8], Some(7), later()).unwrap();
        let (claimed_tx, claimed_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let worker_queue = queue.clone();
        let worker = std::thread::spawn(move || {
            worker_queue.drain(|data, expected| {
                assert_eq!(data, vec![1; 8]);
                assert_eq!(expected, Some(7));
                claimed_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                Ok(position(9))
            });
        });
        claimed_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        ticket.deadline = Instant::now();
        assert!(
            matches!(ticket.wait(), Err(Error::ProposalUnconfirmed)),
            "claimed timeout was falsely definite"
        );
        let snapshot = queue.snapshot();
        assert_eq!(
            (snapshot.queued, snapshot.in_flight, snapshot.encoded_bytes),
            (0, 1, 8)
        );
        assert!(matches!(
            queue.enqueue(vec![1], None, later()),
            Err(Error::ProposalRefused {
                reason: ProposalRefusal::RequestCount
            })
        ));
        queue.close();
        assert_eq!(
            queue.snapshot().in_flight,
            1,
            "stop released an already claimed reservation"
        );
        release_tx.send(()).unwrap();
        worker.join().unwrap();
        assert_eq!(
            (queue.snapshot().in_flight, queue.snapshot().encoded_bytes),
            (0, 0)
        );
    }

    #[test]
    fn count_bytes_and_oversize_refusals_do_not_change_the_ledger() {
        let queue = ProposalQueue::with_limits(Arc::default(), 2, 8);
        let first = queue.enqueue(vec![1; 5], None, later()).unwrap();
        assert!(
            matches!(
                queue.enqueue(vec![2; 4], None, later()),
                Err(Error::ProposalRefused {
                    reason: ProposalRefusal::EncodedBytes
                })
            ),
            "encoded-byte admission bound was bypassed"
        );
        assert!(matches!(
            queue.enqueue(vec![2; 9], None, later()),
            Err(Error::ProposalRefused {
                reason: ProposalRefusal::RequestTooLarge
            })
        ));
        let second = queue.enqueue(vec![2; 3], Some(7), later()).unwrap();
        assert!(
            matches!(
                queue.enqueue(vec![], None, later()),
                Err(Error::ProposalRefused {
                    reason: ProposalRefusal::RequestCount
                })
            ),
            "request-count admission bound was bypassed"
        );
        let snapshot = queue.snapshot();
        assert_eq!(
            (
                snapshot.in_flight,
                snapshot.encoded_bytes,
                snapshot.peak_requests,
                snapshot.peak_bytes
            ),
            (2, 8, 2, 8)
        );
        let mut index = 0;
        queue.drain(|_, _| {
            index += 1;
            Ok(position(index))
        });
        assert_eq!(first.wait().unwrap().index.0, 1);
        assert_eq!(second.wait().unwrap().index.0, 2);
        assert_eq!(
            (queue.snapshot().in_flight, queue.snapshot().encoded_bytes),
            (0, 0)
        );
    }

    #[test]
    fn cancelled_entries_consume_turn_budget_and_preserve_the_fifo_suffix() {
        let queue = ProposalQueue::new(Arc::default());
        for _ in 0..TURN_REQUESTS {
            drop(queue.enqueue(vec![], None, later()).unwrap());
        }
        let last = queue.enqueue(vec![9], None, later()).unwrap();
        queue.drain(|_, _| panic!("cancelled work bypassed the turn inspection bound"));
        assert_eq!(
            queue.snapshot().queued,
            1,
            "cancelled work bypassed the turn inspection bound"
        );
        queue.drain(|data, _| {
            assert_eq!(data, [9]);
            Ok(position(1))
        });
        assert_eq!(last.wait().unwrap().index.0, 1);
    }

    #[test]
    fn byte_target_allows_one_legal_overshoot_and_keeps_the_suffix() {
        let queue = ProposalQueue::new(Arc::default());
        let first = queue
            .enqueue(vec![1; TURN_BYTES + 1], None, later())
            .unwrap();
        let second = queue.enqueue(vec![2], None, later()).unwrap();
        let mut order = Vec::new();
        queue.drain(|data, _| {
            order.push(data[0]);
            Ok(position(1))
        });
        assert_eq!(
            order,
            [1],
            "proposal turn byte target did not retain its suffix"
        );
        assert_eq!(first.wait().unwrap().index.0, 1);
        assert_eq!(queue.snapshot().queued, 1);
        queue.drain(|data, _| {
            order.push(data[0]);
            Ok(position(2))
        });
        assert_eq!(order, [1, 2]);
        assert_eq!(second.wait().unwrap().index.0, 2);
    }

    #[test]
    fn stop_and_destruction_refuse_only_unclaimed_work() {
        let queue = ProposalQueue::new(Arc::default());
        let ticket = queue.enqueue(vec![1], None, later()).unwrap();
        queue.close();
        assert_refused(ticket.wait(), ProposalRefusal::Stopped);
        assert_eq!(
            (queue.snapshot().in_flight, queue.snapshot().encoded_bytes),
            (0, 0)
        );
        assert!(
            matches!(
                queue.enqueue(vec![2], None, later()),
                Err(Error::ProposalRefused {
                    reason: ProposalRefusal::Stopped
                })
            ),
            "stopped queue admitted new work"
        );
        queue.drain(|_, _| panic!("stopped owner submitted a new proposal"));
        let another = ProposalQueue::new(Arc::default());
        let ticket = another.enqueue(vec![3], None, later()).unwrap();
        drop(another);
        assert_refused(ticket.wait(), ProposalRefusal::Stopped);
    }

    #[test]
    fn submit_unwind_finishes_claimed_as_unknown_and_retains_unclaimed_suffix() {
        let queue = ProposalQueue::with_limits(Arc::default(), 2, 12);
        let first = queue.enqueue(vec![1; 8], None, later()).unwrap();
        let receipt = first.receipt.clone();
        let second = queue.enqueue(vec![2; 4], None, later()).unwrap();
        let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            queue.drain(|_, _| {
                assert!(
                    queue.0.state.try_lock().is_ok(),
                    "submission retained the queue guard"
                );
                assert!(
                    receipt.state.try_lock().is_ok(),
                    "submission retained the receipt guard"
                );
                assert_eq!(
                    queue.snapshot().in_flight,
                    2,
                    "submission released capacity early"
                );
                panic!("intentional submitted-effect unwind");
            });
        }));
        assert!(unwind.is_err());
        assert!(
            matches!(first.wait(), Err(Error::ProposalUnconfirmed)),
            "unwound claim became a definite refusal"
        );
        let retained = queue.snapshot();
        assert_eq!(
            (retained.queued, retained.in_flight, retained.encoded_bytes),
            (1, 1, 4)
        );
        queue.drain(|data, _| {
            assert_eq!(data, [2; 4]);
            Ok(position(41))
        });
        assert_eq!(second.wait().unwrap(), position(41));
        assert_eq!(
            (queue.snapshot().in_flight, queue.snapshot().encoded_bytes),
            (0, 0)
        );
    }

    #[test]
    fn close_inside_submission_refuses_suffix_but_preserves_exact_claimed_result() {
        let queue = ProposalQueue::new(Arc::default());
        let first = queue.enqueue(vec![1], Some(7), later()).unwrap();
        let receipt = first.receipt.clone();
        let second = queue.enqueue(vec![2], None, later()).unwrap();
        let (claimed_tx, claimed_rx) = mpsc::channel();
        let (close_tx, close_rx) = mpsc::channel();
        let (closed_tx, closed_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let worker_queue = queue.clone();
        let worker = std::thread::spawn(move || {
            worker_queue.drain(|data, expected_term| {
                assert_eq!((data, expected_term), (vec![1], Some(7)));
                assert!(
                    worker_queue.0.state.try_lock().is_ok(),
                    "submission retained the queue guard"
                );
                assert!(
                    receipt.state.try_lock().is_ok(),
                    "submission retained the receipt guard"
                );
                claimed_tx.send(()).unwrap();
                close_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                worker_queue.close();
                closed_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                Ok(position(41))
            });
            done_tx.send(()).unwrap();
        });
        claimed_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let (waiting_tx, waiting_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let waiter = std::thread::spawn(move || {
            waiting_tx.send(()).unwrap();
            result_tx.send(first.wait()).unwrap();
        });
        waiting_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        close_tx.send(()).unwrap();
        closed_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_refused(second.wait(), ProposalRefusal::Stopped);
        assert!(
            matches!(result_rx.try_recv(), Err(mpsc::TryRecvError::Empty)),
            "stop completed a still-claimed waiter"
        );
        let retained = queue.snapshot();
        assert_eq!(
            (retained.queued, retained.in_flight, retained.encoded_bytes),
            (0, 1, 1)
        );
        release_tx.send(()).unwrap();
        let result = result_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(result.unwrap(), position(41));
        done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        worker.join().unwrap();
        waiter.join().unwrap();
        assert_eq!(
            (queue.snapshot().in_flight, queue.snapshot().encoded_bytes),
            (0, 0)
        );
    }

    #[test]
    fn claimed_ticket_drop_does_not_release_in_progress_reservation() {
        let queue = ProposalQueue::with_limits(Arc::default(), 2, 12);
        let first = queue.enqueue(vec![1; 8], None, later()).unwrap();
        let second = queue.enqueue(vec![2; 4], None, later()).unwrap();
        let (claimed_tx, claimed_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let worker_queue = queue.clone();
        let worker = std::thread::spawn(move || {
            worker_queue.drain(|data, _| {
                assert_eq!(data, [1; 8]);
                claimed_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                Ok(position(41))
            });
            done_tx.send(()).unwrap();
        });
        claimed_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        drop(first);
        queue.close();
        assert_refused(second.wait(), ProposalRefusal::Stopped);
        let retained = queue.snapshot();
        assert_eq!(
            (retained.in_flight, retained.encoded_bytes),
            (1, 8),
            "claimed reservation was released before actual submission returned"
        );
        release_tx.send(()).unwrap();
        done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        worker.join().unwrap();
        assert_eq!(
            (queue.snapshot().in_flight, queue.snapshot().encoded_bytes),
            (0, 0)
        );
    }

    #[test]
    fn cancel_and_claim_race_has_only_one_winner() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        for _ in 0..256 {
            let queue = ProposalQueue::new(Arc::default());
            let ticket = queue.enqueue(vec![1], None, later()).unwrap();
            let receipt = ticket.receipt.clone();
            let effects = Arc::new(AtomicUsize::new(0));
            let (ready_tx, ready_rx) = mpsc::channel();
            let (start_tx, start_rx) = mpsc::channel();
            let (done_tx, done_rx) = mpsc::channel();
            let worker_queue = queue.clone();
            let worker_effects = effects.clone();
            let worker = std::thread::spawn(move || {
                ready_tx.send(()).unwrap();
                start_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                worker_queue.drain(|_, _| {
                    worker_effects.fetch_add(1, Ordering::SeqCst);
                    Ok(position(41))
                });
                done_tx.send(()).unwrap();
            });
            ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            start_tx.send(()).unwrap();
            drop(ticket);
            done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            worker.join().unwrap();
            match &*receipt.state.lock().unwrap() {
                Submission::Cancelled => assert_eq!(effects.load(Ordering::SeqCst), 0),
                Submission::Finished(Some(Ok(at))) => {
                    assert_eq!(*at, position(41));
                    assert_eq!(effects.load(Ordering::SeqCst), 1);
                }
                _ => panic!("cancellation/claim race left an unexpected receipt"),
            }
            let drained = queue.snapshot();
            assert_eq!(
                (drained.queued, drained.in_flight, drained.encoded_bytes),
                (0, 0, 0)
            );
        }
    }

    #[test]
    fn stop_and_claim_race_never_reports_false_definite_refusal() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        for _ in 0..256 {
            let queue = ProposalQueue::new(Arc::default());
            let ticket = queue.enqueue(vec![1], None, later()).unwrap();
            let effects = Arc::new(AtomicUsize::new(0));
            let (ready_tx, ready_rx) = mpsc::channel();
            let (start_tx, start_rx) = mpsc::channel();
            let (done_tx, done_rx) = mpsc::channel();
            let worker_queue = queue.clone();
            let worker_effects = effects.clone();
            let worker = std::thread::spawn(move || {
                ready_tx.send(()).unwrap();
                start_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                worker_queue.drain(|_, _| {
                    worker_effects.fetch_add(1, Ordering::SeqCst);
                    Ok(position(41))
                });
                done_tx.send(()).unwrap();
            });
            ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            start_tx.send(()).unwrap();
            queue.close();
            done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            worker.join().unwrap();
            match ticket.wait() {
                Ok(at) => {
                    assert_eq!(at, position(41));
                    assert_eq!(effects.load(Ordering::SeqCst), 1);
                }
                Err(Error::ProposalRefused {
                    reason: ProposalRefusal::Stopped,
                }) => {
                    assert_eq!(effects.load(Ordering::SeqCst), 0);
                }
                other => panic!("stop/claim race returned an unexpected result: {other:?}"),
            }
            assert_eq!(
                (queue.snapshot().in_flight, queue.snapshot().encoded_bytes),
                (0, 0)
            );
        }
    }

    #[test]
    fn already_finished_receipt_wins_over_late_wait_observation() {
        let queue = ProposalQueue::new(Arc::default());
        let mut ticket = queue.enqueue(vec![1], None, later()).unwrap();
        queue.drain(|_, _| Ok(position(41)));
        // Make observation expire without a scheduler-sensitive wall-clock sleep.
        ticket.deadline = Instant::now();
        assert_eq!(
            ticket.wait().unwrap(),
            position(41),
            "known receipt was discarded after waiter deadline"
        );
    }
}
