//! Existing owner transitions, real metadata Raft and loss of exact receipts.
use super::*;
use crate::checkpoint_retention::{CheckpointLedger, CheckpointOwners};
use crate::runtime::checkpoint_retention::RoutedLedger;
use kv9_common::retention::{OwnerId, PinPhase};
use kv9_engine::checkpoint::{FlushScope, RemoteUploader};
use kv9_meta::retention::{decode_owner_observation, LedgerOwner, LocalLifetime};
mod enrollment;

struct Fault<'a> {
    inner: &'a RoutedLedger,
    calls: AtomicUsize,
    fail_at: usize,
    before: bool,
    lifetime: LocalLifetime,
}
impl CheckpointLedger for Fault<'_> {
    fn apply(&self, bytes: Vec<u8>) -> Result<()> {
        let fail = self.calls.fetch_add(1, Ordering::SeqCst) + 1 == self.fail_at;
        if fail && self.before {
            return Err(Error::Engine("injected pre-append refusal".into()));
        }
        self.inner.apply(bytes)?;
        if fail {
            return Err(Error::Engine("injected committed receipt loss".into()));
        }
        Ok(())
    }
    fn owner(&self, root: RootDigest, owner: OwnerId) -> Result<Option<Vec<u8>>> {
        self.inner.owner(root, owner)
    }
    fn local_lifetime(&self) -> Result<LocalLifetime> {
        Ok(self.lifetime)
    }
}
fn observe(ledger: &RoutedLedger, root: RootDigest, id: OwnerId) -> LedgerOwner {
    decode_owner_observation(&ledger.owner(root, id).unwrap().unwrap(), root, id).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn closed_admission_survives_lost_receipts_delayed_entry_and_metadata_wal_reopen() {
    let mut f = Fixture::new().await;
    let backend = f.backends[0].clone();
    let root = backend.ttl_context().unwrap().root.clone();
    let engine = backend
        .raw_directory
        .by_region(f.range.region)
        .unwrap()
        .capture_parts()
        .1
        .clone();
    let range = f.range.clone();
    let ledger = RoutedLedger::new(&backend, tokio::runtime::Handle::current());
    let (root, pending, expected) = tokio::task::spawn_blocking(move || {
        let manifest = RemoteUploader::plan(
            engine
                .freeze(FlushScope {
                    cluster: root.cluster_id.to_string(),
                    region: range.region.0,
                    conf_ver: range.conf_ver,
                    version: range.version,
                })
                .unwrap(),
        )
        .unwrap()
        .manifest()
        .clone();
        let make = || CheckpointOwners::new(&root, &manifest, 0).unwrap();
        make().before_io(&ledger).unwrap();
        let entered = make().execution(&ledger).unwrap();
        entered.before_io(&ledger).unwrap();
        let lifetime = ledger.local_lifetime().unwrap();
        let mut other = lifetime;
        other.process = *kv9_common::ProcessIncarnation::mint().unwrap().as_bytes();
        let delayed = Fault {
            inner: &ledger,
            calls: AtomicUsize::new(0),
            fail_at: usize::MAX,
            before: false,
            lifetime: other,
        };
        let unentered = make().execution(&delayed).unwrap();
        let lost_close = Fault {
            inner: &ledger,
            calls: AtomicUsize::new(0),
            fail_at: 1,
            before: false,
            lifetime,
        };
        assert!(make().close_admission(&lost_close).is_err());
        let observed = ledger
            .executors(root.digest(), make().pending_id(), None, 1)
            .unwrap();
        assert!(observed.admission.is_closed());
        assert_eq!(observed.owners.len(), 1);
        assert_eq!(observed.owners[0].phase, PinPhase::Published);
        make().before_io(&ledger).unwrap(); // Recording the intent cannot reopen it.
        make().close_admission(&ledger).unwrap();
        assert!(entered.before_io(&ledger).is_err());
        assert!(unentered.before_io(&delayed).is_err());
        let lost_cancel = Fault {
            inner: &ledger,
            calls: AtomicUsize::new(0),
            fail_at: 1,
            before: false,
            lifetime: other,
        };
        assert!(unentered.after_local_drain(&lost_cancel).is_err());
        unentered.after_local_drain(&delayed).unwrap();
        entered.after_local_drain(&ledger).unwrap();
        let next = make().execution(&ledger).unwrap();
        assert!(next.before_io(&ledger).is_err());
        next.after_local_drain(&ledger).unwrap();
        assert!(entered.after_local_drain(&ledger).is_err());
        let pending = observe(&ledger, root.digest(), make().pending_id());
        assert_eq!(pending.phase, PinPhase::Published);
        let mut cursor = None;
        let mut expected = Vec::new();
        loop {
            let page = ledger
                .executors(root.digest(), make().pending_id(), cursor, 1)
                .unwrap();
            assert_eq!(page.admission, observed.admission);
            assert_eq!(page.owners.len(), 1);
            assert_eq!(page.owners[0].phase, PinPhase::Released);
            expected.extend(page.owners);
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(expected.len(), 2);
        assert_eq!(
            expected
                .iter()
                .map(|o| o.binding.generation.get())
                .sum::<u64>(),
            3
        );
        (root, pending, (observed.admission, expected))
    })
    .await
    .unwrap();
    drop(backend);
    for server in f.servers.drain(..) {
        server.abort();
        let _ = server.await;
    }
    for driver in f.meta.iter().chain(f.data.iter()) {
        driver.stop();
    }
    for owner in f.owners.drain(..) {
        owner.join().unwrap();
    }
    f.backends.clear();
    f.meta.clear();
    f.data.clear();
    f.hooks.clear();
    let (metadata, _) = WalEngine::open(f.path.join("n2/meta.wal")).unwrap();
    let view = metadata.snapshot().unwrap();
    let page = kv9_meta::retention::checkpoint_admission::executors(
        view.as_ref(),
        &root,
        pending.binding.descriptor.id,
        None,
        16,
    )
    .unwrap();
    assert_eq!((page.admission, page.owners), expected);
    assert_eq!(
        kv9_meta::retention::retention_owner(view.as_ref(), &root, pending.binding.descriptor.id)
            .unwrap()
            .unwrap(),
        pending
    );
    drop(view);
    drop(metadata);
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn legacy_retention_upgrade_loses_receipt_then_preserves_generation_after_wal_reopen() {
    use kv9_common::retention::{PinCommand, RetentionRecord};
    use kv9_meta::retention::{
        encode_owner_observation, encode_request, LedgerRequest, OwnerBinding, OwnerDescriptor,
        OwnerKind,
    };
    use std::num::NonZeroU64;

    let mut f = Fixture::new().await;
    let backend = f.backends[0].clone();
    let root = backend.ttl_context().unwrap().root.clone();
    let group = backend.raw_directory.by_region(f.range.region).unwrap();
    let engine = group.capture_parts().1.clone();
    let range = f.range.clone();
    let meta = f.meta[1].clone();
    let ledger = RoutedLedger::new(&backend, tokio::runtime::Handle::current());
    drop(group);
    let (root, expected, base_rows) = tokio::task::spawn_blocking(move || {
        let manifest = RemoteUploader::plan(
            engine
                .freeze(FlushScope {
                    cluster: root.cluster_id.to_string(),
                    region: range.region.0,
                    conf_ver: range.conf_ver,
                    version: range.version,
                })
                .unwrap(),
        )
        .unwrap()
        .manifest()
        .clone();
        let resources =
            crate::checkpoint_retention::manifest_resources(root.digest(), &manifest).unwrap();
        let old = OwnerBinding {
            descriptor: OwnerDescriptor {
                root: root.digest(),
                id: OwnerId::new([0xa5; 16]).unwrap(),
                kind: OwnerKind::Build,
                region: range.region.0,
                conf_ver: range.conf_ver,
                version: range.version,
                operation: [0xa6; 32],
                subject: *RootDigest::sha256(&manifest.encode().unwrap()).as_bytes(),
                subject_is_anchor: false,
                local: Some(ledger.local_lifetime().unwrap()),
            },
            generation: NonZeroU64::new(1).unwrap(),
            resources,
        };
        // Deliberately fixed V1 disk fixture, committed by the actual metadata
        // Raft driver. This is not a claim of running an old executable.
        let key = |tag: u8, id: &[u8]| {
            let mut k = b"\0kv9\0retention_v1\0".to_vec();
            k.push(tag);
            k.extend_from_slice(id);
            k
        };
        let seal = |mut b: Vec<u8>| {
            b.extend_from_slice(RootDigest::sha256(&b).as_bytes());
            b
        };
        let mut batch = kv9_engine::WriteBatch::new();
        let mut header = b"KV9LED01".to_vec();
        header.extend_from_slice(root.digest().as_bytes());
        header.push(1);
        header.extend_from_slice(&3u64.to_be_bytes());
        batch.put(ColumnFamily::Default, key(1, b""), seal(header));
        let mut base_rows = Vec::new();
        for resource in &old.resources {
            let mut id = vec![resource.kind() as u8];
            id.extend_from_slice(resource.instance());
            let mut record = RetentionRecord::new(*resource);
            for command in [
                PinCommand::Acquire(old.token()),
                PinCommand::Quiesce(old.token()),
                PinCommand::Release(old.token()),
            ] {
                record.apply(command).unwrap();
            }
            let bytes = record.encode();
            batch.put(ColumnFamily::Default, key(2, &id), bytes.clone());
            base_rows.push((key(7, &id), bytes));
            let mut registration = b"KV9REG01".to_vec();
            registration.extend_from_slice(root.digest().as_bytes());
            registration.extend_from_slice(&id);
            registration.extend_from_slice(resource.content());
            batch.put(ColumnFamily::Default, key(4, &id), seal(registration));
        }
        for (tag, phase) in [(3, PinPhase::Released), (5, PinPhase::Held)] {
            batch.put(
                ColumnFamily::Default,
                key(tag, old.descriptor.id.as_bytes()),
                encode_owner_observation(&LedgerOwner {
                    binding: old.clone(),
                    phase,
                })
                .unwrap(),
            );
        }
        let at = meta
            .propose(&kv9_raft::Command::from_batch(&batch))
            .unwrap();
        meta.wait_applied(at, Duration::from_secs(3)).unwrap();
        assert_eq!(
            observe(&ledger, root.digest(), old.descriptor.id).phase,
            PinPhase::Released
        );
        let make = || CheckpointOwners::new(&root, &manifest, 1).unwrap();
        let fault = Fault {
            inner: &ledger,
            calls: AtomicUsize::new(0),
            fail_at: 1,
            before: false,
            lifetime: ledger.local_lifetime().unwrap(),
        };
        assert!(
            make().before_io(&fault).is_err(),
            "atomic Begin upgrade committed but receipt lost"
        );
        assert_eq!(fault.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            observe(&ledger, root.digest(), old.descriptor.id).binding,
            old
        );
        make().before_io(&ledger).unwrap();
        let pending = observe(&ledger, root.digest(), make().pending_id());
        let mut next = old.clone();
        next.generation = NonZeroU64::new(2).unwrap();
        for request in [
            LedgerRequest::Share {
                from: pending.binding.token(),
                to: next.clone(),
            },
            LedgerRequest::Publish(next.token()),
            LedgerRequest::QuiesceAfterTransfer {
                from: next.token(),
                to: pending.binding.token(),
            },
            LedgerRequest::Release(next.token()),
        ] {
            ledger
                .apply(encode_request(root.digest(), &request).unwrap())
                .unwrap();
        }
        assert!(ledger
            .apply(encode_request(root.digest(), &LedgerRequest::Release(old.token())).unwrap())
            .is_err());
        let expected = observe(&ledger, root.digest(), old.descriptor.id);
        assert_eq!(expected.binding, next);
        assert_eq!(expected.phase, PinPhase::Released);
        (root, expected, base_rows)
    })
    .await
    .unwrap();
    drop(backend);
    for server in f.servers.drain(..) {
        server.abort();
        let _ = server.await;
    }
    for driver in f.meta.iter().chain(f.data.iter()) {
        driver.stop();
    }
    for owner in f.owners.drain(..) {
        owner.join().unwrap();
    }
    f.backends.clear();
    f.meta.clear();
    f.data.clear();
    f.hooks.clear();
    let (metadata, _) = WalEngine::open(f.path.join("n2/meta.wal")).unwrap();
    let view = metadata.snapshot().unwrap();
    assert_eq!(
        kv9_meta::retention::retention_owner(view.as_ref(), &root, expected.binding.descriptor.id)
            .unwrap()
            .unwrap(),
        expected
    );
    for (key, bytes) in base_rows {
        assert_eq!(
            view.get(ColumnFamily::Default, &key).unwrap().unwrap(),
            bytes
        );
    }
    drop(view);
    drop(metadata);
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn executor_receipt_losses_close_exact_generations_and_survive_metadata_wal_reopen() {
    let mut f = Fixture::new().await;
    let backend = f.backends[0].clone();
    let root = backend.ttl_context().unwrap().root.clone();
    let expected_root = root.clone();
    let group = backend.raw_directory.by_region(f.range.region).unwrap();
    let engine = group.capture_parts().1.clone();
    let range = f.range.clone();
    drop(group);
    let ledger = RoutedLedger::new(&backend, tokio::runtime::Handle::current());
    let (records, orphan, root) = tokio::task::spawn_blocking(move || {
        let manifest = RemoteUploader::plan(
            engine
                .freeze(FlushScope {
                    cluster: root.cluster_id.to_string(),
                    region: range.region.0,
                    conf_ver: range.conf_ver,
                    version: range.version,
                })
                .unwrap(),
        )
        .unwrap()
        .manifest()
        .clone();
        let lifetime = ledger.local_lifetime().unwrap();
        let mut records = Vec::new();
        for cut in 1..=4 {
            let make = || CheckpointOwners::new(&root, &manifest, cut as u64).unwrap();
            make().before_io(&ledger).unwrap();
            let id = make().execution_id(lifetime).unwrap();
            let execution = make().execution(&ledger).unwrap();
            let faulty = Fault {
                inner: &ledger,
                calls: AtomicUsize::new(0),
                fail_at: cut,
                before: false,
                lifetime,
            };
            let result = execution
                .before_io(&faulty)
                .and_then(|()| execution.after_local_drain(&faulty));
            assert!(result.is_err(), "receipt cut {cut} must be reached");
            assert_eq!(faulty.calls.load(Ordering::SeqCst), cut);
            // The saved handle retries the original generation, including a
            // Quiesce/Release that actually committed despite its lost receipt.
            execution.after_local_drain(&ledger).unwrap();
            execution.after_local_drain(&ledger).unwrap();
            let first = observe(&ledger, root.digest(), id);
            assert_eq!(first.phase, PinPhase::Released);
            assert_eq!(first.binding.descriptor.local, Some(lifetime));
            assert_eq!(first.binding.generation.get(), 1);
            assert!(
                execution.before_io(&ledger).is_err(),
                "released generation cannot re-enter"
            );

            // Generation 2 is refused before append; closing must recognize
            // the released predecessor instead of getting stuck on its row.
            let next = make().execution(&ledger).unwrap();
            let refused = Fault {
                inner: &ledger,
                calls: AtomicUsize::new(0),
                fail_at: 1,
                before: true,
                lifetime,
            };
            assert!(next.before_io(&refused).is_err());
            assert_eq!(observe(&ledger, root.digest(), id), first);
            next.after_local_drain(&ledger).unwrap();
            assert_eq!(
                observe(&ledger, root.digest(), id).binding.generation.get(),
                2
            );

            let third = make().execution(&ledger).unwrap();
            third.before_io(&ledger).unwrap();
            assert!(
                make().execution(&ledger).is_err(),
                "active generation cannot be adopted by a new handle"
            );
            assert!(execution.after_local_drain(&ledger).is_err());
            assert!(next.after_local_drain(&ledger).is_err());
            assert_eq!(
                observe(&ledger, root.digest(), id).phase,
                PinPhase::Published
            );
            third.after_local_drain(&ledger).unwrap();
            let final_record = observe(&ledger, root.digest(), id);
            assert_eq!(final_record.binding.generation.get(), 3);
            assert_eq!(final_record.phase, PinPhase::Released);
            assert_eq!(
                observe(&ledger, root.digest(), make().pending_id()).phase,
                PinPhase::Published
            );
            records.push(final_record);
        }

        // A different runtime process has a different owner, never an implicit
        // close/takeover of the old process's still-Published execution.
        let make = || CheckpointOwners::new(&root, &manifest, 99).unwrap();
        make().before_io(&ledger).unwrap();
        let old_id = make().execution_id(lifetime).unwrap();
        let old_execution = make().execution(&ledger).unwrap();
        old_execution.before_io(&ledger).unwrap();
        drop(old_execution); // Models loss of the in-memory owner, not a process-kill test.
        let mut newer = lifetime;
        newer.process = *kv9_common::ProcessIncarnation::mint().unwrap().as_bytes();
        let replacement = Fault {
            inner: &ledger,
            calls: AtomicUsize::new(0),
            fail_at: usize::MAX,
            before: false,
            lifetime: newer,
        };
        let new_id = make().execution_id(newer).unwrap();
        assert_ne!(new_id, old_id);
        let execution = make().execution(&replacement).unwrap();
        execution.before_io(&replacement).unwrap();
        // Positive handoff can happen while executor pins still protect the
        // closure. Closing then uses Version, since Pending is already Released.
        make().after_positive_settlement(&ledger).unwrap();
        execution.after_local_drain(&replacement).unwrap();
        assert_eq!(
            observe(&ledger, root.digest(), new_id).phase,
            PinPhase::Released
        );
        let orphan = observe(&ledger, root.digest(), old_id);
        assert_eq!(orphan.phase, PinPhase::Published);
        assert_eq!(
            observe(&ledger, root.digest(), make().version_id()).phase,
            PinPhase::Published
        );
        records.push(observe(&ledger, root.digest(), new_id));
        (records, orphan, root)
    })
    .await
    .unwrap();
    assert_eq!(root, expected_root);
    drop(backend);
    for server in f.servers.drain(..) {
        server.abort();
        let _ = server.await;
    }
    for driver in f.meta.iter().chain(f.data.iter()) {
        driver.stop();
    }
    for owner in f.owners.drain(..) {
        owner.join().unwrap();
    }
    f.backends.clear();
    f.meta.clear();
    f.data.clear();
    f.hooks.clear();
    let (metadata, _) = WalEngine::open(f.path.join("n2/meta.wal")).unwrap();
    let view = metadata.snapshot().unwrap();
    for record in records.into_iter().chain(std::iter::once(orphan)) {
        assert_eq!(
            kv9_meta::retention::retention_owner(
                view.as_ref(),
                &root,
                record.binding.descriptor.id
            )
            .unwrap()
            .unwrap(),
            record
        );
    }
    drop(view);
    drop(metadata);
    f.close().await;
}

mod drain;
mod outbox;
