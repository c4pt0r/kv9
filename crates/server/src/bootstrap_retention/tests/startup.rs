use super::*;
use crate::bootstrap_retention::startup::enroll;
use kv9_common::store_lifecycle::StoreGuard;

fn prepared() -> (Fixture, StoreGuard) {
    let mut f = Fixture::new();
    let mut guard = StoreGuard::lock(&f.dir).unwrap();
    let record = guard.prepare(NodeId(1)).unwrap();
    f.root = RootDescriptor::new(
        f.root.cluster_id,
        f.root.bootstrap_generation,
        vec![RootVoter {
            node_id: record.node_id,
            store_incarnation: record.incarnation,
            addr: f.root.voters[0].addr,
        }],
        b"bootstrap-enrollment-test",
    )
    .unwrap();
    f.identity = StoreIdentity::for_voter(&f.root, NodeId(1)).unwrap();
    guard.bind(&f.root, &f.identity).unwrap();
    guard.activate(&f.identity).unwrap();
    (f, guard)
}
fn reader<'a>(f: &'a Fixture, uploader: &'a RemoteUploader) -> BootstrapReader<'a> {
    BootstrapReader {
        directory: &f.dir,
        root: &f.root,
        identity: f.identity,
        uploader,
    }
}
#[test]
fn startup_enrollment_requires_exact_published_source_and_persists_no_authority_on_failure() {
    let (f, guard) = prepared();
    let uploader = RemoteUploader::for_harness(Arc::new(MemoryObjectStore::default()));
    let reader = reader(&f, &uploader);
    assert!(enroll::<Ledger>(&guard, &reader, &f.manifest, 0, || Err(error("no quorum"))).is_err());
    assert!(!path(&f.dir).exists());
    for phase in [None, Some(PinPhase::Held), Some(PinPhase::Released)] {
        let ledger = f.ledger();
        {
            let mut owners = ledger.owners.lock().unwrap();
            if let Some(phase) = phase {
                for owner in owners.values_mut() {
                    owner.phase = phase;
                }
            } else {
                owners.clear();
            }
        }
        assert!(enroll(&guard, &reader, &f.manifest, 0, || Ok(ledger)).is_err());
        assert!(!path(&f.dir).exists());
    }
    enroll(&guard, &reader, &f.manifest, 0, || Ok(f.ledger())).unwrap();
    let saved = load(&f.dir, f.identity).unwrap();
    assert_eq!(saved.active.len(), 1);
    assert!(saved.active[0].authorized);
    // Repeated identical image with a later publication still uses the original
    // confirmed protection; no metadata quorum is consulted for cold boot.
    enroll::<Ledger>(&guard, &reader, &f.manifest, 8, || panic!("offline boot")).unwrap();
    assert_eq!(load(&f.dir, f.identity).unwrap().active[0].predecessor, 0);
}

#[test]
fn startup_enrollment_retries_only_the_durable_prospective_slot() {
    let (f, guard) = prepared();
    let uploader = RemoteUploader::for_harness(Arc::new(MemoryObjectStore::default()));
    let reader = reader(&f, &uploader);
    for fault in [5, 2] {
        // Unknown Share, then unknown Publish.
        let ledger = f.ledger();
        ledger.fault.store(fault, Ordering::SeqCst);
        assert!(enroll(&guard, &reader, &f.manifest, 0, || Ok(ledger)).is_err());
        let state = load(&f.dir, f.identity).unwrap();
        assert_eq!(state.active.len(), 1);
        assert!(!state.active[0].authorized);
        assert!(reader
            .restore(&f.manifest)
            .unwrap_err()
            .to_string()
            .contains("no durable"));
    }
    let pending = load(&f.dir, f.identity).unwrap().active[0].pending;
    enroll(&guard, &reader, &f.manifest, 9, || Ok(f.ledger())).unwrap();
    let grant = load(&f.dir, f.identity).unwrap().active[0];
    assert!(grant.authorized);
    assert_eq!(grant.pending, pending);
    assert_eq!(grant.predecessor, 0);
}

#[test]
fn startup_enrollment_preserves_fences_unrelated_slots_and_corrupt_pending_without_rpc() {
    let (f, guard) = prepared();
    let uploader = RemoteUploader::for_harness(Arc::new(MemoryObjectStore::default()));
    let reader = reader(&f, &uploader);
    let grant = Grant {
        predecessor: 0,
        image: image(&f.manifest).unwrap(),
        pending: Some(
            CheckpointIntent::new(&f.root, &f.manifest, 0)
                .unwrap()
                .pending_id(),
        ),
        authorized: false,
    };
    let states = [
        State {
            active: vec![],
            retiring: vec![grant],
        },
        State {
            active: vec![Grant {
                image: [91; 32],
                ..grant
            }],
            retiring: vec![],
        },
        State {
            active: vec![Grant {
                authorized: true,
                pending: Some(kv9_common::retention::OwnerId::new([21; 16]).unwrap()),
                ..grant
            }],
            retiring: vec![],
        },
    ];
    for state in states {
        save(&f.dir, f.identity, &state).unwrap();
        let before = fs::read(path(&f.dir)).unwrap();
        assert!(enroll::<Ledger>(&guard, &reader, &f.manifest, 0, || panic!(
            "must not call ledger"
        ))
        .is_err());
        assert_eq!(fs::read(path(&f.dir)).unwrap(), before);
    }
}

