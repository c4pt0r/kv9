//! Real Raft admission-credit tests. The transport can withhold real read ACKs;
//! it never manufactures a confirmation or advances the unified apply fence.

use super::*;
use crate::rawnode::MAX_PENDING_READ_INDEX;
use crate::transport::InProcHub;
use crate::RaftGroup;
use kv9_common::RegionId;
use kv9_engine::ColumnFamily;
use raft::prelude::{Message, MessageType};

struct RecordingTransport {
    inner: Arc<dyn RaftTransport>,
    sent: Arc<Mutex<Vec<Message>>>,
    hold_read_acks: Arc<AtomicBool>,
}

impl RaftTransport for RecordingTransport {
    fn set_work_signal(&self, signal: Arc<crate::work::WorkSignal>) {
        self.inner.set_work_signal(signal);
    }

    fn send(&self, to: NodeId, message: Message) {
        self.sent.lock().unwrap().push(message.clone());
        if self.hold_read_acks.load(Ordering::SeqCst)
            && message.get_msg_type() == MessageType::MsgHeartbeatResponse
            && !message.context.is_empty()
        {
            return;
        }
        self.inner.send(to, message);
    }

    fn drain(&self) -> Vec<Message> {
        self.inner.drain()
    }
}

struct Trio {
    hub: Arc<InProcHub>,
    drivers: Vec<Arc<NodeDriver>>,
    sent: Arc<Mutex<Vec<Message>>>,
    hold_read_acks: Arc<AtomicBool>,
}

impl Trio {
    fn new() -> Self {
        let hub = InProcHub::new();
        let ids = [NodeId(1), NodeId(2), NodeId(3)];
        let sent = Arc::new(Mutex::new(Vec::new()));
        let hold_read_acks = Arc::new(AtomicBool::new(false));
        let drivers = ids
            .iter()
            .map(|&id| {
                NodeDriver::new(
                    Arc::new(RaftPeer::new(id, RegionId(1), &ids).unwrap()),
                    Arc::new(RecordingTransport {
                        inner: Arc::new(hub.endpoint(id)),
                        sent: sent.clone(),
                        hold_read_acks: hold_read_acks.clone(),
                    }),
                    MemStateMachine::new(),
                )
                .unwrap()
            })
            .collect();
        let trio = Self {
            hub,
            drivers,
            sent,
            hold_read_acks,
        };
        trio.drivers[0].peer().campaign().unwrap();
        trio.rounds(16);
        assert_eq!(trio.drivers[0].status().role, Role::Leader);
        assert!(trio.drivers.iter().all(|driver| {
            driver.driver_applied().is_some_and(|at| at.index == 1)
                && driver.peer().pending_read_count() == 0
        }));
        trio.sent.lock().unwrap().clear();
        trio
    }

    fn rounds(&self, count: usize) {
        for _ in 0..count {
            for driver in &self.drivers {
                driver.step().unwrap();
            }
        }
    }

    fn contexts(&self, from: u64) -> Vec<(u64, Vec<u8>)> {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .filter(|message| {
                message.from == from
                    && message.get_msg_type() == MessageType::MsgHeartbeat
                    && !message.context.is_empty()
            })
            .map(|message| (message.to, message.context.to_vec()))
            .collect()
    }

    // Deliver only a response actually produced by a follower. Routing it
    // through the real inbox also exercises the owner's publication/wake path.
    fn release_ack(&self, from: u64, context: &[u8]) {
        assert!(self.hold_read_acks.load(Ordering::SeqCst));
        let message = self
            .sent
            .lock()
            .unwrap()
            .iter()
            .find(|message| {
                message.from == from
                    && message.to == 1
                    && message.get_msg_type() == MessageType::MsgHeartbeatResponse
                    && message.context.as_ref() == context
            })
            .cloned();
        let message = message.expect("follower did not produce the selected real read ACK");
        self.hub.endpoint(NodeId(from)).send(NodeId(1), message);
    }
}

async fn assert_pending<F: std::future::Future>(mut future: std::pin::Pin<&mut F>) {
    let observed =
        std::future::poll_fn(|cx| std::task::Poll::Ready(future.as_mut().poll(cx))).await;
    assert!(
        observed.is_pending(),
        "read escaped its required quorum/apply evidence"
    );
}

