use super::*;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc as blocking, Condvar};
use std::task::Wake;
use std::time::Duration;

#[derive(Default)]
struct WakeCount(AtomicUsize);

impl Wake for WakeCount {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn counter() -> (Arc<WakeCount>, Waker) {
    let count = Arc::new(WakeCount::default());
    (count.clone(), Waker::from(count))
}

fn destination(port: u16) -> Arc<PeerDestination> {
    Arc::new(PeerDestination {
        addr: ([127, 0, 0, 1], port).into(),
    })
}

fn message(destination: &Arc<PeerDestination>, id: u64, bytes: usize) -> OutboundMessage {
    OutboundMessage {
        destination: destination.clone(),
        envelope: pb::RaftEnvelope {
            from_node: id,
            raft_message: vec![7; bytes],
            ..Default::default()
        },
    }
}

fn poll_body(body: &mut Body, waker: &Waker) -> Poll<Option<pb::BatchRaftMessage>> {
    Pin::new(body).poll_next(&mut Context::from_waker(waker))
}

fn batch(body: &mut Body, waker: &Waker) -> pb::BatchRaftMessage {
    match poll_body(body, waker) {
        Poll::Ready(Some(batch)) => batch,
        Poll::Ready(None) => panic!("body closed before delivering the queued batch"),
        Poll::Pending => panic!("already queued valid work did not flush"),
    }
}

fn ids(batch: &pb::BatchRaftMessage) -> Vec<u64> {
    batch.msgs.iter().map(|message| message.from_node).collect()
}

fn poll_future<F: Future>(future: Pin<&mut F>, waker: &Waker) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(waker))
}

#[test]
fn route_generation_filter_rejects_old_queue_entries_even_after_address_reuse() {
    let first_a = destination(1);
    let b = destination(2);
    let new_a = destination(1);
    let (tx, rx) = channel();
    for (route, id) in [(&first_a, 10), (&b, 20), (&new_a, 30)] {
        tx.try_send(message(route, id, 0)).unwrap();
    }
    let (_session, mut body) = rx.open(new_a, RootDigest::from_bytes([1; 32]));
    drop(tx);
    let (_, waker) = counter();
    let delivered = batch(&mut body, &waker);
    assert_eq!(ids(&delivered), vec![30]);
    assert_eq!(delivered.root_digest, vec![1; 32]);
    assert!(matches!(poll_body(&mut body, &waker), Poll::Ready(None)));
}

#[test]
fn queued_batch_bounds_stale_work_and_keeps_same_address_generations_separate() {
    let route = destination(1);
    let stale = destination(1);
    let (tx, rx) = channel();
    tx.try_send(message(&route, 1, 0)).unwrap();
    for index in 0..MAX_BATCH_MSGS {
        tx.try_send(message(&stale, index as u64 + 2, 0)).unwrap();
    }
    tx.try_send(message(&route, 999, 0)).unwrap();
    let (_session, mut body) = rx.open(route, RootDigest::from_bytes([1; 32]));
    let (_, waker) = counter();
    let first = batch(&mut body, &waker);
    assert_eq!(ids(&first), vec![1], "stale generation entered the batch");
    assert_eq!(first.root_digest, vec![1; 32]);
    assert_eq!(
        tx.capacity(),
        PEER_QUEUE - 2,
        "one poll inspected too much stale work"
    );
    assert_eq!(ids(&batch(&mut body, &waker)), vec![999]);
}

