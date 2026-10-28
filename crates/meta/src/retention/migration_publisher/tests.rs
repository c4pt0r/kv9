use super::*;
use crate::retention::tests::{all, apply, fixture as ledger_fixture, owner, plan};
use kv9_common::{
    data_range::DataRange, KeyspaceId, NodeId, RegionId, StoreIdentity, StoreIncarnation, TenantId,
};
use kv9_engine::checkpoint::RemoteUploader;
use kv9_engine::{Engine, MemEngine, MemoryObjectStore, WalEngine};
use kv9_raft::driver::NodeDriver;
use kv9_raft::snapshot_install::capture::{plan_capture_durable, CaptureRequest};
use kv9_raft::storage::DiskRaftStorage;
use kv9_raft::{Command, MemStateMachine, RaftGroup, RaftPeer, Role};
use std::{sync::Arc, time::Duration};

mod acknowledgement;

fn descriptor(root: &RootDescriptor) -> CaptureDescriptor {
    let path = std::env::temp_dir().join(format!(
        "kv9-publisher-{}",
        StoreIncarnation::mint().unwrap()
    ));
    std::fs::create_dir_all(&path).unwrap();
    let descriptor = {
        let storage = DiskRaftStorage::open(&path.join("raft"), &[1]).unwrap().0;
        let engine = Arc::new(WalEngine::open(path.join("data.wal")).unwrap().0);
        let range = DataRange {
            root: root.digest(),
            creation: RootDigest::sha256(b"publisher-creation"),
            region: RegionId(100),
            keyspace: KeyspaceId(7),
            tenant: TenantId(1),
            conf_ver: 1,
            version: 1,
            start: vec![],
            end: vec![],
            sealed: false,
        };
        let mut sm = MemStateMachine::with_engine(engine.clone()).unwrap();
        sm.set_data_group(range.root, range.region, range.creation)
            .unwrap();
        let peer = Arc::new(RaftPeer::with_storage(NodeId(1), range.region, storage).unwrap());
        let hub = kv9_raft::transport::InProcHub::new();
        let driver = NodeDriver::new(peer, Arc::new(hub.endpoint(NodeId(1))), sm).unwrap();
        driver.peer().campaign().unwrap();
        for _ in 0..100 {
            driver.tick_and_step().unwrap();
        }
        assert_eq!(driver.status().role, Role::Leader);
        let apply = |command: &Command| {
            let at = driver.propose(command).unwrap();
            for _ in 0..200 {
                driver.tick_and_step().unwrap();
            }
            driver.wait_applied(at, Duration::from_secs(2)).unwrap();
        };
        apply(&Command::DataRange {
            expected: None,
            next: range.clone(),
        });
        let at = driver.add_learner(NodeId(2)).unwrap();
        for _ in 0..200 {
            driver.tick_and_step().unwrap();
        }
        driver
            .wait_conf_applied(at, Duration::from_secs(2))
            .unwrap();
        apply(&Command::Noop);
        let source = StoreIdentity::for_voter(root, NodeId(1)).unwrap();
        let request = CaptureRequest {
            source,
            destination: StoreIdentity {
                node_id: NodeId(2),
                store_incarnation: StoreIncarnation::from_bytes([4; 16]),
                ..source
            },
            operation: [5; 16],
            range,
        };
        let uploader = RemoteUploader::for_harness(Arc::new(MemoryObjectStore::new()));
        let planned =
            plan_capture_durable(&driver, &engine, &request, &uploader, true, |_, _, _| {
                Ok(false)
            })
            .unwrap();
        let descriptor = planned.descriptor().unwrap();
        drop(planned);
        driver.stop();
        descriptor
    };
    std::fs::remove_dir_all(path).unwrap();
    descriptor
}
fn catalog_key(operation: [u8; 16]) -> Vec<u8> {
    let mut key = b"\0kv9\0migration-capture-v1\0".to_vec();
    key.extend(operation);
    key
}
fn fixture(published: u8) -> (MemEngine, RootDescriptor, CaptureDescriptor, OwnerBinding) {
    let (engine, root) = ledger_fixture();
    let descriptor = descriptor(&root);
    // Canonical applied catalog fixture; source observation/registration itself
    // is exercised by the actual runtime integration, not fabricated as a token.
    let mut bytes = b"KV9MCP01".to_vec();
    bytes.extend(0u64.to_be_bytes());
    bytes.extend(descriptor.encode());
    bytes.extend(*RootDigest::sha256(&bytes).as_bytes());
    let mut batch = WriteBatch::new();
    batch.put(
        ColumnFamily::Default,
        catalog_key(descriptor.request().operation),
        bytes,
    );
    engine.write(batch).unwrap();
    apply(&engine, &root, LedgerRequest::Initialize);
    let (source, destination) = image_owners(&descriptor).unwrap();
    apply(
        &engine,
        &root,
        LedgerRequest::Register(source.resources.clone()),
    );
    apply(&engine, &root, LedgerRequest::Acquire(source.clone()));
    if published > 0 {
        apply(&engine, &root, LedgerRequest::Publish(source.token()));
        apply(
            &engine,
            &root,
            LedgerRequest::Share {
                from: source.token(),
                to: destination.clone(),
            },
        );
    }
    if published > 1 {
        apply(&engine, &root, LedgerRequest::Publish(destination.token()));
    }
    let binding = binding(
        &descriptor,
        LocalLifetime {
            node: 1,
            store: [3; 16],
            process: [7; 16],
        },
    )
    .unwrap();
    (engine, root, descriptor, binding)
}
fn enter(d: &CaptureDescriptor, b: &OwnerBinding) -> LedgerRequest {
    LedgerRequest::EnterMigrationPublisher {
        operation: d.request().operation,
        publisher: b.clone(),
    }
}
fn finish(d: &CaptureDescriptor, b: &OwnerBinding, issued: u64, resolved: u64) -> LedgerRequest {
    LedgerRequest::FinishMigrationPublisher {
        operation: d.request().operation,
        publisher: b.clone(),
        issued,
        resolved,
    }
}
fn fact(e: &MemEngine, r: &RootDescriptor, b: &OwnerBinding) -> PublisherGeneration {
    generation(e.snapshot().unwrap().as_ref(), r, b.token())
        .unwrap()
        .unwrap()
}

