//! Actual Raft and authenticated duplex RPCs. Attempts have no local journal;
//! discovery must come entirely from committed metadata gates.
use super::*;
use crate::checkpoint_retention::{CheckpointLedger, CheckpointOwners};
use crate::runtime::checkpoint_retention::RoutedLedger;
use crate::runtime::checkpoint_sweep::{Maintenance, Reconciler, Stats};
use kv9_common::retention::{OwnerId, OwnerToken, PinPhase};
use kv9_engine::checkpoint::RemoteUploader;
use kv9_engine::MemoryObjectStore;
use kv9_meta::retention::checkpoint_admission::{executors, retirement, sweep};
use kv9_meta::retention::{encode_request, LedgerRequest, OwnerKind};
use tokio_util::sync::CancellationToken;
mod bootstrap_index;
mod build_index;
mod closure;
mod data_membership;
mod data_store_history;
mod reader_index;
mod refusal;
mod remote;
mod settlement;

// A completed owner census must leave owners, indexes and placement unchanged.
// The shared lane may still commit membership observations (namespaces 52/53)
// and confirmation Noops, so the global Raft applied index is not a census cursor.
fn owner_and_placement_rows(f: &Fixture, leader: usize) -> Vec<(Vec<u8>, Vec<u8>)> {
    let view = f.backends[leader]
        .node
        .meta_raft
        .store
        .engine()
        .snapshot()
        .unwrap();
    let mut start = b"\0kv9\0retention_v1\0".to_vec();
    let mut end = start.clone();
    start.push(2); // Exclude the global ledger revision (namespace 1).
    end.push(52); // Include all owner/census/placement state preceding membership.
    view.iter(ColumnFamily::Default, &start, &end)
        .unwrap()
        .collect::<Result<Vec<_>>>()
        .unwrap()
}