#[test]
fn queued_batch_flushes_available_work_and_preserves_fifo_at_count_and_byte_bounds() {
    let route = destination(1);
    let (tx, rx) = channel();
    let (_session, mut body) = rx.open(route.clone(), RootDigest::from_bytes([0; 32]));
    let (_, waker) = counter();
    tx.try_send(message(&route, 42, 0)).unwrap();
    assert_eq!(
        ids(&batch(&mut body, &waker)),
        vec![42],
        "idle first message waited for a batch"
    );
    for index in 0..=MAX_BATCH_MSGS {
        tx.try_send(message(&route, index as u64, 0)).unwrap();
    }
    assert_eq!(
        ids(&batch(&mut body, &waker)),
        (0..MAX_BATCH_MSGS as u64).collect::<Vec<_>>()
    );
    assert_eq!(tx.capacity(), PEER_QUEUE - 1);
    assert_eq!(ids(&batch(&mut body, &waker)), vec![MAX_BATCH_MSGS as u64]);

    tx.try_send(message(&route, 0, MAX_BATCH_BYTES / 2))
        .unwrap();
    for id in 1..=2 {
        tx.try_send(message(&route, id, MAX_BATCH_BYTES / 2 + 1))
            .unwrap();
    }
    let crossing = batch(&mut body, &waker);
    assert_eq!(ids(&crossing), vec![0, 1]);
    assert_eq!(
        crossing
            .msgs
            .iter()
            .map(|m| m.raft_message.len())
            .sum::<usize>(),
        MAX_BATCH_BYTES + 1
    );
    assert_eq!(ids(&batch(&mut body, &waker)), vec![2]);

    tx.try_send(message(&route, 7, MAX_BATCH_BYTES + 1))
        .unwrap();
    tx.try_send(message(&route, 8, 0)).unwrap();
    assert_eq!(
        ids(&batch(&mut body, &waker)),
        vec![7],
        "soft byte target must permit the first legal envelope"
    );
    assert_eq!(ids(&batch(&mut body, &waker)), vec![8]);
}

fn retained_old_body_cannot_steal_replacement_waker(aba: bool) {
    let first_a = destination(1);
    let (tx, rx) = channel();
    let (old_session, mut old_body) = rx.open(first_a.clone(), RootDigest::from_bytes([0; 32]));
    let (_, old_waker) = counter();
    assert!(poll_body(&mut old_body, &old_waker).is_pending());
    let middle = aba.then(|| rx.open(destination(2), RootDigest::from_bytes([0; 32])));
    let route = if aba { destination(1) } else { first_a };
    let (new_session, mut new_body) = rx.open(route.clone(), RootDigest::from_bytes([0; 32]));
    let (new_count, new_waker) = counter();
    assert!(poll_body(&mut new_body, &new_waker).is_pending());
    let (intruder_count, intruder_waker) = counter();
    assert!(matches!(
        poll_body(&mut old_body, &intruder_waker),
        Poll::Ready(None)
    ));
    if let Some((session, mut body)) = middle {
        assert!(matches!(
            poll_body(&mut body, &intruder_waker),
            Poll::Ready(None)
        ));
        drop(body);
        drop(session);
    }
    drop(old_body);
    drop(old_session);
    assert!(owns(&rx.0.state.lock().unwrap(), &new_session.token));
    tx.try_send(message(&route, 900, 0)).unwrap();
    assert_eq!(
        new_count.0.load(Ordering::SeqCst),
        1,
        "replacement waker was overwritten or removed"
    );
    assert_eq!(intruder_count.0.load(Ordering::SeqCst), 0);
    assert_eq!(ids(&batch(&mut new_body, &new_waker)), vec![900]);
}

#[test]
fn same_route_reconnect_fences_retained_body_and_its_drop() {
    retained_old_body_cannot_steal_replacement_waker(false);
}

#[test]
fn aba_replacement_fences_both_retained_bodies_and_their_drops() {
    retained_old_body_cannot_steal_replacement_waker(true);
}

#[derive(Default)]
struct ReleaseGate {
    released: Mutex<bool>,
    changed: Condvar,
    timed_out: AtomicBool,
}

impl ReleaseGate {
    fn release(&self) {
        *self.released.lock().unwrap_or_else(|e| e.into_inner()) = true;
        self.changed.notify_all();
    }

    fn wait(&self) {
        let guard = self.released.lock().unwrap_or_else(|e| e.into_inner());
        let (guard, _) = self
            .changed
            .wait_timeout_while(guard, Duration::from_secs(5), |v| !*v)
            .unwrap_or_else(|e| e.into_inner());
        if !*guard {
            self.timed_out.store(true, Ordering::SeqCst);
        }
    }
}

struct ReleaseOnDrop(Arc<ReleaseGate>);

impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.release();
    }
}

struct HeldWake {
    entered: Mutex<Option<blocking::Sender<()>>>,
    gate: Arc<ReleaseGate>,
}

impl Wake for HeldWake {
    fn wake(self: Arc<Self>) {
        let entered = self
            .entered
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(entered) = entered {
            let _ = entered.send(());
            self.gate.wait();
        }
    }
}

