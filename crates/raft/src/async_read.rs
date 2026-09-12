//! Bounded asynchronous ReadIndex waiters, serviced by the existing Raft owner.
//! Registration never acquires the peer/persistence mutex. A sender completes
//! only after its sealed group's exact context is confirmed and unified apply covers it.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use kv9_common::Error;
use tokio::sync::oneshot;

use crate::driver::{BarrierPhase, ReadIndexError};
use crate::work::WorkSignal;

type ReadResult = std::result::Result<u64, ReadIndexError>;
#[cfg(not(feature = "read-stage-timing"))]
type ReadReply = ReadResult;
#[cfg(feature = "read-stage-timing")]
struct ReadReply {
    result: ReadResult,
    timeline: crate::read_stage_timing::Timeline,
}
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
    sender: Option<oneshot::Sender<ReadReply>>,
    quorum_observed: Arc<AtomicBool>,
    started: Instant,
    deadline: Instant,
    owner: Weak<Inner>,
    #[cfg(feature = "read-stage-timing")]
    timeline: crate::read_stage_timing::Timeline,
}

impl Request {
    fn abandoned(&self) -> bool {
        self.sender.as_ref().is_none_or(|sender| sender.is_closed())
    }

    fn finish(mut self, result: ReadResult) {
        if let Some(sender) = self.sender.take() {
            #[cfg(feature = "read-stage-timing")]
            let result = {
                self.timeline.sending = Some(Instant::now());
                ReadReply {
                    result,
                    timeline: self.timeline,
                }
            };
            let _ = sender.send(result);
        }
    }
}

impl Drop for Request {
    fn drop(&mut self) {
        // A callback unwind/owner destruction cannot leave a live receiver parked.
        if let Some(sender) = self.sender.take() {
            let result = Err(failed("read owner dropped a pending request"));
            #[cfg(feature = "read-stage-timing")]
            let result = ReadReply {
                result,
                timeline: self.timeline,
            };
            let _ = sender.send(result);
        }
        if let Some(owner) = self.owner.upgrade() {
            let mut state = owner.state.lock().expect("async read queue poisoned");
            assert!(
                state.reserved.remove(&self.context),
                "read reservation missing"
            );
            state.in_flight -= 1;
        }
    }
}

struct ReadGroup {
    // Membership is sealed before invoking ReadIndex. Removing canceled or
    // expired members never changes the map key or admits a later caller.
    members: Vec<Request>,
    confirmed: Option<u64>,
}

#[derive(Default)]
struct State {
    queued: VecDeque<Request>,
    active: BTreeMap<[u8; 24], ReadGroup>,
    reserved: BTreeSet<[u8; 24]>,
    in_flight: usize,
    local_reserved: usize,
    peak: usize,
    inspected: u64,
    group_attempts: u64,
    admitted_groups: u64,
    admitted_members: u64,
    max_admitted_group: usize,
    stopped: bool,
}

struct Inner {
    state: Mutex<State>,
    signal: Arc<WorkSignal>,
    #[cfg(feature = "read-stage-timing")]
    timing: Arc<crate::read_stage_timing::ReadStageMetrics>,
}

#[derive(Clone)]
pub(crate) struct AsyncReads(Arc<Inner>);

#[derive(Debug, Clone, Copy)]
pub struct AsyncReadSnapshot {
    pub limit: usize,
    pub queued: usize,
    pub active: usize,
    pub active_groups: usize,
    pub in_flight: usize,
    pub local_reserved: usize,
    pub peak: usize,
    pub inspected: u64,
    pub group_attempts: u64,
    pub admitted_groups: u64,
    pub admitted_members: u64,
    pub max_admitted_group: usize,
    pub stopped: bool,
}

pub(crate) struct ReadTicket {
    receiver: oneshot::Receiver<ReadReply>,
    quorum_observed: Arc<AtomicBool>,
    started: Instant,
    deadline: Instant,
    signal: Arc<WorkSignal>,
    finished: bool,
    #[cfg(feature = "read-stage-timing")]
    timing: Arc<crate::read_stage_timing::ReadStageMetrics>,
}

/// The same bounded admission slot can either finish a local lease attempt or
/// become a queued Safe ReadIndex request, without resetting its deadline.
#[cfg(any(test, feature = "experimental-leader-lease"))]
pub(crate) struct LocalReadReservation {
    owner: Arc<Inner>,
    context: [u8; 24],
    started: Instant,
    deadline: Instant,
    retained: bool,
}