// Stop and join even if a parking/completion assertion fails. No thread may
// survive this test to contend with another test or a later correctness run.
struct Owner {
    driver: Arc<NodeDriver>,
    task: Option<std::thread::JoinHandle<()>>,
}

impl Owner {
    fn start(driver: &Arc<NodeDriver>) -> Self {
        Self {
            driver: driver.clone(),
            task: Some(driver.spawn(Duration::from_secs(30)).unwrap()),
        }
    }

    fn finish(mut self) {
        self.driver.stop();
        self.task
            .take()
            .unwrap()
            .join()
            .expect("read-credit owner panicked");
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        self.driver.stop();
        if let Some(task) = self.task.take() {
            let _ = task.join();
        }
    }
}

#[tokio::test]
async fn cap_two_seals_late_members_until_distinct_confirmation() {
    assert_eq!(
        MAX_PENDING_READ_INDEX, 2,
        "this test pins the two-context protocol window"
    );
    let trio = Trio::new();
    trio.hold_read_acks.store(true, Ordering::SeqCst);
    let leader = &trio.drivers[0];
    let mut early: Vec<_> = (0..3)
        .map(|_| Box::pin(leader.read_barrier_async(Duration::from_secs(5))))
        .collect();
    for read in &mut early {
        assert_pending(read.as_mut()).await;
    }
    leader.step().unwrap();
    assert_eq!(leader.peer().pending_read_count(), 1);
    let first = trio.contexts(1);
    assert_eq!(first.len(), 2);
    assert_eq!(first[0].1, first[1].1);
    assert_eq!(
        first
            .iter()
            .map(|entry| entry.0)
            .collect::<std::collections::BTreeSet<_>>(),
        [2, 3].into_iter().collect()
    );

    let mut second: Vec<_> = (0..4)
        .map(|_| Box::pin(leader.read_barrier_async(Duration::from_secs(5))))
        .collect();
    for read in &mut second {
        assert_pending(read.as_mut()).await;
    }
    // No follower has run. The second, separately sealed group must use the
    // second slot rather than remaining serialized behind the first quorum.
    leader.step().unwrap();
    assert_eq!(
        leader.peer().pending_read_count(),
        2,
        "second protocol slot was not usable"
    );
    let two_groups = trio.contexts(1);
    assert_eq!(two_groups.len(), 4);
    assert_eq!(two_groups[2].1, two_groups[3].1);
    assert_ne!(
        two_groups[0].1, two_groups[2].1,
        "late sealed membership reused an earlier quorum context"
    );

    let mut third: Vec<_> = (0..2)
        .map(|_| Box::pin(leader.read_barrier_async(Duration::from_secs(5))))
        .collect();
    for read in &mut third {
        assert_pending(read.as_mut()).await;
    }
    for _ in 0..8 {
        leader.step().unwrap();
        assert_eq!(
            leader.peer().pending_read_count(),
            2,
            "read credit admitted a third unconfirmed context"
        );
    }
    let waiting = leader.async_read_snapshot();
    assert_eq!(
        (
            waiting.admitted_groups,
            waiting.admitted_members,
            waiting.queued
        ),
        (2, 7, 2)
    );
    assert_eq!(
        trio.contexts(1),
        two_groups,
        "full credit emitted a late ReadIndex broadcast"
    );

    // The follower produces ACKs for both admitted contexts, but only the
    // first is delivered. A later ACK may certify a prefix; an earlier ACK
    // must never certify either later sealed group.
    trio.drivers[1].step().unwrap();
    trio.release_ack(2, &two_groups[0].1);
    leader.step().unwrap();
    for read in early {
        assert_eq!(read.await.unwrap().index(), 1);
    }
    for read in second.iter_mut().chain(third.iter_mut()) {
        assert_pending(read.as_mut()).await;
    }
    let contexts = trio.contexts(1);
    assert_eq!(contexts.len(), 6);
    assert_eq!(contexts[4].1, contexts[5].1);
    assert_ne!(contexts[4].1, contexts[0].1);
    assert_ne!(contexts[4].1, contexts[2].1);
    assert_eq!(leader.peer().pending_read_count(), 2);
    let submitted = leader.async_read_snapshot();
    assert_eq!(
        (
            submitted.admitted_groups,
            submitted.admitted_members,
            submitted.max_admitted_group
        ),
        (3, 9, 4)
    );

    // Even a delayed duplicate of the first ACK cannot advance the two
    // replacement occupants of the window.
    trio.release_ack(2, &contexts[0].1);
    leader.step().unwrap();
    assert_eq!(
        leader.peer().pending_read_count(),
        2,
        "old ACK released a later protocol context"
    );
    for read in second.iter_mut().chain(third.iter_mut()) {
        assert_pending(read.as_mut()).await;
    }
    trio.release_ack(2, &contexts[2].1);
    leader.step().unwrap();
    for read in second {
        assert_eq!(read.await.unwrap().index(), 1);
    }
    assert_eq!(leader.peer().pending_read_count(), 1);
    for read in &mut third {
        assert_pending(read.as_mut()).await;
    }
    trio.drivers[1].step().unwrap();
    trio.release_ack(2, &contexts[4].1);
    leader.step().unwrap();
    for read in third {
        assert_eq!(read.await.unwrap().index(), 1);
    }
    assert_eq!(leader.peer().pending_read_count(), 0);
    assert_eq!(leader.async_read_snapshot().in_flight, 0);
}

