//! Concurrent publication contracts for the single WorkSignal owner.
//! Channels select the interleavings; timeouts only bound failure and cleanup.

use super::WorkSignal;
use std::collections::VecDeque;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const COORDINATION: Duration = Duration::from_secs(2);
const OWNER_DEADLINE: Duration = Duration::from_secs(5);

/// Every gate has a bounded receive. Every owner wait has a bounded deadline.
/// Releasing gates and stopping before joining also covers assertion unwinding.
struct Threads {
    signal: Arc<WorkSignal>,
    gates: Vec<mpsc::Sender<()>>,
    handles: Vec<JoinHandle<()>>,
}

impl Threads {
    fn new(signal: &Arc<WorkSignal>) -> Self {
        Self {
            signal: signal.clone(),
            gates: Vec::new(),
            handles: Vec::new(),
        }
    }

    fn gate(&mut self) -> (mpsc::Sender<()>, mpsc::Receiver<()>) {
        let (tx, rx) = mpsc::channel();
        self.gates.push(tx.clone());
        (tx, rx)
    }

    fn spawn(&mut self, body: impl FnOnce() + Send + 'static) {
        self.handles.push(thread::spawn(body));
    }

    fn finish(&mut self) -> bool {
        for gate in &self.gates {
            let _ = gate.send(());
        }
        // A panic in faulty production code must not create a second unwind in
        // Drop. Owned threads still leave through their finite waits/deadlines.
        let mut clean = catch_unwind(AssertUnwindSafe(|| self.signal.stop())).is_ok();
        for handle in self.handles.drain(..) {
            clean &= handle.join().is_ok();
        }
        clean
    }
}

