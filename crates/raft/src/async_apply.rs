//! Bounded exact-apply waiters serviced by the existing Raft owner. These
//! reservations cover submission, queued waits and completion handoffs.

use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use kv9_common::Error;
use tokio::sync::oneshot;

use crate::driver::{ApplyWaitError, ApplyWaitOutcome};
use crate::rawnode::ProposedAt;
use crate::work::WorkSignal;

pub(crate) type ApplyResult = Result<ApplyWaitOutcome, ApplyWaitError>;
const MAX_REQUESTS: usize = 128;

fn failed(message: &str) -> ApplyWaitError {
    ApplyWaitError::Failed(Error::Raft(message.into()))
}

fn expired(at: ProposedAt, waited: Duration) -> ApplyWaitError {
    ApplyWaitError::Unconfirmed {
        index: at.index.0,
        waited,
    }
}

#[derive(Default)]
struct State {
    queued: Vec<Request>,
    in_flight: usize,
    peak: usize,
    stopped: bool,
}

struct Inner {
    state: Mutex<State>,
    signal: Arc<WorkSignal>,
}

#[derive(Clone)]
pub(crate) struct AsyncApplies(Arc<Inner>);

#[derive(Debug, Clone, Copy)]
pub struct AsyncApplySnapshot {
    pub limit: usize,
    pub queued: usize,
    pub in_flight: usize,
    pub peak: usize,
    pub stopped: bool,
}

pub(crate) struct Reservation {
    owner: Weak<Inner>,
    deadline: Instant,
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if let Some(owner) = self.owner.upgrade() {
            let mut state = owner.state.lock().expect("async apply queue poisoned");
            assert!(state.in_flight > 0, "apply reservation missing");
            state.in_flight -= 1;
        }
    }
}

struct Request {
    at: ProposedAt,
    started: Instant,
    deadline: Instant,
    sender: Option<oneshot::Sender<ApplyResult>>,
    reservation: Option<Reservation>,
}

impl Request {
    fn finish(mut self, result: ApplyResult) {
        let sender = self.sender.take();
        // A completed replacement may immediately reserve its next attempt.
        drop(self.reservation.take());
        if let Some(sender) = sender {
            let _ = sender.send(result);
        }
    }
}

impl Drop for Request {
    fn drop(&mut self) {
        drop(self.reservation.take());
        if let Some(sender) = self.sender.take() {
            let _ = sender.send(Err(failed("apply owner dropped a pending request")));
        }
    }
}

pub(crate) struct ApplyTicket {
    receiver: oneshot::Receiver<ApplyResult>,
    at: ProposedAt,
    started: Instant,
    deadline: Instant,
    signal: Arc<WorkSignal>,
    finished: bool,
}

impl ApplyTicket {
    pub(crate) async fn wait(mut self) -> ApplyResult {
        tokio::select! {
            biased;
            result = &mut self.receiver => {
                self.finished = true;
                result.unwrap_or_else(|_| Err(failed("apply owner lost its sender")))
            }
            _ = tokio::time::sleep_until(self.deadline.into()) => {
                Err(expired(self.at, self.deadline.saturating_duration_since(self.started)))
            }
        }
    }
}

impl Drop for ApplyTicket {
    fn drop(&mut self) {
        // Closing before notifying covers timeout/cancellation while the owner
        // has temporarily extracted this request for a completion handoff.
        if !self.finished {
            self.receiver.close();
            self.signal.notify();
        }
    }
}

impl Reservation {
    pub(crate) fn register(self, at: ProposedAt) -> Result<ApplyTicket, ApplyWaitError> {
        let owner = self
            .owner
            .upgrade()
            .ok_or_else(|| failed("apply owner is gone"))?;
        let started = Instant::now();
        let deadline = self.deadline;
        let (sender, receiver) = oneshot::channel();
        let mut state = owner.state.lock().expect("async apply queue poisoned");
        if state.stopped {
            drop(state);
            return Err(failed("apply owner is stopped after proposal"));
        }
        state.queued.push(Request {
            at,
            started,
            deadline,
            sender: Some(sender),
            reservation: Some(self),
        });
        drop(state);
        // Includes apply-before-registration: a fresh owner turn inspects the
        // retained exact receipt even when no further Raft traffic arrives.
        owner.signal.notify();
        Ok(ApplyTicket {
            receiver,
            at,
            started,
            deadline,
            signal: owner.signal.clone(),
            finished: false,
        })
    }
}

impl AsyncApplies {
    pub(crate) fn new(signal: Arc<WorkSignal>) -> Self {
        Self(Arc::new(Inner {
            state: Mutex::default(),
            signal,
        }))
    }

    pub(crate) fn reserve(&self, deadline: Instant) -> Result<Reservation, Error> {
        let mut state = self.0.state.lock().expect("async apply queue poisoned");
        if state.stopped {
            return Err(Error::Raft("asynchronous apply owner is stopped".into()));
        }
        if state.in_flight >= MAX_REQUESTS {
            return Err(Error::Raft(
                "asynchronous apply admission count limit reached before proposal".into(),
            ));
        }
        if Instant::now() >= deadline {
            return Err(Error::Raft(
                "asynchronous proposal deadline exhausted before submission".into(),
            ));
        }
        state.in_flight += 1;
        state.peak = state.peak.max(state.in_flight);
        Ok(Reservation {
            owner: Arc::downgrade(&self.0),
            deadline,
        })
    }