#[tokio::test]
async fn canceled_members_do_not_refund_protocol_credit_and_queued_cancellation_drains() {
    let trio = Trio::new();
    trio.hold_read_acks.store(true, Ordering::SeqCst);
    let leader = &trio.drivers[0];
    let mut admitted = Vec::new();
    for _ in 0..MAX_PENDING_READ_INDEX {
        let mut read = Box::pin(leader.read_barrier_async(Duration::from_secs(5)));
        assert_pending(read.as_mut()).await;
        leader.step().unwrap();
        admitted.push(read);
    }
    assert_eq!(leader.peer().pending_read_count(), 2);
    let old_contexts = trio.contexts(1);
    assert_eq!(old_contexts.len(), 4);
    let mut canceled: Vec<_> = (0..70)
        .map(|_| Box::pin(leader.read_barrier_async(Duration::from_secs(5))))
        .collect();
    for read in &mut canceled {
        assert_pending(read.as_mut()).await;
    }
    drop(admitted);
    drop(canceled);
    let inspected = leader.async_read_snapshot().inspected;
    leader.step().unwrap();
    let partial = leader.async_read_snapshot();
    assert_eq!(
        partial.inspected - inspected,
        64,
        "queued cancellation exceeded its bounded prefix"
    );
    assert_eq!(
        (partial.queued, partial.active, partial.active_groups),
        (6, 0, 0)
    );
    assert_eq!(
        leader.peer().pending_read_count(),
        2,
        "last-member cancellation refunded live protocol credit"
    );
    leader.step().unwrap();
    assert_eq!(
        leader.async_read_snapshot().in_flight,
        0,
        "full protocol credit stranded queued cancellation cleanup"
    );
    assert_eq!(leader.peer().pending_read_count(), 2);

    let mut fresh = Box::pin(leader.read_barrier_async(Duration::from_secs(5)));
    assert_pending(fresh.as_mut()).await;
    leader.step().unwrap();
    assert_eq!(
        leader.async_read_snapshot().admitted_groups,
        2,
        "empty registry bypassed pending Raft credit"
    );
    assert_eq!(leader.async_read_snapshot().queued, 1);
    assert_eq!(trio.contexts(1), old_contexts);

    trio.drivers[1].step().unwrap();
    trio.release_ack(2, &old_contexts[0].1);
    leader.step().unwrap();
    assert_pending(fresh.as_mut()).await;
    let contexts = trio.contexts(1);
    assert_eq!(contexts.len(), 6);
    assert_ne!(contexts[4].1, contexts[0].1);
    assert_ne!(contexts[4].1, contexts[2].1);
    assert_eq!(leader.peer().pending_read_count(), 2);
    assert_eq!(leader.async_read_snapshot().admitted_groups, 3);
    trio.release_ack(2, &old_contexts[2].1);
    leader.step().unwrap();
    assert_eq!(leader.peer().pending_read_count(), 1);
    assert_pending(fresh.as_mut()).await;
    trio.drivers[1].step().unwrap();
    trio.release_ack(2, &contexts[4].1);
    leader.step().unwrap();
    assert_eq!(fresh.await.unwrap().index(), 1);
    assert_eq!(leader.peer().pending_read_count(), 0);
    assert_eq!(leader.async_read_snapshot().in_flight, 0);
}

