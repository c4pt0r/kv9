//! Owner-local admission must pump in this turn without consuming producer
//! notifications or treating submission as a quorum/apply certificate.

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

struct Cluster {
    hub: Arc<InProcHub>,
    drivers: Vec<Arc<NodeDriver>>,
    sent: Arc<Mutex<Vec<Message>>>,
    hold_read_acks: Arc<AtomicBool>,
}

impl Cluster {
    fn new(count: u64) -> Self {
        let hub = InProcHub::new();
        let ids: Vec<_> = (1..=count).map(NodeId).collect();
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
        let cluster = Self {
            hub,
            drivers,
            sent,
            hold_read_acks,
        };
        cluster.drivers[0].peer().campaign().unwrap();
        cluster.rounds(16);
        assert_eq!(cluster.drivers[0].status().role, Role::Leader);
        assert!(
            cluster.drivers.iter().all(|driver| {
                driver.driver_applied().is_some_and(|at| at.index == 1)
                    && !driver.peer.has_pending_ready()
            }),
            "fixture did not finish its election Ready/apply work"
        );
        cluster.sent.lock().unwrap().clear();
        cluster
    }

    fn rounds(&self, count: usize) {
        for _ in 0..count {
            for driver in &self.drivers {
                driver.step().unwrap();
            }
        }
    }

    fn read_heartbeats(&self) -> Vec<Message> {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .filter(|message| {
                message.from == 1
                    && message.get_msg_type() == MessageType::MsgHeartbeat
                    && !message.context.is_empty()
            })
            .cloned()
            .collect()
    }

    fn held_ack(&self, context: &[u8]) -> Message {
        assert!(self.hold_read_acks.load(Ordering::SeqCst));
        let message = self
            .sent
            .lock()
            .unwrap()
            .iter()
            .find(|message| {
                message.from == 2
                    && message.to == 1
                    && message.get_msg_type() == MessageType::MsgHeartbeatResponse
                    && message.context.as_ref() == context
            })
            .cloned();
        message.expect("follower did not produce the selected real read ACK")
    }
}

async fn assert_pending<F: std::future::Future>(mut future: std::pin::Pin<&mut F>) {
    let observed =
        std::future::poll_fn(|cx| std::task::Poll::Ready(future.as_mut().poll(cx))).await;
    assert!(
        observed.is_pending(),
        "owner submission escaped the quorum/apply fence"
    );
}

fn successful_turns(driver: &NodeDriver) -> u64 {
    driver.metrics().pump_service.snapshot().outcomes[Outcome::Success as usize].count
}

// Always stop and join, including assertion failure. The owner uses the
// original scheduler with a long tick, not a test-only scheduling loop.
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
            .expect("read owner panicked");
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
async fn owner_submission_emits_same_turn_quorum_work_and_preserves_apply_fence() {
    let cluster = Cluster::new(3);
    let leader = &cluster.drivers[0];
    leader.pause_apply(true);
    let at = leader
        .propose(&Command::Put {
            cf: 0,
            key: b"owner-read".to_vec(),
            value: b"after-apply".to_vec(),
        })
        .unwrap();
    cluster.rounds(6);
    assert!(leader.status().raft_committed >= at.index.0);
    assert!(leader.driver_applied().unwrap().index < at.index.0);

    let mut read = Box::pin(leader.read_barrier_async(Duration::from_secs(5)));
    assert_pending(read.as_mut()).await;
    assert!(leader.peer.work_signal.begin_turn());
    leader.step().unwrap();
    let messages = cluster.read_heartbeats();
    assert_eq!(
        messages.len(),
        2,
        "owner submission did not pump its quorum request in the same turn"
    );
    assert_eq!(messages[0].context, messages[1].context);
    assert_eq!(
        messages
            .iter()
            .map(|m| m.to)
            .collect::<std::collections::BTreeSet<_>>(),
        [2, 3].into_iter().collect()
    );
    assert_eq!(leader.async_read_snapshot().admitted_groups, 1);
    assert_pending(read.as_mut()).await;

    // Only the real follower reply provides the second vote. Even after
    // confirmation, the committed write has not crossed unified apply.
    cluster.drivers[1].step().unwrap();
    leader.step().unwrap();
    assert_pending(read.as_mut()).await;
    assert!(leader.driver_applied().unwrap().index < at.index.0);
    leader.pause_apply(false);
    leader.step().unwrap();
    assert_eq!(read.await.unwrap().index(), at.index.0);
    assert_eq!(
        leader.get(ColumnFamily::Default, b"owner-read").unwrap(),
        Some(b"after-apply".to_vec())
    );
    assert_eq!(leader.async_read_snapshot().in_flight, 0);
}