#[test]
fn startup_enrollment_keeps_other_debt_within_two_slots_without_retiring_it_before_recovery() {
    let (f, guard) = prepared();
    let uploader = RemoteUploader::for_harness(Arc::new(MemoryObjectStore::default()));
    let reader = reader(&f, &uploader);
    let other = Grant {
        predecessor: 17,
        image: [23; 32],
        pending: Some(kv9_common::retention::OwnerId::new([25; 16]).unwrap()),
        authorized: false,
    };
    for retiring in [false, true] {
        let state = if retiring {
            State {
                active: vec![],
                retiring: vec![other],
            }
        } else {
            State {
                active: vec![other],
                retiring: vec![],
            }
        };
        save(&f.dir, f.identity, &state).unwrap();
        enroll(&guard, &reader, &f.manifest, 0, || Ok(f.ledger())).unwrap();
        let state = load(&f.dir, f.identity).unwrap();
        assert_eq!(state.active.len() + state.retiring.len(), 2);
        assert!(state
            .active
            .iter()
            .any(|g| g.image == image(&f.manifest).unwrap() && g.authorized));
        assert!(state
            .active
            .iter()
            .chain(&state.retiring)
            .any(|g| *g == other));
        assert_eq!(state.retiring.len(), usize::from(retiring));
    }
}

#[test]
fn fresh_metadata_open_skips_enrollment_and_cross_store_directory_is_refused() {
    use crate::bootstrap_retention::startup::{open_metadata, StartupLedger};
    let (f, guard) = prepared();
    let uploader = RemoteUploader::for_harness(Arc::new(MemoryObjectStore::default()));
    let reader = reader(&f, &uploader);
    let mut storage = kv9_raft::storage::DiskRaftStorage::open(&f.dir.join("raft"), &[1])
        .unwrap()
        .0;
    let catalog = f.dir.join("catalog.wal");
    let (engine, _, anchor) =
        open_metadata::<StartupLedger>(&guard, &mut storage, &catalog, &reader, || {
            panic!("fresh WAL needs no grant")
        })
        .unwrap();
    assert!(anchor.is_none());
    assert!(!path(&f.dir).exists());
    drop(engine);
    assert!(open_metadata::<StartupLedger>(
        &guard,
        &mut storage,
        &f.dir.join("other/catalog.wal"),
        &reader,
        || panic!("cross store open")
    )
    .is_err());
}

#[test]
fn authorized_bootstrap_slot_requires_current_store_floor_before_offline_reuse_or_open() {
    let (f, guard) = prepared();
    let uploader = RemoteUploader::for_harness(Arc::new(MemoryObjectStore::default()));
    let reader = reader(&f, &uploader);
    enroll(&guard, &reader, &f.manifest, 0, || Ok(f.ledger())).unwrap();
    let receipt = fs::read(path(&f.dir)).unwrap();
    drop(guard);
    // Stopped format fixture: the receipt remains unchanged and already
    // authorized. The existing early acknowledgement must still check V9.
    let lifecycle = f
        .dir
        .join(kv9_common::store_lifecycle::STORE_LIFECYCLE_FILE);
    let mut bytes = fs::read(&lifecycle).unwrap();
    bytes[..8].copy_from_slice(b"KV9LIFE8");
    let payload = bytes.len() - 32;
    let digest = RootDigest::sha256(&bytes[..payload]);
    bytes[payload..].copy_from_slice(digest.as_bytes());
    fs::write(&lifecycle, bytes).unwrap();
    let mut guard = StoreGuard::lock(&f.dir).unwrap();
    assert!(guard.require_current_writer_format().is_err());
    assert!(
        enroll::<Ledger>(&guard, &reader, &f.manifest, 0, || panic!(
            "offline grant cannot query quorum"
        ))
        .is_err(),
        "authorized receipt bypassed the new floor"
    );
    let (mut storage, _) =
        kv9_raft::storage::DiskRaftStorage::open(&f.dir.join("raft"), &[1]).unwrap();
    let catalog = f.dir.join("metadata.wal");
    assert!(super::super::startup::open_metadata::<Ledger>(
        &guard,
        &mut storage,
        &catalog,
        &reader,
        || panic!("unfenced startup called ledger")
    )
    .is_err());
    assert!(!catalog.exists(), "unfenced startup opened the engine WAL");
    assert_eq!(fs::read(path(&f.dir)).unwrap(), receipt);
    let record = guard.record();
    guard.fence_legacy_writers().unwrap();
    assert_eq!(guard.record(), record);
    enroll::<Ledger>(&guard, &reader, &f.manifest, 0, || {
        panic!("offline grant cannot query quorum")
    })
    .unwrap();
    assert_eq!(fs::read(path(&f.dir)).unwrap(), receipt);
}