#[tokio::test]
async fn full_credit_owner_parks_and_confirmation_wakes_without_tick() {
    let trio = Trio::new();
    trio.hold_read_acks.store(true, Ordering::SeqCst);
    let leader = &trio.drivers[0];
    let mut first = Box::pin(leader.read_barrier_async(Duration::from_secs(5)));
    assert_pending(first.as_mut()).await;
    leader.step().unwrap();
    let mut second = Box::pin(leader.read_barrier_async(Duration::from_secs(5)));
    assert_pending(second.as_mut()).await;
    leader.step().unwrap();
    let contexts = trio.contexts(1);
    assert_eq!(contexts.len(), 4);
    trio.drivers[1].step().unwrap();
    let mut late = Box::pin(leader.read_barrier_async(Duration::from_secs(5)));
    assert_pending(late.as_mut()).await;
    let parked = leader.peer.work_signal.observe_next_park();
    let owner = Owner::start(leader);
    parked
        .recv_timeout(Duration::from_secs(2))
        .expect("full read credit self-woke instead of parking the real owner");
    assert_eq!(leader.peer().pending_read_count(), 2);
    assert_eq!(leader.async_read_snapshot().queued, 1);

    let parked_again = leader.peer.work_signal.observe_next_park();
    trio.release_ack(2, &contexts[0].1);
    let barrier = tokio::time::timeout(Duration::from_secs(1), first)
        .await
        .expect("real quorum ACK did not wake the parked owner before its tick")
        .unwrap();
    assert_eq!(barrier.index(), 1);
    parked_again
        .recv_timeout(Duration::from_secs(2))
        .expect("new full-credit group failed to return the owner to sleep");
    assert_eq!(
        leader.async_read_snapshot().admitted_groups,
        3,
        "credit release did not submit the already queued group"
    );
    assert_pending(second.as_mut()).await;
    assert_pending(late.as_mut()).await;
    assert_eq!(leader.peer().pending_read_count(), 2);
    let all_contexts = trio.contexts(1);
    assert_eq!(all_contexts.len(), 6);
    trio.drivers[1].step().unwrap();
    trio.release_ack(2, &contexts[2].1);
    trio.release_ack(2, &all_contexts[4].1);
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), second)
            .await
            .expect("second group's quorum ACK waited for an election tick")
            .unwrap()
            .index(),
        1
    );
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), late)
            .await
            .expect("replacement group's quorum ACK waited for an election tick")
            .unwrap()
            .index(),
        1
    );
    owner.finish();
    assert_eq!(leader.peer().pending_read_count(), 0);
    assert_eq!(leader.async_read_snapshot().in_flight, 0);
}

#[tokio::test]
async fn confirmed_apply_held_group_releases_protocol_credit() {
    let trio = Trio::new();
    let leader = &trio.drivers[0];
    leader.pause_apply(true);
    let at = leader
        .propose(&Command::Put {
            cf: 0,
            key: b"credit".to_vec(),
            value: b"applied".to_vec(),
        })
        .unwrap();
    trio.rounds(6);
    assert!(leader.status().raft_committed >= at.index.0);
    assert!(leader.driver_applied().unwrap().index < at.index.0);
    trio.hold_read_acks.store(true, Ordering::SeqCst);

    let mut reads = Vec::new();
    for _ in 0..3 {
        let mut read = Box::pin(leader.read_barrier_async(Duration::from_secs(5)));
        assert_pending(read.as_mut()).await;
        leader.step().unwrap();
        reads.push(read);
    }
    assert_eq!(leader.peer().pending_read_count(), 2);
    assert_eq!(leader.async_read_snapshot().queued, 1);
    let contexts = trio.contexts(1);
    assert_eq!(contexts.len(), 4);
    trio.drivers[1].step().unwrap();
    trio.release_ack(2, &contexts[0].1);
    leader.step().unwrap();
    assert_eq!(
        leader.peer().pending_read_count(),
        2,
        "confirmed apply-held group incorrectly retained protocol credit"
    );
    assert_eq!(
        leader.async_read_snapshot().admitted_groups,
        3,
        "apply-held result blocked the newly available protocol slot"
    );
    assert_eq!(leader.async_read_snapshot().active_groups, 3);
    let all_contexts = trio.contexts(1);
    assert_eq!(all_contexts.len(), 6);
    for read in &mut reads {
        assert_pending(read.as_mut()).await;
    }
    trio.release_ack(2, &contexts[2].1);
    leader.step().unwrap();
    assert_eq!(leader.peer().pending_read_count(), 1);
    trio.drivers[1].step().unwrap();
    trio.release_ack(2, &all_contexts[4].1);
    leader.step().unwrap();
    assert_eq!(leader.peer().pending_read_count(), 0);
    assert_eq!(leader.async_read_snapshot().active_groups, 3);
    for read in &mut reads {
        assert_pending(read.as_mut()).await;
    }
    leader.pause_apply(false);
    leader.step().unwrap();
    for read in reads {
        assert_eq!(read.await.unwrap().index(), at.index.0);
    }
    assert_eq!(
        leader.get(ColumnFamily::Default, b"credit").unwrap(),
        Some(b"applied".to_vec())
    );
    assert_eq!(leader.async_read_snapshot().in_flight, 0);
}