#[tokio::test]
async fn owner_submission_parks_after_one_turn_without_self_notification() {
    let cluster = Cluster::new(3);
    let leader = &cluster.drivers[0];
    let mut read = Box::pin(leader.read_barrier_async(Duration::from_secs(5)));
    assert_pending(read.as_mut()).await;
    let before = successful_turns(leader);
    let parked = leader.peer.work_signal.observe_next_park();
    let owner = Owner::start(leader);
    parked
        .recv_timeout(Duration::from_secs(2))
        .expect("isolated read owner did not park");
    // Followers never run, and the settled fixture has no Ready successor.
    // The observer fires only once the actual owner reaches its idle wait.
    assert_eq!(
        successful_turns(leader) - before,
        1,
        "owner read submission forced an extra empty turn"
    );
    assert_eq!(cluster.read_heartbeats().len(), 2);
    assert_eq!(leader.async_read_snapshot().admitted_groups, 1);
    assert_eq!(leader.async_read_snapshot().queued, 0);
    assert_pending(read.as_mut()).await;
    drop(read);
    owner.finish();
    assert_eq!(leader.async_read_snapshot().in_flight, 0);
}

#[test]
fn external_synchronous_submission_wakes_a_parked_owner() {
    let cluster = Cluster::new(1);
    let leader = &cluster.drivers[0];
    let parked = leader.peer.work_signal.observe_next_park();
    let owner = Owner::start(leader);
    parked
        .recv_timeout(Duration::from_secs(2))
        .expect("external-read fixture owner did not park");
    // No follower, registration wake, proposal or timer can rescue this
    // external submission. Its own public notifying wrapper must wake the owner.
    let result = leader.read_barrier(Duration::from_secs(2));
    assert!(
        result.is_ok(),
        "external synchronous read failed to wake the parked owner: {result:?}"
    );
    assert_eq!(result.unwrap().index(), 1);
    assert_eq!(leader.read_barriers_minted(), 1);
    owner.finish();
}

#[tokio::test]
async fn retained_read_suffix_schedules_the_next_owner_turn() {
    let cluster = Cluster::new(3);
    let leader = &cluster.drivers[0];
    let mut reads: Vec<_> = (0..65)
        .map(|_| Box::pin(leader.read_barrier_async(Duration::from_secs(5))))
        .collect();
    for read in &mut reads {
        assert_pending(read.as_mut()).await;
    }
    let before = successful_turns(leader);
    let parked = leader.peer.work_signal.observe_next_park();
    let owner = Owner::start(leader);
    parked
        .recv_timeout(Duration::from_secs(2))
        .expect("read suffix owner did not park");
    let snapshot = leader.async_read_snapshot();
    assert_eq!(
        (
            snapshot.admitted_groups,
            snapshot.admitted_members,
            snapshot.queued
        ),
        (2, 65, 0),
        "retained read suffix failed to schedule its owner turn"
    );
    assert_eq!(snapshot.max_admitted_group, 64);
    assert_eq!(
        successful_turns(leader) - before,
        2,
        "read suffix required an extra empty owner turn"
    );
    let messages = cluster.read_heartbeats();
    assert_eq!(messages.len(), 4);
    assert_eq!(messages[0].context, messages[1].context);
    assert_eq!(messages[2].context, messages[3].context);
    assert_ne!(messages[0].context, messages[2].context);
    for read in &mut reads {
        assert_pending(read.as_mut()).await;
    }
    drop(reads);
    owner.finish();
    assert_eq!(leader.async_read_snapshot().in_flight, 0);
}