    pub(crate) fn service(
        &self,
        mut inspect: impl FnMut(ProposedAt, Duration) -> Option<ApplyResult>,
    ) {
        let requests = {
            let mut state = self.0.state.lock().expect("async apply queue poisoned");
            std::mem::take(&mut state.queued)
        };
        if requests.is_empty() {
            return;
        }
        let mut pending = Vec::with_capacity(requests.len());
        for request in requests {
            if request
                .sender
                .as_ref()
                .is_none_or(|sender| sender.is_closed())
            {
                continue;
            }
            if let Some(result) = inspect(request.at, request.started.elapsed()) {
                request.finish(result);
            } else if Instant::now() >= request.deadline {
                let result = Err(expired(request.at, request.started.elapsed()));
                request.finish(result);
            } else {
                pending.push(request);
            }
        }
        let mut state = self.0.state.lock().expect("async apply queue poisoned");
        if !state.stopped {
            state.queued.append(&mut pending);
        }
        drop(state);
        // Stop raced the extracted handoff: drop/send failure outside the lock.
        drop(pending);
    }

    pub(crate) fn close(&self) {
        let requests = {
            let mut state = self.0.state.lock().expect("async apply queue poisoned");
            state.stopped = true;
            std::mem::take(&mut state.queued)
        };
        for request in requests {
            request.finish(Err(failed("asynchronous apply owner stopped or failed")));
        }
    }

    pub(crate) fn snapshot(&self) -> AsyncApplySnapshot {
        let state = self.0.state.lock().expect("async apply queue poisoned");
        AsyncApplySnapshot {
            limit: MAX_REQUESTS,
            queued: state.queued.len(),
            in_flight: state.in_flight,
            peak: state.peak,
            stopped: state.stopped,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LogIndex;
    use kv9_common::AppliedPosition;

    fn at() -> ProposedAt {
        ProposedAt {
            term: 7,
            index: LogIndex(19),
        }
    }
    fn deadline() -> Instant {
        Instant::now() + Duration::from_secs(5)
    }

    #[tokio::test]
    async fn admission_covers_unsubmitted_and_extracted_waits() {
        let owner = AsyncApplies::new(Arc::new(WorkSignal::default()));
        let mut held: Vec<_> = (0..MAX_REQUESTS)
            .map(|_| owner.reserve(deadline()).unwrap())
            .collect();
        assert!(
            owner.reserve(deadline()).is_err(),
            "unsubmitted reservations exceeded the bound"
        );
        let ticket = held.pop().unwrap().register(at()).unwrap();
        owner.service(|position, _| {
            assert_eq!(position, at());
            assert!(
                owner.reserve(deadline()).is_err(),
                "completion handoff released capacity early"
            );
            Some(Ok(ApplyWaitOutcome::Applied(AppliedPosition {
                term: 7,
                index: 19,
            })))
        });
        assert!(matches!(
            ticket.wait().await,
            Ok(ApplyWaitOutcome::Applied(_))
        ));
        assert_eq!(owner.snapshot().in_flight, MAX_REQUESTS - 1);
        drop(held);
        assert_eq!(owner.snapshot().in_flight, 0);
    }

    #[tokio::test]
    async fn cancellation_timeout_and_close_drain_without_fabricated_receipts() {
        let owner = AsyncApplies::new(Arc::new(WorkSignal::default()));
        let canceled = owner.reserve(deadline()).unwrap().register(at()).unwrap();
        drop(canceled);
        assert_eq!(
            owner.snapshot().in_flight,
            1,
            "receiver drop released an owner handoff"
        );
        owner.service(|_, _| panic!("an abandoned waiter reached receipt inspection"));
        assert_eq!(owner.snapshot().in_flight, 0);

        let timed = owner
            .reserve(Instant::now() + Duration::from_millis(20))
            .unwrap()
            .register(at())
            .unwrap();
        assert!(
            matches!(
                timed.wait().await,
                Err(ApplyWaitError::Unconfirmed { index: 19, .. })
            ),
            "deadline fabricated an apply receipt"
        );
        owner.service(|_, _| panic!("expired receiver was still live"));
        assert_eq!(owner.snapshot().in_flight, 0);

        let closed = owner.reserve(deadline()).unwrap().register(at()).unwrap();
        owner.service(|_, _| {
            owner.close();
            None
        });
        assert!(matches!(
            closed.wait().await,
            Err(ApplyWaitError::Failed(_))
        ));
        assert!(owner.reserve(deadline()).is_err());
        assert_eq!(
            owner.snapshot().in_flight,
            0,
            "stop lost an extracted reservation"
        );
    }

    #[tokio::test]
    async fn owner_destruction_resolves_pending_receivers() {
        let owner = AsyncApplies::new(Arc::new(WorkSignal::default()));
        let ticket = owner.reserve(deadline()).unwrap().register(at()).unwrap();
        drop(owner);
        let outcome = tokio::time::timeout(Duration::from_secs(1), ticket.wait())
            .await
            .unwrap();
        assert!(
            matches!(outcome, Err(ApplyWaitError::Failed(_))),
            "owner loss left a false or unresolved receipt"
        );
    }
}