#[test]
fn publisher_requires_exact_catalog_published_pair_and_atomic_entry_before_io() {
    for published in 0..2 {
        let (e, r, d, b) = fixture(published);
        assert!(plan(&e, &r, &enter(&d, &b)).is_err());
    }
    let (e, r, d, b) = fixture(2);
    let before = all(&e);
    for case in 0..7 {
        let mut wrong = b.clone();
        match case {
            0 => wrong.descriptor.subject[0] ^= 1,
            1 => wrong.descriptor.operation[0] ^= 1,
            2 => wrong.descriptor.local.as_mut().unwrap().process[0] ^= 1,
            3 => wrong.descriptor.kind = OwnerKind::Build,
            4 => wrong.generation = NonZeroU64::new(2).unwrap(),
            5 => wrong.descriptor.region += 1,
            _ => wrong.resources.pop().map(|_| ()).unwrap(),
        }
        assert!(plan(&e, &r, &enter(&d, &wrong)).is_err(), "case {case}");
    }
    assert!(plan(&e, &r, &LedgerRequest::Acquire(b.clone())).is_err());
    assert!(plan(
        &e,
        &r,
        &LedgerRequest::Share {
            from: image_owners(&d).unwrap().0.token(),
            to: b.clone()
        }
    )
    .is_err());
    assert_eq!(all(&e), before);
    apply(&e, &r, enter(&d, &b));
    assert!(!plan(&e, &r, &enter(&d, &b)).unwrap().changed());
    assert_eq!(owner(&e, &r, b.descriptor.id).phase, PinPhase::Published);
    assert!(fact(&e, &r, &b).admitted_at > 0);
    assert!(plan(&e, &r, &LedgerRequest::Publish(b.token())).is_err());
    assert!(plan(&e, &r, &LedgerRequest::Release(b.token())).is_err());
    assert!(plan(
        &e,
        &r,
        &LedgerRequest::QuiesceAfterTransfer {
            from: b.token(),
            to: image_owners(&d).unwrap().0.token()
        }
    )
    .is_err());
}