#[cfg(any(test, feature = "experimental-leader-lease"))]
impl LocalReadReservation {
    pub(crate) fn submit(mut self) -> std::result::Result<ReadTicket, ReadIndexError> {
        let (sender, receiver) = oneshot::channel();
        let quorum_observed = Arc::new(AtomicBool::new(false));
        {
            let mut state = self.owner.state.lock().expect("async read queue poisoned");
            if state.stopped {
                return Err(failed("read owner is stopped"));
            }
            if Instant::now() >= self.deadline {
                return Err(expired(self.started, false));
            }
            assert!(
                state.reserved.contains(&self.context),
                "read reservation missing"
            );
            state.queued.push_back(Request {
                context: self.context,
                sender: Some(sender),
                quorum_observed: quorum_observed.clone(),
                started: self.started,
                deadline: self.deadline,
                owner: Arc::downgrade(&self.owner),
                #[cfg(feature = "read-stage-timing")]
                timeline: crate::read_stage_timing::Timeline::new(Instant::now()),
            });
            state.local_reserved -= 1;
            self.retained = false;
        }
        self.owner.signal.notify();
        Ok(ReadTicket {
            receiver,
            quorum_observed,
            started: self.started,
            deadline: self.deadline,
            signal: self.owner.signal.clone(),
            finished: false,
            #[cfg(feature = "read-stage-timing")]
            timing: self.owner.timing.clone(),
        })
    }
}

#[cfg(any(test, feature = "experimental-leader-lease"))]
impl Drop for LocalReadReservation {
    fn drop(&mut self) {
        if self.retained {
            let mut state = self.owner.state.lock().expect("async read queue poisoned");
            assert!(
                state.reserved.remove(&self.context),
                "read reservation missing"
            );
            state.local_reserved -= 1;
            state.in_flight -= 1;
        }
    }
}