#[tokio::test]
async fn synchronous_read_shares_credit_and_requires_its_own_confirmation() {
    let trio = Trio::new();
    trio.hold_read_acks.store(true, Ordering::SeqCst);
    let leader = &trio.drivers[0];
    let mut admitted = Vec::new();
    for _ in 0..MAX_PENDING_READ_INDEX {
        let mut read = Box::pin(leader.read_barrier_async(Duration::from_secs(5)));
        assert_pending(read.as_mut()).await;
        leader.step().unwrap();
        admitted.push(read);
    }
    let old_contexts = trio.contexts(1);
    assert_eq!(old_contexts.len(), 4);
    trio.drivers[1].step().unwrap();
    let (attempt_tx, attempt_rx) = std::sync::mpsc::channel();
    *leader.read_attempt_observer.lock().unwrap() = Some(attempt_tx);
    let parked = leader.completion.observe_next_park();

    // Scoped joining retains the synchronous reader through assertion failure.
    // Its original deadline bounds teardown even when a defect loses a wake.
    std::thread::scope(|scope| {
        let read = scope.spawn(|| leader.read_barrier(Duration::from_secs(2)));
        attempt_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        parked.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(
            leader.peer().pending_read_count(),
            2,
            "synchronous admission bypassed outstanding protocol credit"
        );
        assert_eq!(trio.contexts(1), old_contexts);
        assert_eq!(leader.read_barriers_minted(), 3);

        trio.release_ack(2, &old_contexts[0].1);
        leader.step().unwrap();
        attempt_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("credit release did not wake synchronous admission");
        assert_eq!(leader.peer().pending_read_count(), 2);
        assert!(
            !read.is_finished(),
            "synchronous read borrowed an old confirmation"
        );
        leader.step().unwrap();
        let contexts = trio.contexts(1);
        assert_eq!(contexts.len(), 6);
        assert_ne!(contexts[4].1, contexts[0].1);
        assert_ne!(contexts[4].1, contexts[2].1);
        trio.release_ack(2, &old_contexts[2].1);
        leader.step().unwrap();
        assert_eq!(leader.peer().pending_read_count(), 1);
        assert!(
            !read.is_finished(),
            "synchronous read borrowed an old confirmation"
        );
        trio.drivers[1].step().unwrap();
        trio.release_ack(2, &contexts[4].1);
        leader.step().unwrap();
        assert_eq!(read.join().unwrap().unwrap().index(), 1);
    });
    for read in admitted {
        assert_eq!(read.await.unwrap().index(), 1);
    }
    assert_eq!(leader.read_barriers_minted(), 3);
    assert_eq!(leader.peer().pending_read_count(), 0);
    assert_eq!(leader.async_read_snapshot().in_flight, 0);
}

