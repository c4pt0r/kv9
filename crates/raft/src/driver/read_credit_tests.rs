//! Real Raft admission-credit tests. The transport can withhold real read ACKs;
//! it never manufactures a confirmation or advances the unified apply fence.

use super::*;
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
async fn cap_one_seals_late_members_until_distinct_confirmation() {
    let trio = Trio::new();
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

    let mut late: Vec<_> = (0..4)
        .map(|_| Box::pin(leader.read_barrier_async(Duration::from_secs(5))))
        .collect();
    for read in &mut late {
        assert_pending(read.as_mut()).await;
    }
    // Neither follower runs: the first context has no quorum ACK. Repeated
    // admissions may inspect/rotate the late prefix, but must not submit it.
    for _ in 0..8 {
        leader.step().unwrap();
        assert_eq!(
            leader.peer().pending_read_count(),
            1,
            "read credit admitted a second unconfirmed context"
        );
    }
    let waiting = leader.async_read_snapshot();
    assert_eq!(
        (
            waiting.admitted_groups,
            waiting.admitted_members,
            waiting.queued
        ),
        (1, 3, 4)
    );
    assert_eq!(
        trio.contexts(1),
        first,
        "full credit emitted a late ReadIndex broadcast"
    );

    trio.drivers[1].step().unwrap();
    leader.step().unwrap();
    for read in early {
        assert_eq!(read.await.unwrap().index(), 1);
    }
    for read in &mut late {
        assert_pending(read.as_mut()).await;
    }
    let contexts = trio.contexts(1);
    assert_eq!(contexts.len(), 4);
    assert_eq!(contexts[2].1, contexts[3].1);
    assert_ne!(
        contexts[0].1, contexts[2].1,
        "late sealed membership reused an earlier quorum context"
    );
    assert_eq!(leader.peer().pending_read_count(), 1);
    let submitted = leader.async_read_snapshot();
    assert_eq!(
        (
            submitted.admitted_groups,
            submitted.admitted_members,
            submitted.max_admitted_group
        ),
        (2, 7, 4)
    );

    trio.drivers[1].step().unwrap();
    leader.step().unwrap();
    for read in late {
        assert_eq!(read.await.unwrap().index(), 1);
    }
    assert_eq!(leader.peer().pending_read_count(), 0);
    assert_eq!(leader.async_read_snapshot().in_flight, 0);
}

#[tokio::test]
async fn canceled_members_do_not_refund_protocol_credit_and_queued_cancellation_drains() {
    let trio = Trio::new();
    let leader = &trio.drivers[0];
    let mut first = Box::pin(leader.read_barrier_async(Duration::from_secs(5)));
    assert_pending(first.as_mut()).await;
    leader.step().unwrap();
    assert_eq!(leader.peer().pending_read_count(), 1);

    let mut canceled: Vec<_> = (0..70)
        .map(|_| Box::pin(leader.read_barrier_async(Duration::from_secs(5))))
        .collect();
    for read in &mut canceled {
        assert_pending(read.as_mut()).await;
    }
    drop(first);
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
        1,
        "last-member cancellation refunded live protocol credit"
    );
    leader.step().unwrap();
    assert_eq!(
        leader.async_read_snapshot().in_flight,
        0,
        "full protocol credit stranded queued cancellation cleanup"
    );
    assert_eq!(leader.peer().pending_read_count(), 1);

    let mut fresh = Box::pin(leader.read_barrier_async(Duration::from_secs(5)));
    assert_pending(fresh.as_mut()).await;
    leader.step().unwrap();
    assert_eq!(
        leader.async_read_snapshot().admitted_groups,
        1,
        "empty registry bypassed pending Raft credit"
    );
    assert_eq!(leader.async_read_snapshot().queued, 1);
    assert_eq!(trio.contexts(1).len(), 2);

    // The old member is gone, but only its real quorum response frees the
    // protocol slot. That old response cannot serve the new caller.
    trio.drivers[1].step().unwrap();
    leader.step().unwrap();
    assert_pending(fresh.as_mut()).await;
    let contexts = trio.contexts(1);
    assert_eq!(contexts.len(), 4);
    assert_ne!(contexts[0].1, contexts[2].1);
    assert_eq!(leader.async_read_snapshot().admitted_groups, 2);
    trio.drivers[1].step().unwrap();
    leader.step().unwrap();
    assert_eq!(fresh.await.unwrap().index(), 1);
    assert_eq!(leader.peer().pending_read_count(), 0);
    assert_eq!(leader.async_read_snapshot().in_flight, 0);
}