#[test]
fn original_unknown_puts_survive_later_generations_and_old_receipt_retries() {
    let (e, r, d, mut b) = fixture(2);
    apply(&e, &r, enter(&d, &b));
    apply(&e, &r, finish(&d, &b, 3, 1));
    let first = b.clone();
    let original = fact(&e, &r, &b);
    assert_eq!((original.issued, original.resolved), (3, 1));
    assert!(original.finished_at > original.admitted_at);
    let stale = finish(&d, &b, 3, 1);
    assert!(!plan(&e, &r, &stale).unwrap().changed());
    assert!(plan(&e, &r, &finish(&d, &b, 3, 3)).is_err());
    assert!(plan(&e, &r, &enter(&d, &b)).is_err());
    b.generation = NonZeroU64::new(2).unwrap();
    apply(&e, &r, enter(&d, &b));
    assert!(!plan(&e, &r, &stale).unwrap().changed());
    assert_eq!(owner(&e, &r, b.descriptor.id).phase, PinPhase::Published);
    apply(&e, &r, finish(&d, &b, 0, 0));
    assert_eq!(fact(&e, &r, &first), original);
    assert_eq!(fact(&e, &r, &b).issued, 0);
    for pin in [image_owners(&d).unwrap().0, image_owners(&d).unwrap().1] {
        assert_eq!(owner(&e, &r, pin.descriptor.id).phase, PinPhase::Published);
    }
}

#[test]
fn unconfirmed_entry_is_fenced_without_fabricating_an_entered_or_nonzero_report() {
    let (e, r, d, mut b) = fixture(2);
    assert!(plan(&e, &r, &finish(&d, &b, 1, 0)).is_err());
    apply(&e, &r, finish(&d, &b, 0, 0));
    let cancelled = fact(&e, &r, &b);
    assert_eq!(cancelled.admitted_at, 0);
    assert!(cancelled.finished_at > 0);
    assert!(plan(&e, &r, &enter(&d, &b)).is_err());
    b.generation = NonZeroU64::new(2).unwrap();
    apply(&e, &r, enter(&d, &b));
    apply(&e, &r, finish(&d, &b, 0, 0));
    assert!(fact(&e, &r, &b).admitted_at > 0);
    b.generation = NonZeroU64::new(3).unwrap();
    apply(&e, &r, finish(&d, &b, 0, 0));
    assert!(plan(&e, &r, &enter(&d, &b)).is_err());
    assert_eq!(fact(&e, &r, &b).admitted_at, 0);
}

