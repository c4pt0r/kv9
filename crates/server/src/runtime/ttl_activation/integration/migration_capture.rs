//! Actual runtime planning/binding/capture, followed by an offline learner open.
use super::*;
use kv9_common::codec::{encode_key, KeyMode};
use kv9_common::retention::PinPhase;
use kv9_engine::checkpoint::{CheckpointManifest, RemoteUploader};
use kv9_engine::{MemoryObjectStore, ObjectKey, ObjectStore};
use std::sync::atomic::AtomicBool;
mod configuration_origins;
mod installation;
mod publisher;

async fn leader(driver: &Driver) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while driver.status().role != Role::Leader {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn replicated_capture_survives_source_loss_metadata_failover_and_lost_commit_receipt() {
    let f = Fixture::new().await;
    let root = f.backends[0].ttl_context().unwrap().root.clone();
    let digest = root.digest();
    let operation = [93; 16];
    assert!(matches!(
        f.client(0).activate().await.outcome,
        TtlOutcome::Success { .. }
    ));
    assert!(matches!(
        f.client(0)
            .call(Operation::Put {
                pairs: vec![(b"original".to_vec(), b"captured value".to_vec())],
                ttl_secs: Some(60),
            })
            .await
            .outcome,
        TtlOutcome::Success { .. }
    ));
    let (identity, guard, _) = prepare_migration(&f, &root, operation).await;
    let objects = Arc::new(Objects::default());
    let uploader = Arc::new(RemoteUploader::for_harness(objects.clone()));
    let source = with_uploader(&f.backends[0], uploader.clone());
    *f.hooks[0].capture_backend.lock().unwrap() = Some(source.clone());
    *f.hooks[0].capture_change_endpoint.lock().unwrap() = Some(f.backends[1].clone());
    let backend = source.clone();
    assert!(tokio::task::spawn_blocking(
        move || backend.plan_migration_image("test", digest, operation)
    )
    .await
    .unwrap()
    .is_err());
    assert!(kv9_meta::data_groups::capture::committed_capture(
        f.backends[1]
            .node
            .meta_raft
            .store
            .begin()
            .unwrap()
            .into_view()
            .as_ref(),
        &root,
        operation
    )
    .unwrap()
    .is_none());
    assert_eq!(objects.puts.load(Ordering::SeqCst), 0);
    // A durable file without the independently committed registration is not
    // enough to bind owners, even when its manifest is otherwise valid.
    let local = kv9_raft::snapshot_install::capture::observe_durable_capture(
        &f.data[0],
        f.backends[0]
            .raw_directory
            .by_region(f.range.region)
            .unwrap()
            .capture_parts()
            .1,
        source.ttl_context().unwrap().local.identity(),
        operation,
        &uploader,
    )
    .unwrap()
    .unwrap();
    assert!(kv9_meta::data_groups::capture::plan_record(
        f.backends[1].node.meta_raft.store.begin().unwrap(),
        &local,
        None,
    )
    .unwrap_err()
    .to_string()
    .contains("independent source observation"));
    let metadata = f.backends[1].clone();
    let manifest = local.manifest().encode().unwrap();
    assert!(tokio::task::spawn_blocking(move || {
        metadata.bind_migration_image("test", digest, operation, &manifest)
    })
    .await
    .unwrap()
    .unwrap_err()
    .to_string()
    .contains("committed capture plan"));
    // The next attempt commits the exact original descriptor, then loses its
    // actual metadata receipt. No second identity is minted on retry.
    let commits = f.hooks[1].retention_commits.load(Ordering::SeqCst);
    f.hooks[1]
        .retention_fail_after
        .store(commits + 1, Ordering::SeqCst);
    let backend = source.clone();
    assert!(tokio::task::spawn_blocking(
        move || backend.plan_migration_image("test", digest, operation)
    )
    .await
    .unwrap()
    .is_err());
    let fixed = kv9_meta::data_groups::capture::committed_capture(
        f.backends[1]
            .node
            .meta_raft
            .store
            .begin()
            .unwrap()
            .into_view()
            .as_ref(),
        &root,
        operation,
    )
    .unwrap()
    .unwrap();
    f.hooks[1].retention_fail_after.store(0, Ordering::SeqCst);
    f.hooks[0].capture_refuse.store(true, Ordering::SeqCst);
    let reads = f.hooks[0].capture_reads.load(Ordering::SeqCst);
    assert_eq!(reads, 2);
    let bytes = fixed.descriptor().encode().to_vec();
    let metadata = f.backends[2].clone();
    tokio::task::spawn_blocking(move || {
        crate::runtime::checkpoint_retention::RoutedLedger::record_capture(&metadata, bytes)
    })
    .await
    .unwrap()
    .unwrap();
    let barrier = f.meta[1].propose(&Command::Noop).unwrap();
    applied(&f.meta, barrier).await;
    // Catalog registration alone is insufficient for Plan success. Both
    // other voters lack an uploader/copy backend until enabled below.
    let backend = source.clone();
    let refusal = tokio::task::spawn_blocking(move || {
        backend.plan_migration_image("test", digest, operation)
    })
    .await
    .unwrap()
    .unwrap_err();
    assert!(refusal.to_string().contains("voter quorums"));
    assert_eq!(objects.puts.load(Ordering::SeqCst), 0);
    assert!(!f.path.join("n2/capture-replica.plan").exists());
    assert!(!f.path.join("n3/capture-replica.plan").exists());
    enable_payload_replicas(&f, &objects);
    let backend = source.clone();
    let plan = tokio::task::spawn_blocking(move || {
        backend.plan_migration_image("test", digest, operation)
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        plan.manifest,
        fixed.descriptor().manifest().encode().unwrap()
    );
    assert_eq!(f.hooks[0].capture_reads.load(Ordering::SeqCst), reads);
    // The generic ledger API must enforce the catalog binding as well. A
    // different owner kind cannot bypass the check for the same reserved ID.
    use kv9_meta::retention::{
        encode_request, LedgerRequest, OwnerBinding, OwnerDescriptor, OwnerKind,
    };
    let (source_owner, _) = crate::migration_retention::migration_owner_ids(
        digest,
        operation,
        &identity.store_incarnation,
    )
    .unwrap();
    for kind in [OwnerKind::Snapshot, OwnerKind::Backup] {
        let binding = OwnerBinding {
            descriptor: OwnerDescriptor {
                root: digest,
                id: source_owner,
                kind,
                region: f.range.region.0,
                conf_ver: f.range.conf_ver,
                version: f.range.version,
                operation: kv9_meta::data_groups::evidence::migration_operation_digest(
                    digest, operation,
                ),
                subject: *RootDigest::sha256(b"different image").as_bytes(),
                subject_is_anchor: false,
                local: None,
            },
            generation: std::num::NonZeroU64::new(1).unwrap(),
            resources: crate::checkpoint_retention::manifest_resources(digest, local.manifest())
                .unwrap(),
        };
        let request = encode_request(digest, &LedgerRequest::Acquire(binding)).unwrap();
        let metadata = f.backends[1].clone();
        let error = tokio::task::spawn_blocking(move || metadata.apply_retention("test", request))
            .await
            .unwrap()
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("owner differs from committed capture plan"),
            "{error}"
        );
    }
    let metadata = f.backends[1].clone();
    let manifest = plan.manifest.clone();
    let bindings = tokio::task::spawn_blocking(move || {
        metadata.bind_migration_image("test", digest, operation, &manifest)
    })
    .await
    .unwrap()
    .unwrap();
    let metadata = f.backends[1].clone();
    let manifest = plan.manifest.clone();
    let again = tokio::task::spawn_blocking(move || {
        metadata.bind_migration_image("test", digest, operation, &manifest)
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(again, bindings);
    let mut changed = CheckpointManifest::decode(&plan.manifest).unwrap();
    changed.index += 1;
    let metadata = f.backends[1].clone();
    assert!(tokio::task::spawn_blocking(move || {
        metadata.bind_migration_image("test", digest, operation, &changed.encode().unwrap())
    })
    .await
    .unwrap()
    .is_err());
    let view = f.backends[1]
        .node
        .meta_raft
        .store
        .begin()
        .unwrap()
        .into_view();
    let owner = |id| {
        kv9_meta::retention::retention_owner(view.as_ref(), &root, id)
            .unwrap()
            .unwrap()
    };
    let source_pin = owner(bindings.source_owner);
    let destination_pin = owner(bindings.destination_owner);
    assert_eq!(source_pin.phase, PinPhase::Published);
    assert_eq!(destination_pin.phase, PinPhase::Published);
    assert_eq!(source_pin.binding.descriptor.kind, OwnerKind::Snapshot);
    assert_eq!(
        destination_pin.binding.descriptor.kind,
        OwnerKind::Migration
    );
    assert_eq!(
        source_pin.binding.descriptor.subject,
        *RootDigest::sha256(&plan.manifest).as_bytes()
    );
    assert_eq!(
        source_pin.binding.descriptor.subject,
        destination_pin.binding.descriptor.subject
    );
    assert_eq!(
        source_pin.binding.resources,
        destination_pin.binding.resources
    );
    for request in [
        LedgerRequest::QuiesceAfterTransfer {
            from: source_pin.binding.token(),
            to: destination_pin.binding.token(),
        },
        LedgerRequest::Release(source_pin.binding.token()),
    ] {
        let encoded = encode_request(digest, &request).unwrap();
        let metadata = f.backends[1].clone();
        assert!(
            tokio::task::spawn_blocking(move || metadata.apply_retention("test", encoded))
                .await
                .unwrap()
                .is_err()
        );
    }
    drop(view);
    let barrier = f.meta[1].propose(&Command::Noop).unwrap();
    applied(&f.meta, barrier).await;
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    *objects.gate.lock().unwrap() = Some((entered_tx, release_rx));
    let backend = source.clone();
    let upload = tokio::task::spawn_blocking(move || {
        backend.capture_migration_image("test", digest, operation)
    });
    tokio::task::spawn_blocking(move || entered_rx.recv_timeout(Duration::from_secs(5)).unwrap())
        .await
        .unwrap();
    assert_eq!(
        publisher::capture_fact(&f, &root, fixed.descriptor(), 0, 1).finished_at,
        0
    );
    f.data[0].peer().transfer_leader_for_tests(NodeId(3));
    leader(&f.data[2]).await;
    f.data[0].stop();
    f.backends[0].raw_directory.remove(f.range.region);
    let successor = with_uploader(
        &f.backends[2],
        Arc::new(RemoteUploader::for_harness(objects.clone())),
    );
    let backend = successor.clone();
    let takeover = tokio::task::spawn_blocking(move || {
        backend.capture_migration_image("test", digest, operation)
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(takeover.record, fixed.descriptor().image().record);
    assert_eq!(takeover.cut, plan.cut);
    assert!(
        objects.puts.load(Ordering::SeqCst) > 1,
        "successor completes original objects while the original PUT is still unresolved"
    );
    assert!(f.path.join("n2/capture-replica.plan").is_file());
    assert!(f.path.join("n3/capture-replica.plan").is_file());
    let takeover_publisher = publisher::capture_fact(&f, &root, fixed.descriptor(), 2, 1);
    assert!(takeover_publisher.finished_at > takeover_publisher.admitted_at);
    assert!(takeover_publisher.issued > 0);
    assert_eq!(takeover_publisher.resolved, 0);
    assert_eq!(
        publisher::capture_fact(&f, &root, fixed.descriptor(), 0, 1).finished_at,
        0
    );
    release_tx.send(()).unwrap();
    let original = upload.await.unwrap().unwrap();
    let original_publisher = publisher::capture_fact(&f, &root, fixed.descriptor(), 0, 1);
    assert!(original_publisher.finished_at > original_publisher.admitted_at);
    assert!(original_publisher.issued > 0);
    f.hooks[0].capture_backend.lock().unwrap().take();
    drop(source);
    f.meta[1].peer().transfer_leader_for_tests(NodeId(3));
    leader(&f.meta[2]).await;
    let puts = objects.puts.load(Ordering::SeqCst);
    let backend = successor.clone();
    let resumed = tokio::task::spawn_blocking(move || {
        backend.capture_migration_image("test", digest, operation)
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(resumed.record, original.record);
    assert_eq!(resumed.cut, plan.cut);
    assert_eq!(
        resumed.image_digest,
        fixed.descriptor().image().image_digest
    );
    assert_eq!(objects.puts.load(Ordering::SeqCst), puts);
    assert_eq!(f.hooks[0].capture_reads.load(Ordering::SeqCst), reads);
    assert_eq!(
        publisher::capture_fact(&f, &root, fixed.descriptor(), 2, 2).issued,
        0
    );
    assert_eq!(
        publisher::capture_fact(&f, &root, fixed.descriptor(), 2, 1),
        takeover_publisher
    );
    // A complete local payload can repair an absent object with the same bytes.
    let missing = objects.memory.list("").unwrap().pop().unwrap();
    let contents = objects.memory.get(&missing).unwrap().unwrap();
    objects.memory.delete(&missing).unwrap();
    let replica_path = f.path.join("n3/capture-replica.plan");
    let replica_bytes = std::fs::read(&replica_path).unwrap();
    let mut corrupt = replica_bytes.clone();
    let middle = corrupt.len() / 2;
    corrupt[middle] ^= 1;
    std::fs::write(&replica_path, corrupt).unwrap();
    let backend = successor.clone();
    assert!(tokio::task::spawn_blocking(move || {
        backend.capture_migration_image("test", digest, operation)
    })
    .await
    .unwrap()
    .is_err());
    assert_eq!(objects.puts.load(Ordering::SeqCst), puts);
    // Loss of every reachable payload cannot authorize a newer replacement cut.
    std::fs::remove_file(&replica_path).unwrap();
    let other_copy = f.hooks[1].capture_backend.lock().unwrap().take();
    let backend = successor.clone();
    assert!(tokio::task::spawn_blocking(move || {
        backend.capture_migration_image("test", digest, operation)
    })
    .await
    .unwrap()
    .is_err());
    assert_eq!(objects.puts.load(Ordering::SeqCst), puts);
    assert!(!replica_path.exists());
    *f.hooks[1].capture_backend.lock().unwrap() = other_copy;
    std::fs::write(&replica_path, &replica_bytes).unwrap();
    let backend = successor.clone();
    let repaired = tokio::task::spawn_blocking(move || {
        backend.capture_migration_image("test", digest, operation)
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(repaired.record, original.record);
    assert_eq!(objects.memory.get(&missing).unwrap().unwrap(), contents);
    assert!(objects.puts.load(Ordering::SeqCst) > puts);
    for owner in [bindings.source_owner, bindings.destination_owner] {
        assert_eq!(
            kv9_meta::retention::retention_owner(
                f.backends[2]
                    .node
                    .meta_raft
                    .store
                    .begin()
                    .unwrap()
                    .into_view()
                    .as_ref(),
                &root,
                owner
            )
            .unwrap()
            .unwrap()
            .phase,
            PinPhase::Published
        );
    }
    kv9_raft::snapshot_install::install_captured_record(
        &guard,
        identity,
        &resumed.record,
        &uploader,
    )
    .unwrap();
    let adopted =
        kv9_raft::snapshot_install::adopt_for_runtime(&guard, identity, f.range.region, &uploader)
            .unwrap();
    let mut storage = DiskRaftStorage::recover(&adopted.raft_directory).unwrap();
    let (restored, _, _) = adopted
        .open_engine_with_dependencies(&mut storage, &uploader)
        .unwrap();
    let key = encode_key(KeyMode::Raw, f.range.keyspace, b"original").unwrap();
    assert_eq!(
        restored.get(ColumnFamily::Default, &key).unwrap(),
        Some(b"captured value".to_vec())
    );
    assert!(restored.get(ColumnFamily::Write, &key).unwrap().is_some());
    drop((restored, storage, adopted, guard, successor));
    let path = f.shutdown_keep().await;
    // V7 and V9 share capture/catalog encodings. At a fully stopped store,
    // construct the predecessor lifecycle header to exercise the real upgrade
    // gate before reopening its durable catalog and original capture payload.
    // This is a format fixture, not execution of a complete older binary.
    let capture_path = path.join("n1/capture.plan");
    let capture_digest = RootDigest::sha256(&std::fs::read(&capture_path).unwrap());
    let mut upgraded = Vec::new();
    for node in [1, 3] {
        let directory = path.join(format!("n{node}"));
        let lifecycle_path = directory.join(kv9_common::store_lifecycle::STORE_LIFECYCLE_FILE);
        let mut bytes = std::fs::read(&lifecycle_path).unwrap();
        bytes[..8].copy_from_slice(b"KV9LIFE7");
        let payload = bytes.len() - 32;
        let digest = RootDigest::sha256(&bytes[..payload]);
        bytes[payload..].copy_from_slice(digest.as_bytes());
        std::fs::write(&lifecycle_path, bytes).unwrap();
        let mut guard = StoreGuard::lock(&directory).unwrap();
        let before = guard.record().unwrap();
        let identity = StoreIdentity::for_voter(&root, NodeId(node)).unwrap();
        assert!(guard.raw_ttl_format_fence(&root, &identity).is_err());
        assert!(guard.require_current_writer_format().is_err());
        guard.fence_legacy_writers().unwrap();
        assert_eq!(
            guard
                .raw_ttl_format_fence(&root, &identity)
                .unwrap()
                .identity(),
            identity
        );
        assert_eq!(guard.record(), Some(before));
        assert_eq!(&std::fs::read(lifecycle_path).unwrap()[..8], b"KV9LIF11");
        upgraded.push(guard);
    }
    assert_eq!(
        RootDigest::sha256(&std::fs::read(&capture_path).unwrap()),
        capture_digest
    );
    let journal = uploader.transfer_journal(capture_path).unwrap();
    let pending = journal.load().unwrap().unwrap();
    assert_eq!(pending.metadata(), fixed.descriptor().encode());
    assert_eq!(pending.manifest(), fixed.descriptor().manifest());
    let (engine, _) = WalEngine::open(path.join("n3/meta.wal")).unwrap();
    let recovered = kv9_meta::data_groups::capture::committed_capture(
        engine.snapshot().unwrap().as_ref(),
        &root,
        operation,
    )
    .unwrap()
    .unwrap();
    assert_eq!(recovered, fixed);
    for expected in [&original_publisher, &takeover_publisher] {
        assert_eq!(
            kv9_meta::retention::migration_publisher::generation(
                engine.snapshot().unwrap().as_ref(),
                &root,
                expected.publisher
            )
            .unwrap()
            .as_ref(),
            Some(expected)
        );
    }
    for owner in [bindings.source_owner, bindings.destination_owner] {
        assert_eq!(
            kv9_meta::retention::retention_owner(engine.snapshot().unwrap().as_ref(), &root, owner)
                .unwrap()
                .unwrap()
                .phase,
            PinPhase::Published
        );
    }
    assert_eq!(
        recovered
            .descriptor()
            .verify_uploaded(&uploader)
            .unwrap()
            .record,
        original.record
    );
    drop(engine);
    drop((pending, journal, upgraded));
    std::fs::remove_dir_all(path).unwrap();
}

#[derive(Debug, Default)]
struct Objects {
    memory: MemoryObjectStore,
    gets: AtomicUsize,
    puts: AtomicUsize,
    refuse: AtomicBool,
    gate: Mutex<Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>>,
}
impl ObjectStore for Objects {
    fn put(&self, key: &ObjectKey, bytes: &[u8]) -> Result<()> {
        self.puts.fetch_add(1, Ordering::SeqCst);
        let gate = self.gate.lock().unwrap().take();
        if let Some((entered, release)) = gate {
            entered.send(()).unwrap();
            release.recv_timeout(Duration::from_secs(30)).unwrap();
        }
        self.memory.put(key, bytes)?;
        if self.refuse.load(Ordering::SeqCst) {
            return Err(Error::Engine("injected migration upload failure".into()));
        }
        Ok(())
    }
    fn get(&self, key: &ObjectKey) -> Result<Option<Vec<u8>>> {
        self.gets.fetch_add(1, Ordering::SeqCst);
        self.memory.get(key)
    }
    fn delete(&self, _: &ObjectKey) -> Result<()> {
        panic!("no GC")
    }
    fn list(&self, prefix: &str) -> Result<Vec<ObjectKey>> {
        self.memory.list(prefix)
    }
}

fn enable_payload_replicas(f: &Fixture, objects: &Arc<Objects>) {
    for i in 1..3 {
        let uploader = RemoteUploader::for_harness(objects.clone())
            .with_image_budget(Arc::new(
                kv9_engine::checkpoint_memory::ImageBudget::new(9 * 1024 * 1024).unwrap(),
            ))
            .with_pending_budget(Arc::new(
                kv9_engine::checkpoint_pending_budget::PendingDiskBudget::new(64 * 1024 * 1024)
                    .unwrap(),
            ));
        *f.hooks[i].capture_backend.lock().unwrap() =
            Some(with_uploader(&f.backends[i], Arc::new(uploader)));
    }
}

fn with_uploader(old: &Arc<RuntimeBackend>, uploader: Arc<RemoteUploader>) -> Arc<RuntimeBackend> {
    Arc::new(RuntimeBackend {
        maintenance: old.maintenance.clone(),
        ttl_activation: old.ttl_activation.clone(),
        raw_directory: old.raw_directory.clone(),
        group_handles: old.group_handles.clone(),
        adoption_receipts: old.adoption_receipts.clone(),
        node: old.node.clone(),
        driver: old.driver.clone(),
        transport: old.transport.clone(),
        endpoint_ready: old.endpoint_ready.clone(),
        initial_voters: old.initial_voters.clone(),
        uploader: Some(uploader),
        status_cache: Arc::default(),
    })
}

async fn applied(drivers: &[Driver], at: kv9_raft::ProposedAt) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while drivers
            .iter()
            .any(|d| d.status().applied_index < at.index.0)
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

async fn prepare_migration(
    f: &Fixture,
    root: &RootDescriptor,
    operation: [u8; 16],
) -> (StoreIdentity, StoreGuard, u64) {
    let digest = root.digest();
    let destination_path = f.path.join("destination");
    let mut guard = StoreGuard::lock(&destination_path).unwrap();
    let prepared = guard.prepare(NodeId(4)).unwrap();
    let identity = StoreIdentity {
        cluster_id: root.cluster_id,
        root_digest: digest,
        node_id: NodeId(4),
        store_incarnation: prepared.incarnation,
    };
    guard.bind(root, &identity).unwrap();
    let creation = kv9_meta::data_groups::committed_creations(&f.backends[1].node.meta_raft.store)
        .unwrap()
        .remove(0);
    let mut txn = f.backends[1].node.meta_raft.store.begin().unwrap();
    // Fixture registration is committed through real metadata Raft. Admission
    // and old-binary qualification are separate deployment tests.
    txn.insert(
        &NODES_DESC,
        &[memcmp_uint(4)],
        membership_node_row(NodeId(4), "127.0.0.1:22004", 2, 0, prepared.incarnation),
    )
    .unwrap();
    kv9_meta::data_groups::activation::plan_activation(&mut txn, creation.intent()).unwrap();
    kv9_meta::data_groups::migration::plan_migration(
        &mut txn,
        operation,
        creation.intent().task(),
        NodeId(4),
    )
    .unwrap();
    let at = f.meta[1]
        .propose(&Command::from_batch(&txn.into_batch()))
        .unwrap();
    applied(&f.meta, at).await;
    let source = f.backends[0].clone();
    tokio::task::spawn_blocking(move || source.attach_migration_learner("test", digest, operation))
        .await
        .unwrap()
        .unwrap();
    (identity, guard, creation.intent().task())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn large_ttl_runtime_capture_requires_pins_shares_budget_and_installs_exact_image() {
    let f = Fixture::new().await;
    let root = f.backends[0].ttl_context().unwrap().root.clone();
    let digest = root.digest();
    let operation = [91; 16];
    assert!(matches!(
        f.client(0).activate().await.outcome,
        TtlOutcome::Success { .. }
    ));
    for first in (0..900).step_by(10) {
        let outcome = f
            .client(0)
            .call(Operation::Put {
                pairs: (first..first + 10)
                    .map(|i| (format!("a{i:04}").into_bytes(), vec![i as u8; 60 * 1024]))
                    .collect(),
                ttl_secs: Some(60),
            })
            .await
            .outcome;
        assert!(matches!(outcome, TtlOutcome::Success { .. }), "{outcome:?}");
    }
    let (identity, guard, creation_task) = prepare_migration(&f, &root, operation).await;
    let objects = Arc::new(Objects::default());
    enable_payload_replicas(&f, &objects);
    let memory =
        Arc::new(kv9_engine::checkpoint_memory::ImageBudget::new(9 * 1024 * 1024).unwrap());
    let disk =
        Arc::new(kv9_engine::checkpoint_spool_budget::SpoolBudget::new(64 * 1024 * 1024).unwrap());
    let pending_disk = Arc::new(
        kv9_engine::checkpoint_pending_budget::PendingDiskBudget::new(64 * 1024 * 1024).unwrap(),
    );
    let uploader = Arc::new(
        RemoteUploader::for_harness(objects.clone())
            .with_image_budget(memory.clone())
            .with_spool_budget(disk.clone())
            .with_pending_budget(pending_disk.clone()),
    );
    let backend = with_uploader(&f.backends[0], uploader.clone());
    *f.hooks[0].capture_backend.lock().unwrap() = Some(backend.clone());
    let plan = {
        let backend = backend.clone();
        tokio::task::spawn_blocking(move || backend.plan_migration_image("test", digest, operation))
            .await
            .unwrap()
            .unwrap()
    };
    let manifest = CheckpointManifest::decode(&plan.manifest).unwrap();
    assert!(manifest.requires_large_format().unwrap());
    let source_len = std::fs::metadata(f.path.join("n1/capture.plan"))
        .unwrap()
        .len();
    let mut durable_copies = 1;
    for i in 1..3 {
        let path = f.path.join(format!("n{}/capture-replica.plan", i + 1));
        if path.exists() {
            durable_copies += 1;
            let replica = f.hooks[i].capture_backend.lock().unwrap().clone().unwrap();
            let uploader = replica.uploader.as_ref().unwrap();
            let memory = uploader.image_memory_snapshot();
            let disk = uploader.pending_disk_snapshot();
            assert_eq!(std::fs::metadata(path).unwrap().len(), source_len);
            assert_eq!(disk.retained_bytes, source_len);
            assert_eq!(disk.open_readers, 0);
            assert_eq!(disk.open_writers, 0);
            assert_eq!(memory.retained_bytes, 0);
            assert!(memory.peak_bytes > 0);
            assert!(memory.peak_bytes <= 9 * 1024 * 1024);
            eprintln!(
                "capture_replica node={} file_bytes={} peak_sst_buffer={}",
                i + 1,
                source_len,
                memory.peak_bytes
            );
        }
    }
    assert!(durable_copies >= 2);
    assert_eq!(objects.puts.load(Ordering::SeqCst), 0);
    assert_eq!(disk.snapshot().reserved_bytes, 0);
    let unbound = {
        let backend = backend.clone();
        tokio::task::spawn_blocking(move || {
            backend.capture_migration_image("test", digest, operation)
        })
        .await
        .unwrap()
    };
    assert!(unbound.is_err(), "unpublished owners cannot upload");
    assert_eq!(objects.puts.load(Ordering::SeqCst), 0);
    assert_eq!(disk.snapshot().active_spools, 0);
    // A loaded durable plan needs no new spool even while another planner
    // holds nearly all scratch space. A new checkpoint still shares that bound.
    let group = backend.raw_directory.by_region(f.range.region).unwrap();
    let (driver, engine, _) = group.capture_parts();
    let held = kv9_raft::snapshot_install::capture::plan_capture_spooled(
        driver,
        engine,
        root.cluster_id,
        &uploader,
    )
    .unwrap();
    assert!(disk.snapshot().reserved_bytes > 48 * 1024 * 1024);
    let reused = {
        let backend = backend.clone();
        tokio::task::spawn_blocking(move || backend.plan_migration_image("test", digest, operation))
            .await
            .unwrap()
    };
    let reused = reused.unwrap();
    assert_eq!(reused.manifest, plan.manifest);
    assert_eq!(reused.cut, plan.cut);
    let mut journal = uploader.journal(engine.path().with_extension("pending"));
    assert!(uploader
        .plan_spooled(engine.freeze(manifest.scope.clone()).unwrap(), &mut journal)
        .unwrap_err()
        .to_string()
        .contains("spool byte budget"));
    drop(held);
    let capture_path = engine.path().parent().unwrap().join("capture.plan");
    assert_eq!(
        pending_disk.snapshot().retained_bytes,
        std::fs::metadata(&capture_path).unwrap().len()
    );
    assert!(pending_disk.snapshot().retained_bytes > 48 * 1024 * 1024);
    // Reproduce the original plan -> write -> bind -> capture failure.
    assert!(matches!(
        f.client(0)
            .call(Operation::Put {
                pairs: vec![(b"b-between-plan-bind".to_vec(), b"tail only".to_vec())],
                ttl_secs: None,
            })
            .await
            .outcome,
        TtlOutcome::Success { .. }
    ));
    let bindings = {
        let metadata = f.backends[1].clone();
        let bytes = plan.manifest.clone();
        tokio::task::spawn_blocking(move || {
            metadata.bind_migration_image("test", digest, operation, &bytes)
        })
        .await
        .unwrap()
        .unwrap()
    };
    let barrier = f.meta[1].propose(&Command::Noop).unwrap();
    applied(&f.meta, barrier).await;
    objects.refuse.store(true, Ordering::SeqCst);
    let failed = {
        let backend = backend.clone();
        tokio::task::spawn_blocking(move || {
            backend.capture_migration_image("test", digest, operation)
        })
        .await
        .unwrap()
    };
    assert!(failed
        .unwrap_err()
        .to_string()
        .contains("injected migration upload"));
    assert_eq!(disk.snapshot().reserved_bytes, 0);
    for owner in [bindings.source_owner, bindings.destination_owner] {
        let view = f.backends[1]
            .node
            .meta_raft
            .store
            .begin()
            .unwrap()
            .into_view();
        assert_eq!(
            kv9_meta::retention::retention_owner(view.as_ref(), &root, owner)
                .unwrap()
                .unwrap()
                .phase,
            PinPhase::Published
        );
    }
    objects.refuse.store(false, Ordering::SeqCst);
    let captured = {
        let backend = backend.clone();
        tokio::task::spawn_blocking(move || {
            backend.capture_migration_image("test", digest, operation)
        })
        .await
        .unwrap()
        .unwrap()
    };
    assert_eq!(captured.cut, plan.cut);
    assert_eq!(captured.source_owner, bindings.source_owner);
    assert_eq!(disk.snapshot().reserved_bytes, 0);
    assert!(memory.snapshot().peak_bytes <= 9 * 1024 * 1024);
    let puts = objects.puts.load(Ordering::SeqCst);
    assert!(matches!(
        f.client(0)
            .call(Operation::Put {
                pairs: vec![(b"z-after-capture".to_vec(), b"tail only".to_vec())],
                ttl_secs: None,
            })
            .await
            .outcome,
        TtlOutcome::Success { .. }
    ));
    let moved = {
        let backend = backend.clone();
        tokio::task::spawn_blocking(move || {
            backend.capture_migration_image("test", digest, operation)
        })
        .await
        .unwrap()
    };
    let moved = moved.unwrap();
    assert_eq!(moved.cut, plan.cut);
    assert_eq!(moved.record, captured.record);
    assert!(objects.puts.load(Ordering::SeqCst) > puts);
    // Reconstruct the production backend/uploader from the same on-disk slot.
    // A lowered limit below existing debt permits replay, but no new plan.
    let reboot_budget =
        Arc::new(kv9_engine::checkpoint_pending_budget::PendingDiskBudget::new(1).unwrap());
    reboot_budget.inventory_journal(&capture_path).unwrap();
    let reboot_uploader = Arc::new(
        RemoteUploader::for_harness(objects.clone())
            .with_image_budget(memory.clone())
            .with_pending_budget(reboot_budget.clone())
            .with_spool_budget(Arc::new(
                kv9_engine::checkpoint_spool_budget::SpoolBudget::new(1).unwrap(),
            )),
    );
    drop(backend);
    let backend = with_uploader(&f.backends[0], reboot_uploader);
    let recovered = {
        let backend = backend.clone();
        tokio::task::spawn_blocking(move || {
            backend.capture_migration_image("test", digest, operation)
        })
        .await
        .unwrap()
        .unwrap()
    };
    assert_eq!(recovered.record, captured.record);
    assert_eq!(reboot_budget.snapshot().open_readers, 0);
    assert_eq!(disk.snapshot().reserved_bytes, 0);
    let key = encode_key(KeyMode::Raw, f.range.keyspace, b"a0000").unwrap();
    let expected_expiry = engine.get(ColumnFamily::Write, &key).unwrap().unwrap();
    let expected_clock = engine
        .get(ColumnFamily::Default, CLOCK_KEY)
        .unwrap()
        .unwrap();
    kv9_raft::snapshot_install::install_captured_record(
        &guard,
        identity,
        &captured.record,
        &uploader,
    )
    .unwrap();
    let adopted =
        kv9_raft::snapshot_install::adopt_for_runtime(&guard, identity, f.range.region, &uploader)
            .unwrap();
    assert_eq!(adopted.configuration.learners, vec![4]);
    let mut storage = DiskRaftStorage::recover(&adopted.raft_directory).unwrap();
    let (restored, _, dependencies) = adopted
        .open_engine_with_dependencies(&mut storage, &uploader)
        .unwrap();
    assert_eq!(
        restored.get(ColumnFamily::Default, &key).unwrap(),
        Some(vec![0; 60 * 1024])
    );
    assert_eq!(
        restored.get(ColumnFamily::Write, &key).unwrap(),
        Some(expected_expiry)
    );
    assert_eq!(
        restored.get(ColumnFamily::Default, CLOCK_KEY).unwrap(),
        Some(expected_clock)
    );
    for i in 0..900 {
        let key = encode_key(
            KeyMode::Raw,
            f.range.keyspace,
            format!("a{i:04}").as_bytes(),
        )
        .unwrap();
        assert_eq!(
            restored.get(ColumnFamily::Default, &key).unwrap(),
            Some(vec![i as u8; 60 * 1024])
        );
        assert_eq!(
            restored.get(ColumnFamily::Write, &key).unwrap(),
            engine.get(ColumnFamily::Write, &key).unwrap()
        );
    }
    assert_eq!(
        restored
            .get(
                ColumnFamily::Default,
                &encode_key(KeyMode::Raw, f.range.keyspace, b"z-after-capture").unwrap()
            )
            .unwrap(),
        None
    );
    assert_eq!(
        restored
            .get(
                ColumnFamily::Default,
                &encode_key(KeyMode::Raw, f.range.keyspace, b"b-between-plan-bind").unwrap()
            )
            .unwrap(),
        None
    );
    assert!(dependencies.used_original_install_image());
    assert_eq!(
        dependencies.selected().unwrap().manifest(),
        RootDigest::sha256(&plan.manifest)
    );
    eprintln!(
        "runtime_capture image_bytes={} peak_sst_buffer={} peak_spool={}",
        captured.object_bytes,
        memory.snapshot().peak_bytes,
        disk.snapshot().peak_bytes
    );
    // A committed abort permits reuse of the one local slot for a new
    // operation. It does not release the old remote pins, even after unknown PUT.
    let metadata = f.backends[1].clone();
    tokio::task::spawn_blocking(move || metadata.record_migration_abort("test", digest, operation))
        .await
        .unwrap()
        .unwrap();
    let next_operation = [92; 16];
    let metadata = f.backends[1].clone();
    tokio::task::spawn_blocking(move || {
        metadata.migrate_data_group("test", digest, next_operation, creation_task, NodeId(4))
    })
    .await
    .unwrap()
    .unwrap();
    let barrier = f.meta[1].propose(&Command::Noop).unwrap();
    applied(&f.meta, barrier).await;
    let source = backend.clone();
    let aborted =
        tokio::task::spawn_blocking(move || source.plan_migration_image("test", digest, operation))
            .await
            .unwrap()
            .unwrap_err();
    assert!(aborted.to_string().contains("aborted"));
    drop(backend);
    let backend = with_uploader(&f.backends[0], uploader.clone());
    let source = backend.clone();
    let next = tokio::task::spawn_blocking(move || {
        source.plan_migration_image("test", digest, next_operation)
    })
    .await
    .unwrap()
    .unwrap();
    assert!(next.cut.index > plan.cut.index);
    assert_ne!(next.manifest, plan.manifest);
    assert_eq!(pending_disk.snapshot().named_paths, 1);
    assert_eq!(
        pending_disk.snapshot().retained_bytes,
        std::fs::metadata(&capture_path).unwrap().len()
    );
    for owner in [bindings.source_owner, bindings.destination_owner] {
        let view = f.backends[1]
            .node
            .meta_raft
            .store
            .begin()
            .unwrap()
            .into_view();
        assert_eq!(
            kv9_meta::retention::retention_owner(view.as_ref(), &root, owner)
                .unwrap()
                .unwrap()
                .phase,
            PinPhase::Published
        );
    }
    drop((restored, storage, adopted, guard, group, backend));
    f.close().await;
}