#[tokio::test]
async fn transport_publication_during_submission_retains_its_owner_notification() {
    let cluster = Cluster::new(3);
    let leader = &cluster.drivers[0];
    cluster.hold_read_acks.store(true, Ordering::SeqCst);
    let mut first = Box::pin(leader.read_barrier_async(Duration::from_secs(5)));
    assert_pending(first.as_mut()).await;
    leader.step().unwrap();
    let first_messages = cluster.read_heartbeats();
    assert_eq!(first_messages.len(), 2);
    cluster.drivers[1].step().unwrap();
    let acknowledgement = cluster.held_ack(&first_messages[0].context);

    let mut second = Box::pin(leader.read_barrier_async(Duration::from_secs(5)));
    assert_pending(second.as_mut()).await;
    // This is the owner's normal consumption point, before queue inspection.
    assert!(leader.peer.work_signal.begin_turn());
    let (publish_tx, publish_rx) = std::sync::mpsc::channel();
    let (published_tx, published_rx) = std::sync::mpsc::channel();
    let hub = cluster.hub.clone();
    let messages = std::thread::scope(|scope| {
        let publisher = scope.spawn(move || {
            publish_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("submission callback did not reach publication rendezvous");
            hub.endpoint(NodeId(2)).send(NodeId(1), acknowledgement);
            published_tx.send(()).unwrap();
        });
        let messages = {
            let _owner = leader.pump_gate.lock().unwrap();
            leader
                .drain
                .pump_with_read_submission(|admit| {
                    leader.async_reads.submit(|context| {
                        let result = admit(context);
                        publish_tx.send(()).unwrap();
                        published_rx
                            .recv_timeout(Duration::from_secs(2))
                            .expect("concurrent transport publication did not finish");
                        result
                    });
                })
                .unwrap()
        };
        publisher.join().unwrap();
        messages
    });
    for message in messages {
        leader.transport.send(NodeId(message.to), message);
    }
    assert_eq!(cluster.read_heartbeats().len(), 4);
    assert_pending(first.as_mut()).await;
    assert_pending(second.as_mut()).await;

    // The first ACK arrived after this turn's inbox-drain boundary. The
    // transaction must retain its pending notification. A waiter with a
    // 30-second deadline must return before the one-second observation ends.
    // An unconditional cleanup notification joins the waiter even on failure;
    // it is sent only AFTER capturing the observation verdict.
    let signal = leader.peer.work_signal.clone();
    let returned = std::thread::scope(|scope| {
        let (tx, rx) = std::sync::mpsc::channel();
        let waiting_signal = signal.clone();
        let waiter = scope.spawn(move || {
            waiting_signal.wait_until(Instant::now() + Duration::from_secs(30));
            let _ = tx.send(());
        });
        let observed = rx.recv_timeout(Duration::from_secs(1));
        signal.notify();
        waiter.join().unwrap();
        observed
    });
    assert!(
        returned.is_ok(),
        "owner transaction consumed concurrent transport notification"
    );

    // Re-enter the ordinary driver: the actual inbox ACK confirms only the
    // old sealed group. The second group still needs its own real response.
    leader.step().unwrap();
    assert_eq!(first.await.unwrap().index(), 1);
    assert_pending(second.as_mut()).await;
    cluster.drivers[1].step().unwrap();
    let second_messages = cluster.read_heartbeats();
    assert_ne!(second_messages[0].context, second_messages[2].context);
    let acknowledgement = cluster.held_ack(&second_messages[2].context);
    cluster
        .hub
        .endpoint(NodeId(2))
        .send(NodeId(1), acknowledgement);
    leader.step().unwrap();
    assert_eq!(second.await.unwrap().index(), 1);
    assert_eq!(leader.async_read_snapshot().in_flight, 0);
}