#[tokio::test]
async fn full_credit_owner_parks_and_confirmation_wakes_without_tick() {
    let trio = Trio::new();
    let leader = &trio.drivers[0];
    let mut first = Box::pin(leader.read_barrier_async(Duration::from_secs(5)));
    assert_pending(first.as_mut()).await;
    leader.step().unwrap();
    let mut late = Box::pin(leader.read_barrier_async(Duration::from_secs(5)));
    assert_pending(late.as_mut()).await;
    let parked = leader.peer.work_signal.observe_next_park();
    let owner = Owner::start(leader);
    parked
        .recv_timeout(Duration::from_secs(2))
        .expect("full read credit self-woke instead of parking the real owner");
    assert_eq!(leader.peer().pending_read_count(), 1);
    assert_eq!(leader.async_read_snapshot().queued, 1);

    let parked_again = leader.peer.work_signal.observe_next_park();
    trio.drivers[1].step().unwrap();
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
        2,
        "credit release did not submit the already queued group"
    );
    assert_pending(late.as_mut()).await;
    assert_eq!(leader.peer().pending_read_count(), 1);
    trio.drivers[1].step().unwrap();
    let barrier = tokio::time::timeout(Duration::from_secs(1), late)
        .await
        .expect("replacement group's quorum ACK waited for an election tick")
        .unwrap();
    assert_eq!(barrier.index(), 1);
    owner.finish();
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

    let mut first = Box::pin(leader.read_barrier_async(Duration::from_secs(5)));
    assert_pending(first.as_mut()).await;
    leader.step().unwrap();
    assert_eq!(leader.peer().pending_read_count(), 1);
    trio.drivers[1].step().unwrap();
    leader.step().unwrap();
    assert_eq!(leader.peer().pending_read_count(), 0);
    assert_eq!(leader.async_read_snapshot().active_groups, 1);
    assert_pending(first.as_mut()).await;

    let mut second = Box::pin(leader.read_barrier_async(Duration::from_secs(5)));
    assert_pending(second.as_mut()).await;
    leader.step().unwrap();
    assert_eq!(
        leader.peer().pending_read_count(),
        1,
        "confirmed apply-held group incorrectly retained protocol credit"
    );
    assert_eq!(leader.async_read_snapshot().admitted_groups, 2);
    assert_eq!(leader.async_read_snapshot().active_groups, 2);
    trio.drivers[1].step().unwrap();
    leader.step().unwrap();
    assert_eq!(leader.peer().pending_read_count(), 0);
    assert_pending(first.as_mut()).await;
    assert_pending(second.as_mut()).await;

    leader.pause_apply(false);
    leader.step().unwrap();
    assert_eq!(first.await.unwrap().index(), at.index.0);
    assert_eq!(second.await.unwrap().index(), at.index.0);
    assert_eq!(
        leader.get(ColumnFamily::Default, b"credit").unwrap(),
        Some(b"applied".to_vec())
    );
    assert_eq!(leader.async_read_snapshot().in_flight, 0);
}

#[tokio::test]
async fn synchronous_read_shares_credit_and_requires_its_own_confirmation() {
    let trio = Trio::new();
    let leader = &trio.drivers[0];
    let mut first = Box::pin(leader.read_barrier_async(Duration::from_secs(5)));
    assert_pending(first.as_mut()).await;
    leader.step().unwrap();
    let first_context = trio.contexts(1)[0].1.clone();
    let (attempt_tx, attempt_rx) = std::sync::mpsc::channel();
    *leader.read_attempt_observer.lock().unwrap() = Some(attempt_tx);
    let parked = leader.completion.observe_next_park();

    // Scoped joining retains the reader through assertion failure. The
    // original deadline bounds teardown even if a defect loses a wake.
    std::thread::scope(|scope| {
        let read = scope.spawn(|| leader.read_barrier(Duration::from_secs(2)));
        attempt_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        parked.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(
            leader.peer().pending_read_count(),
            1,
            "synchronous admission bypassed outstanding protocol credit"
        );
        assert_eq!(trio.contexts(1).len(), 2);
        assert_eq!(leader.read_barriers_minted(), 2);

        trio.drivers[1].step().unwrap();
        leader.step().unwrap();
        attempt_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("credit release did not wake synchronous admission");
        assert_eq!(leader.peer().pending_read_count(), 1);
        assert!(
            !read.is_finished(),
            "synchronous read borrowed an old confirmation"
        );
        leader.step().unwrap();
        let contexts = trio.contexts(1);
        assert_eq!(contexts.len(), 4);
        assert_ne!(first_context, contexts[2].1);
        trio.drivers[1].step().unwrap();
        leader.step().unwrap();
        assert_eq!(read.join().unwrap().unwrap().index(), 1);
    });
    assert_eq!(first.await.unwrap().index(), 1);
    assert_eq!(leader.read_barriers_minted(), 2);
    assert_eq!(leader.peer().pending_read_count(), 0);
    assert_eq!(leader.async_read_snapshot().in_flight, 0);
}

#[test]
fn credit_deferred_synchronous_read_keeps_its_original_deadline() {
    let trio = Trio::new();
    let leader = &trio.drivers[0];
    assert!(leader
        .peer()
        .read_index(b"held-protocol-context".to_vec())
        .unwrap());
    leader.step().unwrap();
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
    assert_eq!(leader.peer().pending_read_count(), 1);
    assert_eq!(trio.contexts(1).len(), 2);
    // Timeout did not retract the outstanding protocol request.
    trio.drivers[1].step().unwrap();
    leader.step().unwrap();
    assert_eq!(leader.peer().pending_read_count(), 0);
}

#[tokio::test]
async fn role_reset_refuses_queued_reads_and_fresh_leader_can_read() {
    let trio = Trio::new();
    let former = &trio.drivers[0];
    let old_term = former.status().term;
    trio.hold_read_acks.store(true, Ordering::SeqCst);
    let mut admitted = Box::pin(former.read_barrier_async(Duration::from_secs(5)));
    assert_pending(admitted.as_mut()).await;
    former.step().unwrap();
    assert_eq!(former.peer().pending_read_count(), 1);
    let old_context = trio.contexts(1)[0].1.clone();
    let mut queued = Box::pin(former.read_barrier_async(Duration::from_secs(5)));
    assert_pending(queued.as_mut()).await;
    former.step().unwrap();
    assert_eq!(former.async_read_snapshot().queued, 1);

    // A real transfer uses TimeoutNow and a new election. Only context-bearing
    // read ACKs are withheld, so no old read confirmation can refund the slot.
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
    assert_pending(admitted.as_mut()).await;
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
    assert_ne!(old_context, new_context);
    trio.rounds(4);
    let barrier = fresh.await.unwrap();
    assert!(
        barrier.index() > 1,
        "fresh leader read missed the new-term commit fence"
    );
    assert_eq!(current.peer().pending_read_count(), 0);
    assert_eq!(current.async_read_snapshot().in_flight, 0);
}