#[test]
fn invalidation_linearizes_before_wake_cleanup_and_replacement_dequeue() {
    let route = destination(1);
    let (tx, rx) = channel();
    let (session, mut old_body) = rx.open(route.clone(), RootDigest::from_bytes([0; 32]));
    let gate = Arc::new(ReleaseGate::default());
    let (entered_tx, entered_rx) = blocking::channel();
    let held_waker = Waker::from(Arc::new(HeldWake {
        entered: Mutex::new(Some(entered_tx)),
        gate: gate.clone(),
    }));
    assert!(poll_body(&mut old_body, &held_waker).is_pending());
    std::thread::scope(|scope| {
        // Releases on assertion failure before scope joins the invalidating thread.
        let _release = ReleaseOnDrop(gate.clone());
        let invalidating = scope.spawn(move || drop(session));
        entered_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("invalidation did not reach wake cleanup");
        {
            let state =
                rx.0.state
                    .try_lock()
                    .expect("invalidation invoked a waker under the queue lock");
            assert!(
                state.active.is_none(),
                "wake happened before token invalidation"
            );
        }
        let (_new_session, mut new_body) = rx.open(route.clone(), RootDigest::from_bytes([0; 32]));
        let (_, waker) = counter();
        tx.try_send(message(&route, 77, 0)).unwrap();
        assert!(matches!(
            poll_body(&mut old_body, &waker),
            Poll::Ready(None)
        ));
        assert_eq!(
            tx.capacity(),
            PEER_QUEUE - 1,
            "invalidated body stole replacement work"
        );
        assert_eq!(ids(&batch(&mut new_body, &waker)), vec![77]);
        assert!(!gate.timed_out.load(Ordering::SeqCst));
        gate.release();
        invalidating.join().unwrap();
    });
}

#[tokio::test(flavor = "current_thread")]
async fn abort_before_first_poll_invalidates_retained_body_and_allows_replacement() {
    let route = destination(1);
    let (tx, rx) = channel();
    let (session, mut retained) = rx.open(route.clone(), RootDigest::from_bytes([0; 32]));
    let polled = Arc::new(AtomicBool::new(false));
    let observed = polled.clone();
    let owner = tokio::spawn(async move {
        let _session = session;
        observed.store(true, Ordering::SeqCst);
        std::future::pending::<()>().await;
    });
    owner.abort();
    let error = tokio::time::timeout(Duration::from_secs(1), owner)
        .await
        .unwrap()
        .unwrap_err();
    assert!(error.is_cancelled());
    assert!(
        !polled.load(Ordering::SeqCst),
        "abort-before-first-poll fixture was not armed"
    );
    assert!(rx.0.state.lock().unwrap().active.is_none());
    let (_new_session, mut replacement) = rx.open(route.clone(), RootDigest::from_bytes([0; 32]));
    let (_, waker) = counter();
    tx.try_send(message(&route, 55, 0)).unwrap();
    assert!(matches!(
        poll_body(&mut retained, &waker),
        Poll::Ready(None)
    ));
    drop(retained);
    assert_eq!(ids(&batch(&mut replacement, &waker)), vec![55]);
}

#[test]
fn receiver_owner_drop_closes_retained_body_and_releases_queued_work() {
    for queued in [false, true] {
        let route = destination(1);
        let (tx, rx) = channel();
        let (_session, mut body) = rx.open(route.clone(), RootDigest::from_bytes([0; 32]));
        let (count, waker) = counter();
        if queued {
            tx.try_send(message(&route, 1, 64)).unwrap();
        } else {
            assert!(poll_body(&mut body, &waker).is_pending());
        }
        drop(rx);
        assert_eq!(tx.capacity(), PEER_QUEUE);
        assert!(tx.try_send(message(&route, 2, 0)).is_err());
        assert!(matches!(poll_body(&mut body, &waker), Poll::Ready(None)));
        if !queued {
            assert_eq!(
                count.0.load(Ordering::SeqCst),
                1,
                "receiver drop did not wake the idle retained body"
            );
        }
    }
}

