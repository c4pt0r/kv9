//! One bounded envelope queue, polled directly by the active tonic body.
//!
//! Queue access, receive-waker ownership and RPC invalidation share one lock.
//! A body can outlive its RPC future, so route identity alone is insufficient:
//! every RPC receives a fresh token. The independent owner watches backlog age;
//! it never polls the body's receiver or relies on HTTP/2 polling a timer.

use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use tokio::sync::Notify;
use tokio::time::Instant;
use tokio_stream::Stream;

use super::{pb, RootDigest, STREAM_PROGRESS_BUDGET};
use super::{OutboundMessage, PeerDestination, MAX_BATCH_BYTES, MAX_BATCH_MSGS, PEER_QUEUE};

struct State {
    messages: VecDeque<OutboundMessage>,
    sender_open: bool,
    receiver_open: bool,
    active: Option<Arc<()>>,
    body_waker: Option<Waker>,
    // First queued work, or latest valid dequeue while backlog remains.
    // Repeated sends, stale inspection and empty polls do not reset it.
    pending_since: Option<Instant>,
}

struct Shared {
    state: Mutex<State>,
    changed: Notify,
}

pub(super) struct Sender(Arc<Shared>);
pub(super) struct Receiver(Arc<Shared>);

pub(super) fn channel() -> (Sender, Receiver) {
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            messages: VecDeque::new(),
            sender_open: true,
            receiver_open: true,
            active: None,
            body_waker: None,
            pending_since: None,
        }),
        changed: Notify::new(),
    });
    (Sender(shared.clone()), Receiver(shared))
}

impl Sender {
    pub(super) fn try_send(&self, message: OutboundMessage) -> Result<(), ()> {
        let waker = {
            let mut state = self.0.state.lock().expect("peer body queue poisoned");
            if !state.receiver_open || state.messages.len() == PEER_QUEUE {
                return Err(());
            }
            let newly_pending = state.messages.is_empty();
            if newly_pending {
                state.pending_since = Some(Instant::now());
            }
            state.messages.push_back(message);
            state.body_waker.take()
        };
        // Never invoke an arbitrary waker while holding the queue lock.
        if let Some(waker) = waker {
            waker.wake();
        }
        // The independent watchdog's idle check is already due no later than
        // this newly queued work's deadline. Only the body needs a send wake.
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn capacity(&self) -> usize {
        PEER_QUEUE - self.0.state.lock().unwrap().messages.len()
    }
}

impl Drop for Sender {
    fn drop(&mut self) {
        let waker = {
            let mut state = self.0.state.lock().expect("peer body queue poisoned");
            state.sender_open = false;
            state.body_waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
        self.0.changed.notify_one();
    }
}

impl Receiver {
    pub(super) fn sender_closed(&self) -> bool {
        !self
            .0
            .state
            .lock()
            .expect("peer body queue poisoned")
            .sender_open
    }

    pub(super) fn discard_outage(&self) {
        let discarded = {
            let mut state = self.0.state.lock().expect("peer body queue poisoned");
            state.pending_since = None;
            std::mem::take(&mut state.messages)
        };
        // At most PEER_QUEUE envelopes; release their buffers outside the lock.
        drop(discarded);
        self.0.changed.notify_one();
    }

    pub(super) fn open(
        &self,
        destination: Arc<PeerDestination>,
        root_digest: RootDigest,
    ) -> (Session, Body) {
        let token = Arc::new(());
        let old_waker = {
            let mut state = self.0.state.lock().expect("peer body queue poisoned");
            state.active = Some(token.clone());
            // A newly connected RPC gets its own progress budget; time spent
            // connecting/backing off was covered by those separate contracts.
            state.pending_since = (!state.messages.is_empty()).then(Instant::now);
            state.body_waker.take()
        };
        if let Some(waker) = old_waker {
            waker.wake();
        }
        (
            Session {
                shared: self.0.clone(),
                token: token.clone(),
            },
            Body {
                shared: self.0.clone(),
                token,
                destination,
                root_digest,
            },
        )
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        let (waker, discarded) = {
            let mut state = self.0.state.lock().expect("peer body queue poisoned");
            state.receiver_open = false;
            state.active = None;
            state.pending_since = None;
            (state.body_waker.take(), std::mem::take(&mut state.messages))
        };
        drop(discarded);
        if let Some(waker) = waker {
            waker.wake();
        }
        self.0.changed.notify_one();
    }
}

fn owns(state: &State, token: &Arc<()>) -> bool {
    state.receiver_open
        && state
            .active
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, token))
}

