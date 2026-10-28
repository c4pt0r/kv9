use super::*;
use kv9_meta::retention::checkpoint_admission::{
    enrollment::{history, GenerationState},
    executor_binding,
};
use kv9_meta::retention::checkpoint_intent::CheckpointIntent;
use kv9_meta::retention::{encode_request, LedgerRequest, OwnerKind};
use std::num::NonZeroU64;

fn key(tag: u8, suffix: &[u8]) -> Vec<u8> {
    let mut k = b"\0kv9\0retention_v1\0".to_vec();
    k.push(tag);
    k.extend_from_slice(suffix);
    k
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn original_generation_entry_survives_lost_rpc_receipt_and_wal_reopen_without_certifying_legacy_held(
) {
    let mut f = Fixture::new().await;
    let backend = f.backends[1].clone();
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    let (root, intent, legacy, old_managed, fresh) = tokio::task::spawn_blocking(move || {
        let root = backend.ttl_context().unwrap().root.clone();
        let plan = RemoteUploader::plan(
            backend
                .node
                .meta_raft
                .store
                .engine()
                .freeze(FlushScope {
                    cluster: root.cluster_id.to_string(),
                    region: kv9_common::META_REGION_0.0,
                    conf_ver: 1,
                    version: 1,
                })
                .unwrap(),
        )
        .unwrap();
        let intent = CheckpointIntent::new(&root, plan.manifest(), 0).unwrap();
        let pending = intent.binding(OwnerKind::Pending).unwrap();
        let apply = |r| {
            ledger
                .apply(encode_request(root.digest(), &r).unwrap())
                .unwrap()
        };
        apply(LedgerRequest::Initialize);
        apply(LedgerRequest::Register(pending.resources.clone()));
        apply(LedgerRequest::Acquire(pending.clone()));
        apply(LedgerRequest::Publish(pending.token()));
        let make = |n| {
            let mut lifetime = ledger.local_lifetime().unwrap();
            lifetime.process = [n; 16];
            executor_binding(&intent, lifetime).unwrap()
        };
        let legacy = make(1);
        let old_managed = make(2);
        let fresh = make(3);
        apply(LedgerRequest::Share {
            from: pending.token(),
            to: legacy.clone(),
        });
        apply(LedgerRequest::RecordCheckpoint(intent.clone()));
        apply(LedgerRequest::Share {
            from: pending.token(),
            to: old_managed.clone(),
        });
        // Fixed V4 fixture, committed to all replicas. V1 and V2 owner formats
        // are both present, and neither has per-generation provenance.
        let view = backend.node.meta_raft.store.engine().snapshot().unwrap();
        let mut batch = kv9_engine::WriteBatch::new();
        for row in view
            .iter(ColumnFamily::Default, &key(16, &[]), &key(20, &[]))
            .unwrap()
        {
            batch.delete(ColumnFamily::Default, row.unwrap().0);
        }
        // A stopped pre-V20 fixture has no image Publisher index/count rows.
        for row in view
            .iter(ColumnFamily::Default, &key(42, &[]), &key(48, &[]))
            .unwrap()
        {
            batch.delete(ColumnFamily::Default, row.unwrap().0);
        }
        let mut header = view
            .get(ColumnFamily::Default, &key(1, &[]))
            .unwrap()
            .unwrap();
        header.truncate(header.len() - 32);
        header[..8].copy_from_slice(b"KV9LED04");
        header.extend_from_slice(RootDigest::sha256(&header).as_bytes());
        batch.put(ColumnFamily::Default, key(1, &[]), header);
        drop(view);
        let at = backend
            .driver
            .propose(&kv9_raft::Command::from_batch(&batch))
            .unwrap();
        backend
            .driver
            .wait_applied(at, Duration::from_secs(3))
            .unwrap();
        for old in [&legacy, &old_managed] {
            for r in [
                LedgerRequest::Share {
                    from: pending.token(),
                    to: old.clone(),
                },
                LedgerRequest::Publish(old.token()),
            ] {
                assert!(ledger
                    .apply(encode_request(root.digest(), &r).unwrap())
                    .is_err());
            }
            assert_eq!(
                observe(&ledger, root.digest(), old.descriptor.id).phase,
                PinPhase::Held
            );
        }
        (root, intent, legacy, old_managed, fresh)
    })
    .await
    .unwrap();

    f.hooks[1].retention_commits.store(0, Ordering::SeqCst);
    f.hooks[1].retention_fail_after.store(1, Ordering::SeqCst);
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    let r = root.clone();
    let i = intent.clone();
    let e = fresh.clone();
    tokio::task::spawn_blocking(move || {
        assert!(ledger
            .apply(
                encode_request(
                    r.digest(),
                    &LedgerRequest::Share {
                        from: i.binding(OwnerKind::Pending).unwrap().token(),
                        to: e
                    }
                )
                .unwrap()
            )
            .is_err());
    })
    .await
    .unwrap();
    assert_eq!(f.hooks[1].retention_commits.load(Ordering::SeqCst), 1);
    f.hooks[1].retention_fail_after.store(0, Ordering::SeqCst);

    let view = f.backends[1]
        .node
        .meta_raft
        .store
        .engine()
        .snapshot()
        .unwrap();
    assert_eq!(
        history(
            view.as_ref(),
            &root,
            intent.pending_id(),
            fresh.token(),
            None
        )
        .unwrap()
        .generations[0]
            .state,
        GenerationState::Active(PinPhase::Held)
    );
    let mut origin_key = intent.pending_id().as_bytes().to_vec();
    origin_key.extend_from_slice(fresh.descriptor.id.as_bytes());
    origin_key.extend_from_slice(&1u64.to_be_bytes());
    let origin_key = key(16, &origin_key);
    let original = view
        .get(ColumnFamily::Default, &origin_key)
        .unwrap()
        .unwrap();
    drop(view);
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    let r = root.clone();
    let i = intent.clone();
    let e = fresh.clone();
    let old = [legacy.clone(), old_managed.clone()];
    tokio::task::spawn_blocking(move || {
        let apply = |r| ledger.apply(encode_request(i.root(), &r).unwrap());
        let pending = i.binding(OwnerKind::Pending).unwrap().token();
        apply(LedgerRequest::Share {
            from: pending,
            to: e.clone(),
        })
        .unwrap();
        apply(LedgerRequest::Publish(e.token())).unwrap();
        apply(LedgerRequest::FinishCheckpointExecution {
            pending,
            executor: e.clone(),
            issued: 2,
            resolved: 1,
        })
        .unwrap();
        apply(LedgerRequest::CloseCheckpointAdmission(pending)).unwrap();
        for old in old {
            assert!(apply(LedgerRequest::RetireUnpublishedCheckpoint {
                pending,
                executor: old.token()
            })
            .is_err());
            assert_eq!(
                observe(&ledger, r.digest(), old.descriptor.id).phase,
                PinPhase::Held
            );
        }
        let mut later = e;
        later.generation = NonZeroU64::new(2).unwrap();
        apply(LedgerRequest::CancelUnenteredCheckpoint(later)).unwrap();
    })
    .await
    .unwrap();

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
    let (engine, _) = WalEngine::open(f.path.join("n2/meta.wal")).unwrap();
    let view = engine.snapshot().unwrap();
    assert_eq!(
        view.get(ColumnFamily::Default, &origin_key)
            .unwrap()
            .unwrap(),
        original
    );
    for old in [legacy, old_managed] {
        assert_eq!(
            history(view.as_ref(), &root, intent.pending_id(), old.token(), None)
                .unwrap()
                .generations[0]
                .state,
            GenerationState::MissingOrigin
        );
        assert_eq!(
            kv9_meta::retention::retention_owner(view.as_ref(), &root, old.descriptor.id)
                .unwrap()
                .unwrap()
                .phase,
            PinPhase::Held
        );
    }
    let mut through = fresh.token();
    through.generation = NonZeroU64::new(2).unwrap();
    let page = history(view.as_ref(), &root, intent.pending_id(), through, None).unwrap();
    assert!(page.admission.is_closed());
    assert!(page.next.is_none());
    assert_eq!(
        page.generations
            .into_iter()
            .map(|o| o.state)
            .collect::<Vec<_>>(),
        vec![
            GenerationState::Drained {
                issued: 2,
                resolved: 1
            },
            GenerationState::Cancelled
        ]
    );
    assert_eq!(
        kv9_meta::retention::retention_owner(view.as_ref(), &root, intent.pending_id())
            .unwrap()
            .unwrap()
            .phase,
        PinPhase::Published
    );
    drop(view);
    drop(engine);
    f.close().await;
}