#[test]
fn publisher_codec_header_upgrade_and_missing_or_damaged_records_refuse() {
    let (e, r, d, b) = fixture(2);
    for request in [enter(&d, &b), finish(&d, &b, 4, 2)] {
        let bytes = encode_request(r.digest(), &request).unwrap();
        assert_eq!(decode_request(&bytes, r.digest()).unwrap(), request);
        for cut in 0..bytes.len() {
            assert!(decode_request(&bytes[..cut], r.digest()).is_err());
        }
    }
    let mut h = e.get(ColumnFamily::Default, &key(1, &[])).unwrap().unwrap();
    h[..8].copy_from_slice(CLOSURE_HEADER_MAGIC);
    h.truncate(h.len() - 32);
    let h = seal(h);
    let mut batch = WriteBatch::new();
    batch.put(ColumnFamily::Default, key(1, &[]), h);
    e.write(batch).unwrap();
    apply(&e, &r, enter(&d, &b));
    let h = e.get(ColumnFamily::Default, &key(1, &[])).unwrap().unwrap();
    assert!(checked(&h, CLOSURE_HEADER_MAGIC, 81).is_err());
    assert!(h.starts_with(HEADER_MAGIC));
    let record = e
        .get(ColumnFamily::Default, &record_key(b.token()))
        .unwrap()
        .unwrap();
    for case in 0..3 {
        let mut batch = WriteBatch::new();
        match case {
            0 => {
                batch.delete(ColumnFamily::Default, record_key(b.token()));
            }
            1 => {
                let mut bad = record.clone();
                bad[80] ^= 1;
                batch.put(ColumnFamily::Default, record_key(b.token()), bad);
            }
            _ => {
                let mut old = h.clone();
                old[..8].copy_from_slice(CLOSURE_HEADER_MAGIC);
                old.truncate(old.len() - 32);
                batch.put(ColumnFamily::Default, key(1, &[]), seal(old));
            }
        }
        e.write(batch).unwrap();
        assert!(retention_owner(e.snapshot().unwrap().as_ref(), &r, b.descriptor.id).is_err());
        assert!(plan(&e, &r, &finish(&d, &b, 0, 0)).is_err());
        if case == 2 {
            assert!(plan(&e, &r, &LedgerRequest::Initialize).is_err());
        }
        let mut batch = WriteBatch::new();
        batch.put(ColumnFamily::Default, record_key(b.token()), record.clone());
        batch.put(ColumnFamily::Default, key(1, &[]), h.clone());
        e.write(batch).unwrap();
    }
}