impl ReadTicket {
    pub(crate) async fn wait(mut self) -> ReadResult {
        tokio::select! {
            biased;
            result = &mut self.receiver => {
                self.finished = true;
                #[cfg(feature = "read-stage-timing")]
                let result = result.map(|reply| {
                    let resumed = Instant::now();
                    if reply.result.is_ok() {
                        self.timing.record(reply.timeline, self.started, resumed);
                    }
                    reply.result
                });
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
            #[cfg(feature = "read-stage-timing")]
            timing: Arc::default(),
        }))
    }

    #[cfg(feature = "read-stage-timing")]
    pub(crate) fn timing(&self) -> Arc<crate::read_stage_timing::ReadStageMetrics> {
        self.0.timing.clone()
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
            if state.active.contains_key(&context) || !state.reserved.insert(context) {
                return Err(failed("duplicate asynchronous read context"));
            }
            state.in_flight += 1;
            state.peak = state.peak.max(state.in_flight);
            state.queued.push_back(Request {
                context,
                sender: Some(sender),
                quorum_observed: quorum_observed.clone(),
                started,
                deadline,
                owner: Arc::downgrade(&self.0),
                #[cfg(feature = "read-stage-timing")]
                timeline: crate::read_stage_timing::Timeline::new(Instant::now()),
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
            #[cfg(feature = "read-stage-timing")]
            timing: self.0.timing.clone(),
        })
    }

    #[cfg(any(test, feature = "experimental-leader-lease"))]
    pub(crate) fn reserve_local(
        &self,
        context: [u8; 24],
        started: Instant,
        deadline: Instant,
    ) -> std::result::Result<LocalReadReservation, ReadIndexError> {
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
        if state.active.contains_key(&context) || !state.reserved.insert(context) {
            return Err(failed("duplicate asynchronous read context"));
        }
        state.in_flight += 1;
        state.local_reserved += 1;
        state.peak = state.peak.max(state.in_flight);
        Ok(LocalReadReservation {
            owner: self.0.clone(),
            context,
            started,
            deadline,
            retained: true,
        })
    }

    /// One owner only. Seal a bounded prefix before invoking ReadIndex once.
    /// Deferred admission has not initiated a quorum read; members may be
    /// regrouped on retry, retaining their identities and absolute deadlines.
    pub(crate) fn submit(&self, mut read_index: impl FnMut(Vec<u8>) -> kv9_common::Result<bool>) {
        let (prefix, retained) = {
            let mut state = self.0.state.lock().expect("async read queue poisoned");
            if state.stopped {
                return;
            }
            let count = state.queued.len().min(TURN_REQUESTS);
            state.inspected = state.inspected.saturating_add(count as u64);
            let prefix: Vec<_> = state.queued.drain(..count).collect();
            (prefix, !state.queued.is_empty())
        };
        let mut members = Vec::with_capacity(prefix.len());
        for request in prefix {
            if request.abandoned() {
                continue;
            }
            if Instant::now() >= request.deadline {
                let error = expired(request.started, false);
                request.finish(Err(error));
                continue;
            }
            members.push(request);
        }
        let mut deferred = false;
        if let Some(first) = members.first() {
            // The checked invocation context is unique, and the sealed group
            // retains it independently of the first member's later lifetime.
            let context = first.context;
            {
                let mut state = self.0.state.lock().expect("async read queue poisoned");
                if state.stopped {
                    drop(state);
                    for request in members {
                        request.finish(Err(failed("read owner stopped before group admission")));
                    }
                    return;
                }
                state.group_attempts = state.group_attempts.saturating_add(1);
            }
            #[cfg(feature = "read-stage-timing")]
            let issued = Instant::now();
            let admitted = read_index(context.to_vec());
            let admitted = match admitted {
                Ok(value) => Some(value),
                Err(Error::NotLeader { leader }) => {
                    for request in members.drain(..) {
                        request.finish(Err(ReadIndexError::NotLeader { hint: leader }));
                    }
                    None
                }
                Err(error) => {
                    // Peer admission's non-routing failure is terminal Raft
                    // failure. Each member receives its own typed read error.
                    let message = error.to_string();
                    for request in members.drain(..) {
                        request.finish(Err(failed(&message)));
                    }
                    None
                }
            };
            let mut state = self.0.state.lock().expect("async read queue poisoned");
            if admitted == Some(true) {
                state.admitted_groups = state.admitted_groups.saturating_add(1);
                state.admitted_members =
                    state.admitted_members.saturating_add(members.len() as u64);
                state.max_admitted_group = state.max_admitted_group.max(members.len());
            }
            if state.stopped {
                drop(state);
                for request in members {
                    request.finish(Err(failed("read owner stopped during admission")));
                }
            } else if admitted == Some(true) {
                #[cfg(feature = "read-stage-timing")]
                for request in &mut members {
                    request.timeline.issued = Some(issued);
                }
                let previous = state.active.insert(
                    context,
                    ReadGroup {
                        members,
                        confirmed: None,
                    },
                );
                drop(state);
                assert!(
                    previous.is_none(),
                    "unique asynchronous read group replaced"
                );
            } else if admitted == Some(false) {
                deferred = true;
                state.queued.extend(members);
            }
        }
        // A full prefix can retain uninspected requests. An unready short
        // prefix waits for real Raft work or its tick, avoiding an idle spin.
        if retained && !deferred {
            self.0.signal.notify();
        }
    }

    /// First exact-group-context confirmation wins. Other contexts, including
    /// synchronous callers, remain the responsibility of their original path.
    pub(crate) fn confirm(&self, context: &[u8], index: u64) -> bool {
        let Ok(context) = <[u8; 24]>::try_from(context) else {
            return false;
        };
        let mut state = self.0.state.lock().expect("async read queue poisoned");
        let Some(group) = state.active.get_mut(&context) else {
            return false;
        };
        if group.confirmed.is_none() {
            group.confirmed = Some(index);
            #[cfg(feature = "read-stage-timing")]
            let confirmed = Instant::now();
            for request in &mut group.members {
                request.quorum_observed.store(true, Ordering::Release);
                #[cfg(feature = "read-stage-timing")]
                {
                    request.timeline.confirmed = Some(confirmed);
                }
            }
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
        let mut selected = Vec::new();
        state.active.retain(|_, group| {
            let covered = group
                .confirmed
                .filter(|index| applied.is_some_and(|at| at >= *index));
            let mut pos = 0;
            while pos < group.members.len() {
                let request = &group.members[pos];
                if covered.is_some() || request.abandoned() || Instant::now() >= request.deadline {
                    let request = group.members.swap_remove(pos);
                    let result =
                        covered.ok_or_else(|| expired(request.started, group.confirmed.is_some()));
                    selected.push((request, result));
                } else {
                    pos += 1;
                }
            }
            !group.members.is_empty()
        });
        selected
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
        for request in queued
            .into_iter()
            .chain(active.into_values().flat_map(|g| g.members))
        {
            request.finish(Err(failed("read owner is stopped")));
        }
    }

    pub(crate) fn snapshot(&self) -> AsyncReadSnapshot {
        let state = self.0.state.lock().expect("async read queue poisoned");
        AsyncReadSnapshot {
            limit: MAX_REQUESTS,
            queued: state.queued.len(),
            active: state.active.values().map(|g| g.members.len()).sum(),
            active_groups: state.active.len(),
            in_flight: state.in_flight,
            local_reserved: state.local_reserved,
            peak: state.peak,
            inspected: state.inspected,
            group_attempts: state.group_attempts,
            admitted_groups: state.admitted_groups,
            admitted_members: state.admitted_members,
            max_admitted_group: state.max_admitted_group,
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

    #[test]
    fn local_reads_share_capacity_and_contexts_with_queued_reads() {
        let queue = AsyncReads::new(Arc::default());
        let started = Instant::now();
        let deadline = started + Duration::from_secs(5);
        let locals: Vec<_> = (0..MAX_REQUESTS as u64 / 2)
            .map(|id| queue.reserve_local(context(id), started, deadline).unwrap())
            .collect();
        assert!(queue
            .register(context(0), started, Duration::from_secs(5))
            .is_err());
        let queued: Vec<_> = (MAX_REQUESTS as u64 / 2..MAX_REQUESTS as u64)
            .map(|id| register(&queue, id))
            .collect();
        assert!(queue
            .reserve_local(context(999), started, deadline)
            .is_err());
        assert!(queue
            .register(context(999), started, Duration::from_secs(5))
            .is_err());
        assert_eq!(queue.snapshot().in_flight, MAX_REQUESTS);
        assert_eq!(queue.snapshot().local_reserved, MAX_REQUESTS / 2);
        drop(locals);
        assert_eq!(queue.snapshot().in_flight, MAX_REQUESTS / 2);
        assert_eq!(queue.snapshot().local_reserved, 0);
        drop(queued);
        queue.submit(|_| panic!("canceled request was submitted"));
        assert_eq!(queue.snapshot().in_flight, 0);
    }

    #[tokio::test]
    async fn local_fallback_keeps_one_slot_original_context_and_absolute_deadline() {
        let queue = AsyncReads::new(Arc::default());
        let started = Instant::now() - Duration::from_secs(2);
        let deadline = started + Duration::from_secs(5);
        let local = queue.reserve_local(context(1), started, deadline).unwrap();
        let ticket = local.submit().unwrap();
        assert_eq!(
            (ticket.started, ticket.deadline),
            (started, deadline),
            "local fallback reset the original request budget"
        );
        let snapshot = queue.snapshot();
        assert_eq!(
            (snapshot.in_flight, snapshot.local_reserved, snapshot.queued),
            (1, 0, 1)
        );
        assert!(queue.reserve_local(context(1), started, deadline).is_err());
        queue.submit(|ctx| {
            assert_eq!(ctx, context(1));
            Ok(true)
        });
        assert!(queue.confirm(&context(1), 7));
        queue.complete(Some(7));
        assert_eq!(ticket.wait().await.unwrap(), 7);
        assert_eq!(queue.snapshot().in_flight, 0);
    }

    #[test]
    fn expired_or_stopped_local_fallback_releases_its_slot_without_admission() {
        for stopped in [false, true] {
            let queue = AsyncReads::new(Arc::default());
            let started = Instant::now();
            let mut local = queue
                .reserve_local(context(1), started, started + Duration::from_secs(5))
                .unwrap();
            if stopped {
                queue.close();
                assert_eq!(queue.snapshot().in_flight, 1);
            } else {
                local.deadline = Instant::now();
            }
            let result = local.submit();
            if stopped {
                assert!(matches!(result, Err(ReadIndexError::Failed(_))));
            } else {
                assert!(matches!(
                    result,
                    Err(ReadIndexError::Unconfirmed {
                        phase: BarrierPhase::QuorumConfirmation,
                        ..
                    })
                ));
            }
            let snapshot = queue.snapshot();
            assert_eq!(
                (snapshot.in_flight, snapshot.local_reserved, snapshot.queued),
                (0, 0, 0)
            );
        }
    }

    #[test]
    fn canceled_local_fallback_keeps_owner_storage_until_bounded_cleanup() {
        let queue = AsyncReads::new(Arc::default());
        let started = Instant::now();
        let local = queue
            .reserve_local(context(1), started, started + Duration::from_secs(5))
            .unwrap();
        drop(local.submit().unwrap());
        assert_eq!(queue.snapshot().in_flight, 1);
        queue.submit(|_| panic!("canceled fallback reached Raft"));
        assert_eq!(queue.snapshot().in_flight, 0);
    }

    #[cfg(feature = "read-stage-timing")]
    #[tokio::test]
    async fn timing_requires_the_same_sealed_confirmation_apply_and_successful_delivery() {
        use kv9_common::metrics::Outcome;

        let queue = AsyncReads::new(Arc::default());
        let first = register(&queue, 1);
        let second = register(&queue, 2);
        queue.submit(|_| Ok(false));
        let mut late = None;
        queue.submit(|ctx| {
            assert_eq!(ctx, context(1));
            late = Some(register(&queue, 3));
            Ok(true)
        });
        drop(first);
        assert!(!queue.confirm(&context(2), 9));
        assert!(queue.confirm(&context(1), 9));
        queue.complete(Some(8));
        assert!(queue
            .timing()
            .snapshots()
            .iter()
            .all(|metric| { metric.latency.outcomes.iter().all(|h| h.count == 0) }));
        queue.complete(Some(9));
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), second.wait())
                .await
                .unwrap()
                .unwrap(),
            9
        );
        queue.close();
        assert!(late.unwrap().wait().await.is_err());
        let snapshots = queue.timing().snapshots();
        let sums: Vec<_> = snapshots
            .iter()
            .map(|metric| {
                let success = &metric.latency.outcomes[Outcome::Success as usize];
                assert_eq!(success.count, 1);
                assert!(metric.latency.outcomes[1..].iter().all(|h| h.count == 0));
                success.sum_ns
            })
            .collect();
        assert_eq!(sums[..5].iter().sum::<u64>(), sums[5]);
        assert_eq!(queue.snapshot().in_flight, 0);
    }

    #[tokio::test]
    async fn sealed_members_share_one_confirmation_and_late_arrivals_require_another() {
        let queue = AsyncReads::new(Arc::default());
        let first = register(&queue, 1);
        let second = register(&queue, 2);
        let mut late = None;
        let mut calls = Vec::new();
        queue.submit(|ctx| {
            calls.push(ctx);
            late = Some(register(&queue, 3));
            Ok(true)
        });
        assert_eq!(
            calls,
            vec![context(1).to_vec()],
            "sealed group emitted more than one ReadIndex"
        );
        assert_eq!(
            queue.snapshot().queued,
            1,
            "arrival joined a group after its quorum request started"
        );
        assert_eq!(queue.snapshot().active, 2);
        let mut late = late.unwrap();
        queue.submit(|ctx| {
            assert_eq!(ctx, context(3));
            Ok(true)
        });
        assert!(
            !queue.confirm(&context(2), 1),
            "member identity was accepted as a group confirmation"
        );
        queue.confirm(&context(1), 7);
        queue.complete(Some(7));
        assert_eq!(first.wait().await.unwrap(), 7);
        assert_eq!(second.wait().await.unwrap(), 7);
        assert!(
            matches!(
                late.receiver.try_recv(),
                Err(oneshot::error::TryRecvError::Empty)
            ),
            "late arrival borrowed an earlier group's confirmation"
        );
        queue.confirm(&context(3), 8);
        queue.complete(Some(7));
        assert!(
            matches!(
                late.receiver.try_recv(),
                Err(oneshot::error::TryRecvError::Empty)
            ),
            "late group bypassed its own apply coverage"
        );
        queue.complete(Some(8));
        assert_eq!(late.wait().await.unwrap(), 8);
        let state = queue.snapshot();
        assert_eq!(
            (
                state.admitted_groups,
                state.admitted_members,
                state.max_admitted_group
            ),
            (2, 3, 2)
        );
        assert_eq!(state.in_flight, 0);
    }

    #[tokio::test]
    async fn canceled_representative_leaves_group_identity_for_its_live_members() {
        let queue = AsyncReads::new(Arc::default());
        let representative = register(&queue, 1);
        let survivor = register(&queue, 2);
        queue.submit(|_| Ok(true));
        drop(representative);
        queue.complete(Some(0));
        assert_eq!(queue.snapshot().in_flight, 1);
        assert_eq!(queue.snapshot().active_groups, 1);
        assert!(
            queue
                .register(context(1), Instant::now(), Duration::from_secs(1))
                .is_err(),
            "a live group's context was reused after representative cancellation"
        );
        assert!(
            queue.confirm(&context(1), 7),
            "representative cancellation destroyed confirmation routing"
        );
        queue.complete(Some(7));
        assert_eq!(survivor.wait().await.unwrap(), 7);
        assert_eq!(queue.snapshot().in_flight, 0);
    }

    #[tokio::test]
    async fn group_members_keep_independent_deadlines_and_capacity() {
        let queue = AsyncReads::new(Arc::default());
        let mut expired_member = register(&queue, 1);
        let mut live_member = register(&queue, 2);
        queue.submit(|_| Ok(true));
        queue.confirm(&context(1), 7);
        expired_member.deadline = Instant::now();
        assert!(matches!(
            expired_member.wait().await,
            Err(ReadIndexError::Unconfirmed {
                phase: BarrierPhase::ApplyCatchUp,
                ..
            })
        ));
        assert_eq!(
            queue.snapshot().in_flight,
            2,
            "one member's timeout released owner storage"
        );
        queue.complete(Some(6));
        assert_eq!(queue.snapshot().in_flight, 1);
        assert!(
            matches!(
                live_member.receiver.try_recv(),
                Err(oneshot::error::TryRecvError::Empty)
            ),
            "one member's deadline completed a live sibling"
        );
        queue.confirm(&context(1), 3);
        queue.complete(Some(6));
        assert!(
            matches!(
                live_member.receiver.try_recv(),
                Err(oneshot::error::TryRecvError::Empty)
            ),
            "duplicate group confirmation lowered a live member's barrier"
        );
        queue.complete(Some(7));
        assert_eq!(live_member.wait().await.unwrap(), 7);
        assert_eq!(queue.snapshot().in_flight, 0);
    }

    #[tokio::test]
    async fn full_queue_emits_two_bounded_groups_and_retains_every_member() {
        let queue = AsyncReads::new(Arc::default());
        let tickets: Vec<_> = (0..MAX_REQUESTS as u64)
            .map(|id| register(&queue, id))
            .collect();
        let mut calls = 0;
        queue.submit(|_| {
            calls += 1;
            Ok(true)
        });
        let first = queue.snapshot();
        assert_eq!(
            (calls, first.active, first.active_groups, first.queued),
            (1, TURN_REQUESTS, 1, MAX_REQUESTS - TURN_REQUESTS),
            "sealed group exceeded its inspection or membership bound"
        );
        queue.submit(|_| {
            calls += 1;
            Ok(true)
        });
        let full = queue.snapshot();
        assert_eq!(
            (calls, full.active, full.active_groups, full.queued),
            (2, MAX_REQUESTS, 2, 0)
        );
        assert_eq!(
            (
                full.inspected,
                full.admitted_groups,
                full.admitted_members,
                full.max_admitted_group
            ),
            (MAX_REQUESTS as u64, 2, MAX_REQUESTS as u64, TURN_REQUESTS)
        );
        queue.close();
        for ticket in tickets {
            assert!(matches!(
                ticket.wait().await,
                Err(ReadIndexError::Failed(_))
            ));
        }
        assert_eq!(queue.snapshot().in_flight, 0);
    }

    #[tokio::test]
    async fn exact_first_confirmation_and_apply_coverage_are_both_required() {
        let queue = AsyncReads::new(Arc::default());
        let mut first = register(&queue, 1);
        queue.submit(|_| Ok(true));
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
        let claimed: Vec<_> = (0..TURN_REQUESTS as u64)
            .map(|id| register(&queue, id))
            .collect();
        let unclaimed = register(&queue, TURN_REQUESTS as u64);
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
            matches!(unclaimed.wait().await, Err(ReadIndexError::Failed(_))),
            "unclaimed suffix survived stop"
        );
        assert_eq!(
            queue.snapshot().in_flight,
            TURN_REQUESTS,
            "claimed storage was released before callback completion"
        );
        release_tx.send(()).unwrap();
        assert_eq!(
            worker.join().unwrap(),
            1,
            "stop admitted an unclaimed suffix"
        );
        for member in claimed {
            assert!(matches!(
                member.wait().await,
                Err(ReadIndexError::Failed(_))
            ));
        }
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
        assert_eq!(calls, 1, "unready admission retried within the same turn");
        assert_eq!(
            queue.snapshot().inspected,
            TURN_REQUESTS as u64,
            "grouping bypassed the owner inspection bound"
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
        queue.submit(|_| Ok(true));
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