fn invalidate(shared: &Shared, token: &Arc<()>) {
    let waker = {
        let mut state = shared.state.lock().expect("peer body queue poisoned");
        if !owns(&state, token) {
            return;
        }
        state.active = None;
        state.body_waker.take()
    };
    if let Some(waker) = waker {
        waker.wake();
    }
    shared.changed.notify_one();
}

/// Constructed before handing Body to tonic; Drop fences retained bodies even
/// when their first poll never happened. The peer worker owns this guard.
pub(super) struct Session {
    shared: Arc<Shared>,
    token: Arc<()>,
}

impl Session {
    pub(super) async fn stalled(&self) {
        loop {
            // Register BEFORE inspecting ownership and closure. Lifecycle
            // changes retain a permit or wake us; sends only wake the body.
            let changed = self.shared.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let deadline = {
                let state = self.shared.state.lock().expect("peer body queue poisoned");
                if !owns(&state, &self.token) || (!state.sender_open && state.messages.is_empty()) {
                    return;
                }
                // Capture time under the same lock as the empty predicate.
                // Any later first enqueue at e has now+B <= e+B. An idle
                // check therefore observes it without a producer Notify.
                let now = Instant::now();
                let deadline = state.pending_since.map(|at| at + STREAM_PROGRESS_BUDGET);
                // An expired backlog is terminal even if notification remains
                // continuously ready; expiry must not depend on select order.
                if deadline.is_some_and(|at| now >= at) {
                    return;
                }
                deadline.unwrap_or(now + STREAM_PROGRESS_BUDGET)
            };
            // Keep this Sleep across ordinary polls. An idle expiry only
            // rechecks current state; it never declares an empty queue stalled.
            tokio::select! {
                _ = &mut changed => {},
                _ = tokio::time::sleep_until(deadline) => {
                    let state = self.shared.state.lock().expect("peer body queue poisoned");
                    if !owns(&state, &self.token) || state.pending_since.is_some_and(|at| Instant::now() >= at + STREAM_PROGRESS_BUDGET) {
                        return;
                    }
                },
            }
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        invalidate(&self.shared, &self.token);
    }
}

pub(super) struct Body {
    shared: Arc<Shared>,
    token: Arc<()>,
    destination: Arc<PeerDestination>,
    root_digest: RootDigest,
}

impl Stream for Body {
    type Item = pb::BatchRaftMessage;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Replacing Tokio's receiver also removes its cooperative-budget
        // charge. Bound consecutive ready batches, not only each batch's size.
        // Poll outside the mutex: exhausted budget can register/wake a task.
        let coop = std::task::ready!(tokio::task::coop::poll_proceed(cx));
        let mut state = self.shared.state.lock().expect("peer body queue poisoned");
        if !owns(&state, &self.token) {
            coop.made_progress();
            return Poll::Ready(None);
        }
        let mut batch = Vec::new();
        let mut bytes = 0;
        let mut inspected = 0;
        while inspected < MAX_BATCH_MSGS && bytes < MAX_BATCH_BYTES {
            let Some(message) = state.messages.pop_front() else {
                break;
            };
            inspected += 1;
            if Arc::ptr_eq(&message.destination, &self.destination) {
                bytes += message.envelope.raft_message.len();
                batch.push(message.envelope);
            }
        }
        if state.messages.is_empty() {
            state.pending_since = None;
        } else if !batch.is_empty() {
            state.pending_since = Some(Instant::now());
        }
        let result = if !batch.is_empty() {
            Poll::Ready(Some(pb::BatchRaftMessage {
                msgs: batch,
                flushed_unix_nanos: 0,
                root_digest: self.root_digest.as_bytes().to_vec(),
            }))
        } else if state.messages.is_empty() && !state.sender_open {
            Poll::Ready(None)
        } else {
            state.body_waker = Some(cx.waker().clone());
            Poll::Pending
        };
        // If a bounded stale-only prefix exhausted the turn, there may be no
        // future producer event. Arrange continuation after releasing the lock.
        let continue_stale = result.is_pending() && !state.messages.is_empty();
        drop(state);
        if inspected != 0 || result.is_ready() {
            coop.made_progress();
        }
        if continue_stale {
            cx.waker().wake_by_ref();
        }
        result
    }
}

impl Drop for Body {
    fn drop(&mut self) {
        invalidate(&self.shared, &self.token);
    }
}

#[cfg(test)]
mod tests;
