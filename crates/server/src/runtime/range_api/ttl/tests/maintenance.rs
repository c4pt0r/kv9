use super::*;
use crate::runtime::ttl_maintenance::Maintenance;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Condvar;

async fn until(mut predicate: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(6), async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("maintenance made no progress");
}
fn new_directory(f: &Fixture, clock: Option<Arc<dyn TtlClock>>) -> Arc<RawDirectory> {
    let directory = Arc::new(RawDirectory::with_clock(clock));
    directory.insert(f.group.clone());
    directory
}
fn value(f: &Fixture, key: &[u8], cf: ColumnFamily) -> Option<Vec<u8>> {
    f.group
        .engine
        .get(
            cf,
            &kv9_common::codec::encode_key(
                kv9_common::codec::KeyMode::Raw,
                f.group.binding.keyspace,
                key,
            )
            .unwrap(),
        )
        .unwrap()
}
async fn put_many(f: &Fixture, keys: &[Vec<u8>], ttl: u64) {
    for keys in keys.chunks(256) {
        f.call(Operation::Put {
            pairs: keys.iter().map(|k| (k.clone(), b"v".to_vec())).collect(),
            ttl_secs: Some(ttl),
        })
        .await
        .unwrap();
    }
}
fn keys(prefix: &str, n: usize) -> Vec<Vec<u8>> {
    (0..n)
        .map(|i| format!("{prefix}{i:04}").into_bytes())
        .collect()
}
fn republish(f: &Fixture, directory: &RawDirectory, max_log_entries: u64) {
    let group = RawGroup::new(
        f.group.binding.clone(),
        f.group.engine.clone(),
        f.group.driver.clone(),
        f.group.clock.clone(),
    );
    group
        .driver
        .peer()
        .set_max_write_log_entries(max_log_entries);
    directory.insert(group);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn background_time_and_empty_candidate_pages_progress_across_wrapper_republication() {
    let clock = Sample::at(1000);
    let f = Fixture::new(Some(clock.clone()));
    // More than one scan budget of live expiry records precedes the expired key.
    put_many(&f, &keys("a", 1024), 100).await;
    f.put(b"z", b"expired later", Some(1)).await;
    f.put(b"permanent", b"keep", None).await;
    clock.set(3000);
    let directory = new_directory(&f, Some(clock));
    let task = Maintenance::spawn(
        directory.clone(),
        Arc::new(|_| Ok(())),
        &tokio::runtime::Handle::current(),
    );
    until(|| task.snapshot().groups_with_cursor == 1).await;
    assert!(value(&f, b"z", ColumnFamily::Default).is_some());
    // Runtime reconciliation replaces RawGroup wrappers with the same owner.
    republish(&f, &directory, 0);
    until(|| value(&f, b"z", ColumnFamily::Default).is_none()).await;
    until(|| task.snapshot().candidates_applied == 1).await;
    let stats = task.snapshot();
    assert_eq!(stats.examined, 1025);
    assert_eq!(stats.pages, 2);
    assert_eq!(stats.sweeps, 1);
    assert_eq!(stats.observations_applied, 1);
    assert_eq!(stats.clock_refusals, 0);
    assert_eq!(stats.owner_changed, 0);
    assert!(value(&f, b"z", ColumnFamily::Write).is_none());
    assert!(value(&f, b"a0000", ColumnFamily::Default).is_some());
    assert!(value(&f, b"a0000", ColumnFamily::Write).is_some());
    assert_eq!(
        value(&f, b"permanent", ColumnFamily::Default),
        Some(b"keep".to_vec())
    );
    task.cancel();
    task.drain().await;
    drop(directory);
    f.close();
}

struct Gate {
    released: Mutex<bool>,
    wake: Condvar,
    entered: AtomicUsize,
}
impl Gate {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            released: Mutex::new(false),
            wake: Condvar::new(),
            entered: AtomicUsize::new(0),
        })
    }
    fn wait(&self) {
        self.entered.fetch_add(1, Ordering::SeqCst);
        let mut released = self.released.lock().unwrap();
        while !*released {
            released = self.wake.wait(released).unwrap();
        }
    }
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.wake.notify_all();
    }
}
// An assertion panic must release the fixture's blocking worker before Tokio drains.
struct Release(Arc<Gate>);
impl Drop for Release {
    fn drop(&mut self) {
        self.0.release();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stalled_group_keeps_its_slot_while_other_groups_progress_and_cancel_drains() {
    let f = Fixture::new(None);
    f.put(b"k", b"slow", Some(0)).await;
    let mut others = Vec::new();
    let directory = new_directory(&f, None);
    for id in [101, 102] {
        let mut binding = range();
        binding.region = RegionId(id);
        binding.keyspace = KeyspaceId(id as u32);
        let path = std::env::temp_dir().join(format!(
            "kv9-ttl-maintenance-{}",
            StoreIncarnation::mint().unwrap()
        ));
        std::fs::create_dir_all(&path).unwrap();
        let other = Fixture::open_range(path, None, binding);
        other.put(b"k", b"fast", Some(0)).await;
        directory.insert(other.group.clone());
        others.push(other);
    }
    let gate = Gate::new();
    let _release_on_panic = Release(gate.clone());
    let blocked = gate.clone();
    let task = Maintenance::spawn(
        directory.clone(),
        Arc::new(move |region| {
            if region == RegionId(100) {
                blocked.wait();
            }
            Ok(())
        }),
        &tokio::runtime::Handle::current(),
    );
    until(|| gate.entered.load(Ordering::SeqCst) == 1).await;
    until(|| {
        others
            .iter()
            .all(|f| value(f, b"k", ColumnFamily::Default).is_none())
    })
    .await;
    until(|| task.snapshot().candidates_applied == 2).await;
    assert_eq!(task.snapshot().peak_in_flight, 2);
    assert!(task.snapshot().clock_refusals >= 2);
    assert!(value(&f, b"k", ColumnFamily::Default).is_some());
    task.cancel();
    until(|| !task.snapshot().running).await;
    assert_eq!(task.snapshot().in_flight, 1);
    let drain = tokio::spawn(task.drain());
    tokio::task::yield_now().await;
    assert!(
        !drain.is_finished(),
        "cancellation released a still-blocked job"
    );
    gate.release();
    tokio::time::timeout(Duration::from_secs(3), drain)
        .await
        .unwrap()
        .unwrap();
    assert!(value(&f, b"k", ColumnFamily::Default).is_some());
    drop(directory);
    for other in others {
        other.close();
    }
    f.close();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn background_cleanup_does_not_delete_a_version_renewed_after_candidate_selection() {
    let f = Fixture::new(Some(Sample::at(1000)));
    f.put(b"k", b"old", Some(0)).await;
    let before = f.expiry(b"k").unwrap();
    // No maintenance clock: Cleanup may use only the committed watermark.
    let directory = new_directory(&f, None);
    let gate = Gate::new();
    let _release_on_panic = Release(gate.clone());
    let blocked = gate.clone();
    let calls = AtomicUsize::new(0);
    let task = Maintenance::spawn(
        directory.clone(),
        Arc::new(move |_| {
            // Guard immediately after collecting the same-snapshot candidate page.
            if calls.fetch_add(1, Ordering::SeqCst) == 3 {
                blocked.wait();
            }
            Ok(())
        }),
        &tokio::runtime::Handle::current(),
    );
    until(|| gate.entered.load(Ordering::SeqCst) == 1).await;
    f.put(b"k", b"renewed", Some(100)).await;
    let renewed = f.expiry(b"k").unwrap();
    assert_ne!(before, renewed);
    gate.release();
    until(|| task.snapshot().candidates_applied == 1).await;
    assert_eq!(
        value(&f, b"k", ColumnFamily::Default),
        Some(b"renewed".to_vec())
    );
    assert_eq!(f.expiry(b"k"), Some(renewed));
    task.cancel();
    task.drain().await;
    drop(directory);
    f.close();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn maintenance_respects_log_backpressure_and_resumes_with_the_same_owner() {
    let f = Fixture::new(None);
    f.put(b"k", b"expired", Some(0)).await;
    let directory = new_directory(&f, None);
    republish(&f, &directory, 1);
    let before = f.group.driver.status().applied_index;
    let task = Maintenance::spawn(
        directory.clone(),
        Arc::new(|_| Ok(())),
        &tokio::runtime::Handle::current(),
    );
    until(|| task.snapshot().backpressure == 1).await;
    assert_eq!(task.snapshot().groups_backing_off, 1);
    assert_eq!(task.snapshot().pages, 0);
    assert_eq!(f.group.driver.status().applied_index, before);
    assert!(value(&f, b"k", ColumnFamily::Default).is_some());
    republish(&f, &directory, 0);
    until(|| task.snapshot().candidates_applied == 1).await;
    assert!(value(&f, b"k", ColumnFamily::Default).is_none());
    assert!(value(&f, b"k", ColumnFamily::Write).is_none());
    task.cancel();
    task.drain().await;
    drop(directory);
    f.close();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelled_partial_sweep_restarts_from_durable_expiry_records_after_wal_reopen() {
    let mut f = Fixture::new(None);
    let keys = keys("k", 620);
    put_many(&f, &keys, 0).await;
    let directory = new_directory(&f, None);
    let task = Maintenance::spawn(
        directory.clone(),
        Arc::new(|_| Ok(())),
        &tokio::runtime::Handle::current(),
    );
    until(|| task.snapshot().pages == 1).await;
    task.cancel();
    task.drain().await;
    assert!(value(&f, keys.last().unwrap(), ColumnFamily::Write).is_some());
    let path = f.path.clone();
    drop(directory);
    f.stop();
    drop(f);
    let f = Fixture::open(path, None);
    let directory = new_directory(&f, None);
    let task = Maintenance::spawn(
        directory.clone(),
        Arc::new(|_| Ok(())),
        &tokio::runtime::Handle::current(),
    );
    until(|| task.snapshot().sweeps == 1).await;
    assert_eq!(task.snapshot().candidates_applied, 620 - 256);
    for key in keys {
        assert!(value(&f, &key, ColumnFamily::Default).is_none());
        assert!(value(&f, &key, ColumnFamily::Write).is_none());
    }
    task.cancel();
    task.drain().await;
    drop(directory);
    f.close();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unconfirmed_cleanup_holds_the_group_append_budget_until_its_position_is_consumed() {
    let f = Fixture::new(None);
    let keys = keys("k", 257);
    put_many(&f, &keys, 0).await;
    let directory = new_directory(&f, None);
    let driver = f.group.driver.clone();
    struct Unpause(Arc<NodeDriver<DiskRaftStorage, WalEngine>>);
    impl Drop for Unpause {
        fn drop(&mut self) {
            self.0.pause_apply(false);
        }
    }
    let _unpause_on_panic = Unpause(driver.clone());
    let calls = AtomicUsize::new(0);
    let task = Maintenance::spawn(
        directory.clone(),
        Arc::new(move |_| {
            // ReadIndex and candidate scan finished; the accepted Cleanup will
            // commit, but its exact apply wait must time out while apply is frozen.
            if calls.fetch_add(1, Ordering::SeqCst) == 3 {
                driver.pause_apply(true);
            }
            Ok(())
        }),
        &tokio::runtime::Handle::current(),
    );
    until(|| task.snapshot().groups_awaiting_apply == 1).await;
    assert_eq!(task.snapshot().unconfirmed_submissions, 1);
    assert_eq!(task.snapshot().pages, 0);
    assert_eq!(task.snapshot().candidates_applied, 0);
    let committed = f.group.driver.status().raft_committed;
    assert!(committed > f.group.driver.status().applied_index);
    until(|| task.snapshot().failures >= 2).await;
    assert_eq!(f.group.driver.status().raft_committed, committed);
    assert_eq!(task.snapshot().unconfirmed_submissions, 1);
    assert!(value(&f, &keys[0], ColumnFamily::Default).is_some());
    f.group.driver.pause_apply(false);
    until(|| task.snapshot().sweeps == 1).await;
    assert_eq!(task.snapshot().groups_awaiting_apply, 0);
    // The earlier late apply is never relabelled as an exact receipt. Only
    // the one remaining key was checked by a subsequently confirmed command.
    assert_eq!(task.snapshot().candidates_applied, 1);
    for key in keys {
        assert!(value(&f, &key, ColumnFamily::Default).is_none());
        assert!(value(&f, &key, ColumnFamily::Write).is_none());
    }
    task.cancel();
    task.drain().await;
    drop(directory);
    f.close();
}