#[test]
fn stale_only_turn_self_wakes_and_next_valid_message_flushes_without_new_send() {
    let route = destination(1);
    let stale = destination(1);
    let (tx, rx) = channel();
    for id in 0..MAX_BATCH_MSGS {
        tx.try_send(message(&stale, id as u64, 0)).unwrap();
    }
    tx.try_send(message(&route, 777, 0)).unwrap();
    let (_session, mut body) = rx.open(route, RootDigest::from_bytes([0; 32]));
    let (count, waker) = counter();
    assert!(poll_body(&mut body, &waker).is_pending());
    assert_eq!(
        count.0.load(Ordering::SeqCst),
        1,
        "stale-only turn arranged no continuation"
    );
    assert_eq!(
        tx.capacity(),
        PEER_QUEUE - 1,
        "leading stale inspection exceeded one turn"
    );
    assert_eq!(ids(&batch(&mut body, &waker)), vec![777]);
}

#[tokio::test(flavor = "current_thread")]
async fn stalled_watchdog_expires_with_unpolled_body_and_retains_backlog() {
    let route = destination(1);
    let (tx, rx) = channel();
    let (session, _unpolled_body) = rx.open(route.clone(), RootDigest::from_bytes([0; 32]));
    tx.try_send(message(&route, 1, 0)).unwrap();
    rx.0.state.lock().unwrap().pending_since =
        Some(Instant::now() - STREAM_PROGRESS_BUDGET - Duration::from_millis(1));
    tokio::time::timeout(Duration::from_secs(1), session.stalled())
        .await
        .expect("watchdog depended on body polling");
    assert_eq!(
        tx.capacity(),
        PEER_QUEUE - 1,
        "watchdog consumed the body's queued work"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn idle_session_does_not_expire_and_first_queued_work_wakes_watchdog() {
    let route = destination(1);
    let (tx, rx) = channel();
    let (session, _unpolled_body) = rx.open(route.clone(), RootDigest::from_bytes([0; 32]));
    assert!(
        tokio::time::timeout(
            STREAM_PROGRESS_BUDGET + Duration::from_millis(100),
            session.stalled()
        )
        .await
        .is_err(),
        "healthy idle stream churned after the progress budget"
    );
    let (count, waker) = counter();
    let mut watch = Box::pin(session.stalled());
    assert!(poll_future(watch.as_mut(), &waker).is_pending());
    tx.try_send(message(&route, 1, 0)).unwrap();
    assert!(
        count.0.load(Ordering::SeqCst) > 0,
        "empty-to-nonempty transition lost the owner wake"
    );
    rx.0.state.lock().unwrap().pending_since =
        Some(Instant::now() - STREAM_PROGRESS_BUDGET - Duration::from_millis(1));
    tokio::time::timeout(Duration::from_secs(1), watch)
        .await
        .unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn sends_stale_inspection_and_notifications_do_not_reset_stall_age() {
    let route = destination(1);
    let stale = destination(1);
    let (tx, rx) = channel();
    let (session, mut body) = rx.open(route.clone(), RootDigest::from_bytes([0; 32]));
    tx.try_send(message(&stale, 0, 0)).unwrap();
    let expired = Instant::now() - STREAM_PROGRESS_BUDGET - Duration::from_millis(1);
    rx.0.state.lock().unwrap().pending_since = Some(expired);
    for id in 1..=MAX_BATCH_MSGS {
        tx.try_send(message(&stale, id as u64, 0)).unwrap();
        rx.0.changed.notify_one();
    }
    tx.try_send(message(&route, 900, 0)).unwrap();
    let (_, waker) = counter();
    assert!(poll_body(&mut body, &waker).is_pending());
    // Release the guard before asserting: an intended mutation failure must
    // not poison the queue and cause a second panic during owner cleanup.
    let observed_pending = rx.0.state.lock().unwrap().pending_since;
    assert_eq!(
        observed_pending,
        Some(expired),
        "stale traffic reset the backlog age"
    );
    assert_eq!(tx.capacity(), PEER_QUEUE - 2);
    tokio::time::timeout(Duration::from_secs(1), session.stalled())
        .await
        .expect("stale traffic postponed the independent watchdog");
}

#[tokio::test(flavor = "current_thread")]
async fn valid_batch_progress_resets_armed_budget_and_empty_queue_clears_it() {
    let route = destination(1);
    let (tx, rx) = channel();
    let (session, mut body) = rx.open(route.clone(), RootDigest::from_bytes([0; 32]));
    for id in 0..=MAX_BATCH_MSGS {
        tx.try_send(message(&route, id as u64, 0)).unwrap();
    }
    let nearly_expired = Instant::now() - STREAM_PROGRESS_BUDGET + Duration::from_millis(100);
    rx.0.state.lock().unwrap().pending_since = Some(nearly_expired);
    let (_, waker) = counter();
    let mut watch = Box::pin(session.stalled());
    assert!(poll_future(watch.as_mut(), &waker).is_pending());
    let before_progress = Instant::now();
    assert_eq!(batch(&mut body, &waker).msgs.len(), MAX_BATCH_MSGS);
    let after_progress = rx.0.state.lock().unwrap().pending_since.unwrap();
    assert!(after_progress >= before_progress && after_progress > nearly_expired);
    assert!(
        tokio::time::timeout(Duration::from_millis(150), watch.as_mut())
            .await
            .is_err(),
        "an already-armed old deadline ignored real dequeue progress"
    );
    assert_eq!(ids(&batch(&mut body, &waker)), vec![MAX_BATCH_MSGS as u64]);
    assert!(rx.0.state.lock().unwrap().pending_since.is_none());
    assert!(poll_future(watch.as_mut(), &waker).is_pending());
}

#[tokio::test(flavor = "current_thread")]
async fn sender_close_drains_last_batch_and_completes_idle_watchdog() {
    let route = destination(1);
    let (tx, rx) = channel();
    let (session, mut body) = rx.open(route.clone(), RootDigest::from_bytes([0; 32]));
    let (_, waker) = counter();
    assert!(poll_body(&mut body, &waker).is_pending());
    tx.try_send(message(&route, 1, 0)).unwrap();
    drop(tx);
    assert!(rx.sender_closed());
    assert_eq!(ids(&batch(&mut body, &waker)), vec![1]);
    assert!(matches!(poll_body(&mut body, &waker), Poll::Ready(None)));
    tokio::time::timeout(Duration::from_secs(1), session.stalled())
        .await
        .unwrap();
}

#[test]
fn single_queue_capacity_and_outage_discard_preserve_the_bound() {
    let route = destination(1);
    let (tx, rx) = channel();
    for id in 0..PEER_QUEUE {
        tx.try_send(message(&route, id as u64, 0)).unwrap();
    }
    assert_eq!(tx.capacity(), 0);
    assert!(tx.try_send(message(&route, 9999, 0)).is_err());
    rx.discard_outage();
    assert_eq!(tx.capacity(), PEER_QUEUE);
    assert!(rx.0.state.lock().unwrap().pending_since.is_none());
    tx.try_send(message(&route, 42, 0)).unwrap();
    let (_session, mut body) = rx.open(route, RootDigest::from_bytes([0; 32]));
    let (_, waker) = counter();
    assert_eq!(ids(&batch(&mut body, &waker)), vec![42]);
}

#[tokio::test(flavor = "current_thread")]
async fn ready_batches_exhaust_task_budget_without_dequeueing_on_pending() {
    // A spawned task has a cooperative budget; block_on's root future need not.
    let task = tokio::spawn(async {
        let route = destination(1);
        let (tx, rx) = channel();
        let (_session, mut body) = rx.open(route.clone(), RootDigest::from_bytes([0; 32]));
        let ready_count = std::future::poll_fn(|cx| {
            for id in 0..MAX_BATCH_MSGS * 4 {
                tx.try_send(message(&route, id as u64, 0)).unwrap();
                match Pin::new(&mut body).poll_next(cx) {
                    Poll::Ready(Some(value)) => assert_eq!(ids(&value), vec![id as u64]),
                    Poll::Ready(None) => panic!("active body closed while exercising cooperation"),
                    Poll::Pending => {
                        assert!(id > 0, "task began with no budget for the fixture");
                        assert!(
                            !tokio::task::coop::has_budget_remaining(),
                            "body returned Pending for a reason other than exhaustion"
                        );
                        assert_eq!(
                            tx.capacity(),
                            PEER_QUEUE - 1,
                            "exhausted poll touched the queue"
                        );
                        return Poll::Ready(id);
                    }
                }
            }
            panic!("repeated Ready batches bypassed Tokio's cooperative budget");
        })
        .await;
        tokio::task::yield_now().await;
        let (_, waker) = counter();
        assert_eq!(ids(&batch(&mut body, &waker)), vec![ready_count as u64]);
        assert_eq!(tx.capacity(), PEER_QUEUE);
    });
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap();
}