#[test]
fn credit_deferred_synchronous_read_keeps_its_original_deadline() {
    let trio = Trio::new();
    let leader = &trio.drivers[0];
    for context in [b"held-protocol-context-a", b"held-protocol-context-b"] {
        assert!(leader.peer().read_index(context.to_vec()).unwrap());
        leader.step().unwrap();
    }
    assert_eq!(leader.peer().pending_read_count(), 2);
    let started = Instant::now();
    let result = leader.read_barrier(Duration::from_millis(30));
    assert!(
        matches!(
            result,
            Err(ReadIndexError::Unconfirmed {
                phase: BarrierPhase::QuorumConfirmation,
                ..
            })
        ),
        "credit deferral must retain the original quorum deadline: {result:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(leader.read_barriers_minted(), 1);
    assert_eq!(leader.peer().pending_read_count(), 2);
    assert_eq!(trio.contexts(1).len(), 4);
    trio.drivers[1].step().unwrap();
    leader.step().unwrap();
    assert_eq!(leader.peer().pending_read_count(), 0);
}

#[tokio::test]
async fn expired_async_members_do_not_refund_protocol_credit() {
    let trio = Trio::new();
    let leader = &trio.drivers[0];
    let mut reads = Vec::new();
    for _ in 0..3 {
        let mut read = Box::pin(leader.read_barrier_async(Duration::from_millis(150)));
        assert_pending(read.as_mut()).await;
        leader.step().unwrap();
        reads.push(read);
    }
    assert_eq!(leader.peer().pending_read_count(), 2);
    assert_eq!(leader.async_read_snapshot().queued, 1);
    let contexts = trio.contexts(1);
    assert_eq!(contexts.len(), 4);
    // Await the real caller deadlines: no mocked queue timestamps or sleep
    // duration is used as evidence of protocol admission or confirmation.
    for read in reads {
        let result = tokio::time::timeout(Duration::from_secs(1), read)
            .await
            .expect("original asynchronous deadline did not fire");
        assert!(
            matches!(
                result,
                Err(ReadIndexError::Unconfirmed {
                    phase: BarrierPhase::QuorumConfirmation,
                    ..
                })
            ),
            "expired unconfirmed group changed its typed deadline result: {result:?}"
        );
    }
    leader.step().unwrap();
    assert_eq!(leader.async_read_snapshot().in_flight, 0);
    assert_eq!(
        leader.peer().pending_read_count(),
        2,
        "member deadlines refunded live protocol credit"
    );
    assert_eq!(trio.contexts(1), contexts);
    let mut fresh = Box::pin(leader.read_barrier_async(Duration::from_secs(5)));
    assert_pending(fresh.as_mut()).await;
    leader.step().unwrap();
    assert_eq!(
        leader.async_read_snapshot().queued,
        1,
        "expired registry bypassed the full protocol window"
    );
    assert_eq!(trio.contexts(1), contexts);
    trio.drivers[1].step().unwrap();
    leader.step().unwrap();
    assert_eq!(leader.peer().pending_read_count(), 1);
    assert_pending(fresh.as_mut()).await;
    assert_eq!(trio.contexts(1).len(), 6);
    trio.drivers[1].step().unwrap();
    leader.step().unwrap();
    assert_eq!(fresh.await.unwrap().index(), 1);
    assert_eq!(leader.peer().pending_read_count(), 0);
}

#[tokio::test]
async fn configuration_quorum_reevaluation_releases_the_full_protocol_window() {
    let trio = Trio::new();
    trio.hold_read_acks.store(true, Ordering::SeqCst);
    let leader = &trio.drivers[0];
    let mut reads = Vec::new();
    for _ in 0..3 {
        let mut read = Box::pin(leader.read_barrier_async(Duration::from_secs(5)));
        assert_pending(read.as_mut()).await;
        leader.step().unwrap();
        reads.push(read);
    }
    assert_eq!(leader.peer().pending_read_count(), 2);
    assert_eq!(leader.async_read_snapshot().queued, 1);
    assert_eq!(trio.contexts(1).len(), 4);

    // These are ordinary committed, applied configuration proposals, not
    // direct tracker edits. ACKs carrying read contexts remain withheld.
    let remove_third = leader
        .peer()
        .propose_conf_change_traced(single_change(NodeId(3), ConfChangeType::RemoveNode))
        .unwrap();
    trio.rounds(16);
    let receipt = leader
        .wait_conf_applied(remove_third, Duration::from_millis(50))
        .unwrap();
    assert_eq!(receipt.voters, vec![1, 2]);
    assert_eq!(
        leader.peer().pending_read_count(),
        2,
        "two-voter configuration fabricated a read quorum"
    );
    for read in &mut reads {
        assert_pending(read.as_mut()).await;
    }

    let remove_second = leader
        .peer()
        .propose_conf_change_traced(single_change(NodeId(2), ConfChangeType::RemoveNode))
        .unwrap();
    trio.rounds(16);
    let receipt = leader
        .wait_conf_applied(remove_second, Duration::from_millis(50))
        .unwrap();
    assert_eq!(receipt.voters, vec![1]);
    assert_eq!(
        leader.peer().pending_read_count(),
        0,
        "configuration quorum reevaluation stranded protocol credit"
    );
    assert_eq!(
        leader.async_read_snapshot().admitted_groups,
        3,
        "configuration release stranded the queued group"
    );
    let mut indices = Vec::new();
    for read in reads {
        indices.push(read.await.unwrap().index());
    }
    assert_eq!(indices[0], 1);
    assert_eq!(indices[1], 1);
    assert!(indices[2] >= remove_second.index.0);
    assert!(leader.driver_applied().unwrap().index >= indices[2]);
    assert_eq!(leader.async_read_snapshot().in_flight, 0);

    // The now-single voter confirms synchronously in raft-rs and must not
    // retain an artificial occupancy until some nonexistent remote ACK.
    let mut fresh = Box::pin(leader.read_barrier_async(Duration::from_secs(5)));
    assert_pending(fresh.as_mut()).await;
    leader.step().unwrap();
    assert_eq!(leader.peer().pending_read_count(), 0);
    assert!(fresh.await.unwrap().index() >= remove_second.index.0);
}

#[tokio::test]
async fn role_reset_refuses_queued_reads_and_fresh_leader_can_read() {
    let trio = Trio::new();
    let former = &trio.drivers[0];
    let old_term = former.status().term;
    trio.hold_read_acks.store(true, Ordering::SeqCst);
    let mut admitted = Vec::new();
    for _ in 0..MAX_PENDING_READ_INDEX {
        let mut read = Box::pin(former.read_barrier_async(Duration::from_secs(5)));
        assert_pending(read.as_mut()).await;
        former.step().unwrap();
        admitted.push(read);
    }
    assert_eq!(former.peer().pending_read_count(), 2);
    let old_contexts = trio.contexts(1);
    assert_eq!(old_contexts.len(), 4);
    let mut queued = Box::pin(former.read_barrier_async(Duration::from_secs(5)));
    assert_pending(queued.as_mut()).await;
    former.step().unwrap();
    assert_eq!(former.async_read_snapshot().queued, 1);

    // A real transfer uses TimeoutNow and a new election. Only context-bearing
    // read ACKs are withheld, so no old read confirmation can refund a slot.
    former.peer().transfer_leader_for_tests(NodeId(2));
    trio.rounds(24);
    assert_eq!(former.status().role, Role::Follower);
    assert_eq!(trio.drivers[1].status().role, Role::Leader);
    assert!(former.status().term > old_term);
    assert_eq!(
        former.peer().pending_read_count(),
        0,
        "new-term reset retained stale protocol credit"
    );
    let refusal = queued.await;
    assert!(
        matches!(refusal, Err(ReadIndexError::NotLeader { .. })),
        "role change stranded a credit-deferred read instead of typed refusal: {refusal:?}"
    );
    for read in &mut admitted {
        assert_pending(read.as_mut()).await;
    }
    drop(admitted);
    former.step().unwrap();
    assert_eq!(former.async_read_snapshot().in_flight, 0);

    trio.hold_read_acks.store(false, Ordering::SeqCst);
    let current = &trio.drivers[1];
    let mut fresh = Box::pin(current.read_barrier_async(Duration::from_secs(5)));
    assert_pending(fresh.as_mut()).await;
    current.step().unwrap();
    assert_eq!(current.peer().pending_read_count(), 1);
    let new_context = trio.contexts(2)[0].1.clone();
    assert!(old_contexts.iter().all(|(_, old)| old != &new_context));
    trio.rounds(4);
    let barrier = fresh.await.unwrap();
    assert!(
        barrier.index() > 1,
        "fresh leader read missed the new-term commit fence"
    );
    assert_eq!(current.peer().pending_read_count(), 0);
    assert_eq!(current.async_read_snapshot().in_flight, 0);
}