impl Drop for Threads {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

#[test]
fn concurrent_producer_burst_is_delivered_behind_one_pending_hint() {
    const PRODUCERS: usize = 4;
    const ITEMS: usize = 8;
    let signal = Arc::new(WorkSignal::default());
    let queue = Arc::new(Mutex::new(VecDeque::new()));
    let mut threads = Threads::new(&signal);

    let parked = signal.observe_next_park();
    let (woke_tx, woke_rx) = mpsc::channel();
    let (release_owner, owner_gate) = threads.gate();
    let (result_tx, result_rx) = mpsc::channel();
    let owner = signal.clone();
    let owned_queue = queue.clone();
    threads.spawn(move || {
        if !owner.begin_turn() {
            return;
        }
        owner.wait_until(Instant::now() + OWNER_DEADLINE);
        let _ = woke_tx.send(());
        // The first hint remains pending until all publishers have finished.
        if owner_gate.recv_timeout(OWNER_DEADLINE).is_err() {
            return;
        }
        if !owner.begin_turn() {
            return;
        }
        let delivered: Vec<_> = owned_queue.lock().unwrap().drain(..).collect();
        let _ = result_tx.send(delivered);
    });

    let observed_park = parked.recv_timeout(COORDINATION);
    queue.lock().unwrap().push_back(0);
    signal.notify();
    let first_wake = woke_rx.recv_timeout(COORDINATION);
    let (published_tx, published_rx) = mpsc::channel();
    for producer in 0..PRODUCERS {
        let queue = queue.clone();
        let signal = signal.clone();
        let done = published_tx.clone();
        threads.spawn(move || {
            for item in 0..ITEMS {
                queue.lock().unwrap().push_back(1 + producer * ITEMS + item);
                // No publication-queue mutex is held while taking WorkSignal.
                signal.notify();
            }
            let _ = done.send(producer);
        });
    }
    drop(published_tx);
    let published: Vec<_> = (0..PRODUCERS)
        .map(|_| published_rx.recv_timeout(COORDINATION))
        .collect();
    let released = release_owner.send(()).is_ok();
    let result = result_rx.recv_timeout(COORDINATION);
    let clean = threads.finish();

    assert!(clean, "producer burst left a failed owned thread");
    assert!(
        observed_park.is_ok(),
        "owner did not reach the initial producer park"
    );
    assert!(
        first_wake.is_ok(),
        "first publication did not wake the parked owner"
    );
    assert!(
        published.iter().all(Result::is_ok) && released,
        "concurrent publishers did not finish behind the held owner"
    );
    assert!(
        result.is_ok(),
        "pending producer burst was lost before owner consumption"
    );
    let delivered = result.unwrap();
    let mut sorted = delivered.clone();
    sorted.sort_unstable();
    assert_eq!(
        sorted,
        (0..=PRODUCERS * ITEMS).collect::<Vec<_>>(),
        "coalesced hint lost or duplicated published work"
    );
    for producer in 0..PRODUCERS {
        let first = 1 + producer * ITEMS;
        let observed: Vec<_> = delivered
            .iter()
            .copied()
            .filter(|value| (first..first + ITEMS).contains(value))
            .collect();
        assert_eq!(
            observed,
            (first..first + ITEMS).collect::<Vec<_>>(),
            "publication order changed within one producer"
        );
    }
    assert!(queue.lock().unwrap().is_empty());
}

#[test]
fn publication_during_bounded_drain_survives_to_the_next_owner_turn() {
    let signal = Arc::new(WorkSignal::default());
    let queue = Arc::new(Mutex::new(VecDeque::from([1, 2])));
    let mut threads = Threads::new(&signal);
    signal.notify();
    let parked = signal.observe_next_park();
    let (first_tx, first_rx) = mpsc::channel();
    let (continue_drain, drain_gate) = threads.gate();
    let (result_tx, result_rx) = mpsc::channel();
    let owner = signal.clone();
    let owned_queue = queue.clone();
    threads.spawn(move || {
        if !owner.begin_turn() {
            return;
        }
        // The first turn owns only this bounded prefix. Publish the checkpoint
        // after releasing the queue mutex, without finishing the owner turn.
        let first = owned_queue.lock().unwrap().pop_front();
        let _ = first_tx.send(first);
        if drain_gate.recv_timeout(OWNER_DEADLINE).is_err() {
            return;
        }
        owner.wait_until(Instant::now() + OWNER_DEADLINE);
        if !owner.begin_turn() {
            return;
        }
        let next: Vec<_> = owned_queue.lock().unwrap().drain(..).collect();
        let _ = result_tx.send(next);
    });

    let first = first_rx.recv_timeout(COORDINATION);
    let (published_tx, published_rx) = mpsc::channel();
    let producer = signal.clone();
    let published_queue = queue.clone();
    threads.spawn(move || {
        published_queue.lock().unwrap().push_back(3);
        producer.notify();
        let _ = published_tx.send(());
    });
    let published = published_rx.recv_timeout(COORDINATION);
    let released = continue_drain.send(()).is_ok();
    let result = result_rx.recv_timeout(COORDINATION);
    let attempted_park = parked.try_recv().is_ok();
    let clean = threads.finish();

    assert!(clean, "bounded drain left a failed owned thread");
    assert_eq!(
        first.ok(),
        Some(Some(1)),
        "bounded drain checkpoint was not reached"
    );
    assert!(
        published.is_ok() && released,
        "producer did not publish during the held drain"
    );
    assert!(
        result.is_ok() && !attempted_park,
        "publication during drain was lost before the next owner turn"
    );
    assert_eq!(
        result.unwrap(),
        vec![2, 3],
        "retained prefix or later publication was lost"
    );
    assert!(queue.lock().unwrap().is_empty());
}

#[test]
fn stop_wakes_a_parked_owner_and_later_publishers_cannot_restart_it() {
    let signal = Arc::new(WorkSignal::default());
    let queue = Arc::new(Mutex::new(VecDeque::new()));
    let mut threads = Threads::new(&signal);
    let parked = signal.observe_next_park();
    let (woke_tx, woke_rx) = mpsc::channel();
    let (release_check, check_gate) = threads.gate();
    let (result_tx, result_rx) = mpsc::channel();
    let owner = signal.clone();
    threads.spawn(move || {
        if !owner.begin_turn() {
            return;
        }
        owner.wait_until(Instant::now() + OWNER_DEADLINE);
        let _ = woke_tx.send(());
        if check_gate.recv_timeout(OWNER_DEADLINE).is_err() {
            return;
        }
        let restarted = owner.begin_turn();
        owner.wait_until(Instant::now() + OWNER_DEADLINE);
        let restarted_again = owner.begin_turn();
        let _ = result_tx.send((restarted, restarted_again));
    });

    // The observer is sent under the predicate mutex before Condvar releases
    // it. stop must then either prevent the park or wake the actual waiter.
    let observed_park = parked.recv_timeout(COORDINATION);
    signal.stop();
    let woke_without_producer = woke_rx.recv_timeout(COORDINATION);
    let (published_tx, published_rx) = mpsc::channel();
    for value in [7, 8] {
        let producer = signal.clone();
        let published_queue = queue.clone();
        let done = published_tx.clone();
        threads.spawn(move || {
            published_queue.lock().unwrap().push_back(value);
            producer.notify();
            let _ = done.send(());
        });
    }
    drop(published_tx);
    let published: Vec<_> = (0..2)
        .map(|_| published_rx.recv_timeout(COORDINATION))
        .collect();
    let released = release_check.send(()).is_ok();
    let result = result_rx.recv_timeout(COORDINATION);
    let clean = threads.finish();

    assert!(clean, "stop race left a failed owned thread");
    assert!(
        observed_park.is_ok(),
        "owner did not reach the predicate-protected park"
    );
    assert!(
        woke_without_producer.is_ok(),
        "stop left the parked owner waiting for an unrelated producer"
    );
    assert!(
        published.iter().all(Result::is_ok) && released,
        "post-stop publishers did not finish"
    );
    assert_eq!(
        result.ok(),
        Some((false, false)),
        "post-stop publication restarted the owner"
    );
    // Stop is terminal; these later publications have no claimed service.
    let mut retained: Vec<_> = queue.lock().unwrap().iter().copied().collect();
    retained.sort_unstable();
    assert_eq!(retained, vec![7, 8]);
}