struct Attempt {
    id: OwnerId,
    tokens: Vec<(OwnerToken, bool)>,
}
async fn seed(f: &Fixture, held: u8) -> (RootDescriptor, Vec<Attempt>) {
    let backend = f.backends[1].clone();
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    tokio::task::spawn_blocking(move || {
        let root = backend.ttl_context().unwrap().root.clone();
        let uploader = RemoteUploader::for_harness(Arc::new(MemoryObjectStore::new()));
        let frozen = backend
            .node
            .meta_raft
            .store
            .engine()
            .freeze_with_scope(|view| {
                Ok(
                    kv9_meta::checkpoint::inspect_initial_checkpoint_base(view, &root)?
                        .flush_scope(),
                )
            })
            .unwrap();
        let plan = uploader.plan_checkpoint(frozen).unwrap();
        let mut attempts = Vec::new();
        for predecessor in 0..2 {
            let owners = CheckpointOwners::new(&root, plan.manifest(), predecessor).unwrap();
            owners.before_io(&ledger).unwrap();
            let intent = ledger.intent(&root, owners.pending_id()).unwrap().unwrap();
            let pending = intent.binding(OwnerKind::Pending).unwrap();
            let mut tokens = Vec::new();
            for n in 1..=held + 1 {
                let mut lifetime = ledger.local_lifetime().unwrap();
                lifetime.process = [n; 16];
                let binding =
                    kv9_meta::retention::checkpoint_admission::executor_binding(&intent, lifetime)
                        .unwrap();
                ledger
                    .apply(
                        encode_request(
                            root.digest(),
                            &LedgerRequest::Share {
                                from: pending.token(),
                                to: binding.clone(),
                            },
                        )
                        .unwrap(),
                    )
                    .unwrap();
                let published = n == held + 1;
                if published {
                    ledger
                        .apply(
                            encode_request(root.digest(), &LedgerRequest::Publish(binding.token()))
                                .unwrap(),
                        )
                        .unwrap();
                }
                tokens.push((binding.token(), published));
            }
            owners.close_admission(&ledger).unwrap();
            attempts.push(Attempt {
                id: owners.pending_id(),
                tokens,
            });
        }
        attempts.sort_by_key(|a| a.id);
        (root, attempts)
    })
    .await
    .unwrap()
}
fn observation(
    f: &Fixture,
    leader: usize,
    root: &RootDescriptor,
    id: OwnerId,
) -> sweep::SweepObservation {
    sweep::observe(
        f.backends[leader]
            .node
            .meta_raft
            .store
            .engine()
            .snapshot()
            .unwrap()
            .as_ref(),
        root,
        id,
    )
    .unwrap()
}
fn cursor(
    f: &Fixture,
    leader: usize,
    root: &RootDescriptor,
    id: OwnerId,
) -> (sweep::SweepCursor, usize) {
    match observation(f, leader, root, id) {
        sweep::SweepObservation::Page { cursor, page } => (
            cursor,
            page.owners
                .iter()
                .filter(|o| o.phase == PinPhase::Held)
                .count(),
        ),
        _ => panic!("expected unfinished page"),
    }
}
async fn pass(state: &mut Reconciler, f: &Fixture, leader: usize) -> Result<Stats> {
    let mut moved = std::mem::take(state);
    let backend = f.backends[leader].clone();
    // Deliberately use another node's authenticated route so the fixture can
    // drop actual commit receipts without replacing the planner or Raft apply.
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    let (restored, stats, result) = tokio::task::spawn_blocking(move || {
        let mut stats = Stats::default();
        let result = moved.pass(
            &backend,
            &ledger,
            &CancellationToken::new(),
            Instant::now() + Duration::from_secs(5),
            &mut stats,
        );
        (moved, stats, result)
    })
    .await
    .unwrap();
    *state = restored;
    result.map(|_| stats)
}
async fn until(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(20), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("checkpoint sweep deadline");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn journal_free_sweep_rotates_attempts_and_resumes_lost_retire_and_cursor_receipts_on_new_leader(
) {
    let mut f = Fixture::new().await;
    let (root, attempts) = seed(&f, 9).await;
    let first = attempts[0].id;
    let second = attempts[1].id;
    let mut state = Reconciler::default();
    f.hooks[1].retention_calls.store(0, Ordering::SeqCst);
    f.hooks[1].retention_commits.store(0, Ordering::SeqCst);
    f.hooks[1].retention_fail_after.store(1, Ordering::SeqCst);
    assert!(pass(&mut state, &f, 1).await.is_err());
    assert_eq!(f.hooks[1].retention_commits.load(Ordering::SeqCst), 1);
    assert_eq!(cursor(&f, 1, &root, first).0.sequence, 0);
    let view = f.backends[1]
        .node
        .meta_raft
        .store
        .engine()
        .snapshot()
        .unwrap();
    assert_eq!(
        executors(view.as_ref(), &root, first, None, 16)
            .unwrap()
            .owners
            .iter()
            .filter(|o| o.phase == PinPhase::Held)
            .count(),
        8
    );
    drop(view);
    f.hooks[1].retention_fail_after.store(0, Ordering::SeqCst);
    let stats = pass(&mut state, &f, 1).await.unwrap();
    assert_eq!(stats.examined, 4);
    assert_eq!(
        cursor(&f, 1, &root, second).0.sequence,
        1,
        "another attempt gets service before the failed page is retried"
    );
    assert_eq!(pass(&mut state, &f, 1).await.unwrap().wraps, 1);
    state = Reconciler::default(); // process-local discovery cursor lost
    let remaining = cursor(&f, 1, &root, first).1;
    let commits = f.hooks[1].retention_commits.load(Ordering::SeqCst);
    f.hooks[1]
        .retention_fail_after
        .store(commits + remaining + 1, Ordering::SeqCst);
    assert!(
        pass(&mut state, &f, 1).await.is_err(),
        "lose the cursor commit receipt"
    );
    assert_eq!(cursor(&f, 1, &root, first).0.sequence, 1);
    f.hooks[1].retention_fail_after.store(0, Ordering::SeqCst);
    state = Reconciler::default();
    pass(&mut state, &f, 1).await.unwrap();
    assert_eq!(
        cursor(&f, 1, &root, first).0.sequence,
        2,
        "read durable cursor instead of starting over"
    );
    f.meta[1].peer().transfer_leader_for_tests(NodeId(3));
    until(|| f.meta[2].status().role == Role::Leader && f.meta[1].status().role != Role::Leader)
        .await;
    assert!(
        pass(&mut state, &f, 1).await.is_err(),
        "former leader cannot dispatch"
    );
    state = Reconciler::default();
    for _ in 0..6 {
        pass(&mut state, &f, 2).await.unwrap();
    }
    for attempt in &attempts {
        assert!(matches!(
            observation(&f, 2, &root, attempt.id),
            sweep::SweepObservation::Done
        ));
    }
    let applied = f.meta[2].status().applied_index;
    for _ in 0..6 {
        pass(&mut state, &f, 2).await.unwrap();
    }
    assert_eq!(
        f.meta[2].status().applied_index,
        applied,
        "completed sweeps must not append progress forever"
    );
    for server in f.servers.drain(..) {
        server.abort();
        let _ = server.await;
    }
    for driver in f.meta.iter().chain(f.data.iter()) {
        driver.stop();
    }
    for thread in f.owners.drain(..) {
        thread.join().unwrap();
    }
    f.backends.clear();
    f.meta.clear();
    f.data.clear();
    f.hooks.clear();
    let (engine, _) = WalEngine::open(f.path.join("n3/meta.wal")).unwrap();
    let view = engine.snapshot().unwrap();
    for attempt in attempts {
        assert!(matches!(
            sweep::observe(view.as_ref(), &root, attempt.id).unwrap(),
            sweep::SweepObservation::Done
        ));
        for (token, published) in attempt.tokens {
            let proof = retirement::unpublished_retirement(view.as_ref(), &root, attempt.id, token)
                .unwrap();
            assert_eq!(proof.is_some(), !published);
            assert_eq!(
                kv9_meta::retention::retention_owner(view.as_ref(), &root, token.owner)
                    .unwrap()
                    .unwrap()
                    .phase,
                if published {
                    PinPhase::Published
                } else {
                    PinPhase::Released
                }
            );
        }
        assert_eq!(
            kv9_meta::retention::retention_owner(view.as_ref(), &root, attempt.id)
                .unwrap()
                .unwrap()
                .phase,
            PinPhase::Published
        );
    }
    drop(view);
    drop(engine);
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn background_leader_discovers_without_uploader_and_corrupt_attempt_does_not_starve_the_next()
{
    let f = Fixture::new().await;
    let (root, attempts) = seed(&f, 1).await;
    assert!(f.backends.iter().all(|backend| backend.uploader.is_none()));
    let engine = f.backends[1].node.meta_raft.store.engine();
    // Inject a damaged value under an otherwise valid discovery key. Invalid
    // key/header corruption intentionally refuses the whole scan instead.
    let mut key = b"\0kv9\0retention_v1\0".to_vec();
    key.push(9);
    key.extend_from_slice(attempts[0].id.as_bytes());
    assert!(engine
        .snapshot()
        .unwrap()
        .get(ColumnFamily::Default, &key)
        .unwrap()
        .is_some());
    let mut batch = kv9_engine::WriteBatch::new();
    batch.put(ColumnFamily::Default, key, vec![0; 136]);
    engine.write(batch).unwrap();
    let tasks: Vec<_> = f
        .backends
        .iter()
        .map(|backend| Maintenance::start(backend.clone(), &tokio::runtime::Handle::current()))
        .collect();
    until(|| tasks[1].snapshot().pages >= 1 && tasks[1].snapshot().failures >= 2).await;
    assert!(matches!(
        observation(&f, 1, &root, attempts[1].id),
        sweep::SweepObservation::Done
    ));
    assert_eq!(tasks[1].snapshot().peak_in_flight, 1);
    assert_eq!(tasks[0].snapshot().discovered, 0);
    assert_eq!(tasks[2].snapshot().discovered, 0);
    assert!(!tasks[1].stopped_unexpectedly());
    for task in &tasks {
        task.cancel();
    }
    for task in tasks {
        task.drain().await;
    }
    assert!(f
        .backends
        .iter()
        .all(|backend| backend.maintenance.snapshot().active == 0));
    f.close().await;
}