#[test]
fn actual_wal_tail_cuts_keep_entry_and_drain_atomic_with_every_resource_pin() {
    for finishing in [false, true] {
        let (memory, r, d, b) = fixture(2);
        if finishing {
            apply(&memory, &r, enter(&d, &b));
        }
        let request = if finishing {
            finish(&d, &b, 2, 1)
        } else {
            enter(&d, &b)
        };
        let dir = std::env::temp_dir().join(format!(
            "kv9-publisher-wal-{}",
            StoreIncarnation::mint().unwrap()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("meta.wal");
        let (e, _) = WalEngine::open(&path).unwrap();
        let (_, mut seed) = crate::retention::tests::seed();
        for (k, v) in all(&memory) {
            seed.put(ColumnFamily::Default, k, v);
        }
        let catalog = catalog_key(d.request().operation);
        seed.put(
            ColumnFamily::Default,
            catalog.clone(),
            memory
                .get(ColumnFamily::Default, &catalog)
                .unwrap()
                .unwrap(),
        );
        e.write(seed).unwrap();
        let before = std::fs::read(&path).unwrap();
        e.write(plan(&e, &r, &request).unwrap().into_batch())
            .unwrap();
        drop(e);
        let after = std::fs::read(&path).unwrap();
        for cut in [
            before.len(),
            before.len() + 1,
            before.len() + (after.len() - before.len()) / 2,
            after.len() - 1,
            after.len(),
        ] {
            let copy = dir.join(format!("cut-{cut}.wal"));
            std::fs::write(&copy, &after[..cut]).unwrap();
            let (e, _) = WalEngine::open(&copy).unwrap();
            let observed =
                retention_owner(e.snapshot().unwrap().as_ref(), &r, b.descriptor.id).unwrap();
            if finishing {
                let record = generation(e.snapshot().unwrap().as_ref(), &r, b.token())
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    observed.unwrap().phase,
                    if cut == after.len() {
                        PinPhase::Released
                    } else {
                        PinPhase::Published
                    }
                );
                assert_eq!(
                    (record.issued, record.resolved),
                    if cut == after.len() { (2, 1) } else { (0, 0) }
                );
                assert_eq!(record.finished_at != 0, cut == after.len());
            } else if cut == after.len() {
                assert_eq!(observed.unwrap().phase, PinPhase::Published);
                assert!(
                    generation(e.snapshot().unwrap().as_ref(), &r, b.token())
                        .unwrap()
                        .unwrap()
                        .admitted_at
                        > 0
                );
            } else {
                assert!(observed.is_none());
            }
            for pin in [image_owners(&d).unwrap().0, image_owners(&d).unwrap().1] {
                assert_eq!(owner(&e, &r, pin.descriptor.id).phase, PinPhase::Published);
            }
            e.write(plan(&e, &r, &request).unwrap().into_batch())
                .unwrap();
            assert!(!plan(&e, &r, &request).unwrap().changed());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn reader_index_upgrade_preserves_v8_publisher_generations_and_unknown_puts() {
    let (e, r, d, b) = fixture(2);
    apply(&e, &r, enter(&d, &b));
    apply(&e, &r, finish(&d, &b, 4, 2));
    let original = fact(&e, &r, &b);
    let bytes = e
        .get(ColumnFamily::Default, &record_key(b.token()))
        .unwrap()
        .unwrap();
    // This fixture contains no Reader indexes: every row has V8 semantics.
    let mut h = e.get(ColumnFamily::Default, &key(1, &[])).unwrap().unwrap();
    h.truncate(h.len() - 32);
    h[..8].copy_from_slice(MIGRATION_HEADER_MAGIC);
    let mut batch = WriteBatch::new();
    batch.put(ColumnFamily::Default, key(1, &[]), seal(h));
    // A stopped older-ledger fixture has no V20 image Publisher records.
    for (k, _) in all(&e) {
        if k >= key(42, &[]) && k < key(48, &[]) {
            batch.delete(ColumnFamily::Default, k);
        }
    }
    e.write(batch).unwrap();
    assert_eq!(fact(&e, &r, &b), original);
    assert!(matches!(
        reader_index::observe(e.snapshot().unwrap().as_ref(), &r).unwrap(),
        reader_index::Work::Upgrade
    ));
    apply(&e, &r, LedgerRequest::Initialize);
    loop {
        match reader_index::observe(e.snapshot().unwrap().as_ref(), &r).unwrap() {
            reader_index::Work::Page { progress, next, .. } => {
                apply(&e, &r, progress.request(next));
            }
            reader_index::Work::Complete => break,
            _ => panic!("expected upgraded ledger"),
        }
    }
    assert_eq!(fact(&e, &r, &b), original);
    assert_eq!(
        e.get(ColumnFamily::Default, &record_key(b.token()))
            .unwrap()
            .unwrap(),
        bytes
    );
    assert!(!plan(&e, &r, &finish(&d, &b, 4, 2)).unwrap().changed());
    assert!(plan(&e, &r, &finish(&d, &b, 4, 4)).is_err());
}

#[test]
fn typed_migration_publishers_are_indexed_by_exact_region_image_across_generations() {
    let (e, r, d, mut b) = fixture(2);
    apply(&e, &r, enter(&d, &b));
    loop {
        match image_publisher_index::observe(e.snapshot().unwrap().as_ref(), &r).unwrap() {
            image_publisher_index::Work::Page { progress, next, .. } => {
                apply(&e, &r, progress.request(next));
            }
            image_publisher_index::Work::Complete => break,
            _ => panic!("expected current Publisher catalog"),
        }
    }
    apply(&e, &r, finish(&d, &b, 2, 0));
    b.generation = NonZeroU64::new(2).unwrap();
    apply(&e, &r, enter(&d, &b));
    let view = e.snapshot().unwrap();
    let page = image_publisher_index::publishers(
        view.as_ref(),
        &r,
        b.descriptor.region,
        b.descriptor.subject,
        None,
    )
    .unwrap();
    assert_eq!(page.owners.len(), 1);
    assert_eq!(page.owners[0].binding, b);
    assert_eq!(page.owners[0].phase, PinPhase::Published);
    assert!(page.next.is_none());
    assert!(image_publisher_index::publishers(
        view.as_ref(),
        &r,
        kv9_common::META_REGION_0.0,
        b.descriptor.subject,
        None
    )
    .unwrap()
    .owners
    .is_empty());
    let first = generation(
        view.as_ref(),
        &r,
        OwnerToken {
            owner: b.descriptor.id,
            generation: NonZeroU64::new(1).unwrap(),
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!((first.issued, first.resolved), (2, 0));
}
