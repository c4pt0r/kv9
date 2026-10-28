use super::*;
use kv9_common::retention::ResourceKind;
use kv9_common::{ClusterId, NodeId, StoreIncarnation};
use kv9_meta::retention::{LocalLifetime, OwnerDescriptor};
use std::sync::atomic::{AtomicBool, Ordering};

struct Fixture {
    path: PathBuf,
    identity: StoreIdentity,
    binding: OwnerBinding,
}
impl Fixture {
    fn new() -> Self {
        let identity = StoreIdentity {
            root_digest: RootDigest::sha256(b"drain-outbox-test"),
            cluster_id: ClusterId::mint().unwrap(),
            node_id: NodeId(1),
            store_incarnation: StoreIncarnation::mint().unwrap(),
        };
        let path =
            std::env::temp_dir().join(format!("kv9-drain-outbox-{}", identity.store_incarnation));
        fs::create_dir_all(&path).unwrap();
        let binding = OwnerBinding {
            descriptor: OwnerDescriptor {
                root: identity.root_digest,
                id: OwnerId::new([1; 16]).unwrap(),
                kind: OwnerKind::Build,
                region: 100,
                conf_ver: 1,
                version: 1,
                operation: [2; 32],
                subject: [3; 32],
                subject_is_anchor: false,
                local: Some(LocalLifetime {
                    node: 1,
                    store: *identity.store_incarnation.as_bytes(),
                    process: [8; 16],
                }),
            },
            generation: NonZeroU64::new(1).unwrap(),
            resources: vec![ResourceIdentity::new(
                identity.root_digest,
                ResourceKind::Sst,
                [4; 16],
                [5; 32],
            )
            .unwrap()],
        };
        Self {
            path,
            identity,
            binding,
        }
    }
    fn open(&self) -> Arc<DrainOutbox> {
        DrainOutbox::open(&self.path, self.identity).unwrap()
    }
    fn bytes(&self, issued: u64, resolved: u64) -> Vec<u8> {
        encode_request(
            self.identity.root_digest,
            &LedgerRequest::FinishCheckpointExecution {
                pending: OwnerToken {
                    owner: OwnerId::new([7; 16]).unwrap(),
                    generation: NonZeroU64::new(1).unwrap(),
                },
                executor: self.binding.clone(),
                issued,
                resolved,
            },
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
#[derive(Default)]
struct Ledger {
    seen: Mutex<Vec<Vec<u8>>>,
    refuse: AtomicBool,
}
impl CheckpointLedger for Ledger {
    fn apply(&self, bytes: Vec<u8>) -> Result<()> {
        self.seen.lock().unwrap().push(bytes);
        if self.refuse.load(Ordering::SeqCst) {
            Err(error("lost receipt"))
        } else {
            Ok(())
        }
    }
    fn owner(&self, _: RootDigest, _: OwnerId) -> Result<Option<Vec<u8>>> {
        panic!("replay must preserve saved bytes")
    }
}

#[test]
fn durable_report_reopens_replays_identically_and_only_acknowledgement_removes_it() {
    let f = Fixture::new();
    let outbox = f.open();
    let slot = outbox.reserve(f.binding.token()).unwrap();
    let bytes = f.bytes(7, 3);
    slot.persist(&bytes).unwrap();
    assert!(slot.persist(&f.bytes(7, 7)).is_err());
    assert!(slot.release_unused().is_ok()); // A durable slot is not an unused reservation.
    drop(slot);
    drop(outbox);
    let outbox = f.open();
    assert_eq!(outbox.snapshot().ready, 1);
    let mut next = f.binding.token();
    next.generation = NonZeroU64::new(2).unwrap();
    assert!(outbox.reserve(next).is_err());
    let ledger = Ledger::default();
    ledger.refuse.store(true, Ordering::SeqCst);
    assert!(outbox.replay_one(&ledger).is_err());
    assert_eq!(outbox.snapshot().ready, 1);
    drop(outbox);
    let outbox = f.open();
    ledger.refuse.store(false, Ordering::SeqCst);
    assert!(outbox.replay_one(&ledger).unwrap());
    assert_eq!(*ledger.seen.lock().unwrap(), vec![bytes.clone(), bytes]);
    assert!(!outbox.has_pending());
    drop(outbox);
    assert!(!f.open().has_pending());
}

#[test]
fn reader_report_uses_same_bounded_slots_and_recovers_the_original_process_generation() {
    let mut f = Fixture::new();
    f.binding.descriptor.kind = OwnerKind::Reader;
    let bytes = encode_request(
        f.identity.root_digest,
        &LedgerRequest::FinishCheckpointRead {
            successor: OwnerToken {
                owner: OwnerId::new([7; 16]).unwrap(),
                generation: NonZeroU64::new(1).unwrap(),
            },
            reader: f.binding.clone(),
        },
    )
    .unwrap();
    let outbox = f.open();
    let slot = outbox.reserve(f.binding.token()).unwrap();
    slot.persist(&bytes).unwrap();
    drop(slot);
    drop(outbox);
    let outbox = f.open();
    let mut next = f.binding.token();
    next.generation = NonZeroU64::new(2).unwrap();
    assert!(outbox.reserve(next).is_err());
    let ledger = Ledger::default();
    ledger.refuse.store(true, Ordering::SeqCst);
    assert!(outbox.replay_one(&ledger).is_err());
    assert!(outbox.has_pending());
    ledger.refuse.store(false, Ordering::SeqCst);
    assert!(outbox.replay_one(&ledger).unwrap());
    assert_eq!(*ledger.seen.lock().unwrap(), vec![bytes.clone(), bytes]);
    assert!(!outbox.has_pending());
}

#[test]
fn complete_temporary_report_recovers_but_torn_temporary_never_invents_a_report() {
    let f = Fixture::new();
    let bytes = f.bytes(4, 0);
    let dir = f.path.join("checkpoint-drains");
    fs::create_dir_all(&dir).unwrap();
    for cut in [0, 1, 8, bytes.len() / 2, bytes.len() - 1, bytes.len()] {
        fs::write(dir.join("0000.tmp"), &bytes[..cut]).unwrap();
        let outbox = f.open();
        let ledger = Ledger::default();
        assert_eq!(outbox.replay_one(&ledger).unwrap(), cut == bytes.len());
        assert_eq!(
            ledger.seen.lock().unwrap().len(),
            usize::from(cut == bytes.len())
        );
        assert!(!dir.join("0000.tmp").exists());
        assert!(!dir.join("0000.drain").exists());
    }
}

#[test]
fn committed_corruption_wrong_store_unknown_files_and_oversize_never_disappear() {
    let f = Fixture::new();
    let outbox = f.open();
    let slot = outbox.reserve(f.binding.token()).unwrap();
    let bytes = f.bytes(2, 1);
    slot.persist(&bytes).unwrap();
    let path = outbox.path(slot.index, false);
    drop(slot);
    drop(outbox);
    for cut in [0, 1, 8, bytes.len() - 1] {
        fs::write(&path, &bytes[..cut]).unwrap();
        assert!(DrainOutbox::open(&f.path, f.identity).is_err());
        assert!(path.exists());
    }
    fs::write(&path, &bytes).unwrap();
    for identity in [
        StoreIdentity {
            root_digest: RootDigest::sha256(b"another root"),
            ..f.identity
        },
        StoreIdentity {
            node_id: NodeId(2),
            ..f.identity
        },
        StoreIdentity {
            store_incarnation: StoreIncarnation::mint().unwrap(),
            ..f.identity
        },
    ] {
        assert!(DrainOutbox::open(&f.path, identity).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
    fs::rename(&path, path.with_extension("tmp")).unwrap();
    assert!(DrainOutbox::open(
        &f.path,
        StoreIdentity {
            node_id: NodeId(2),
            ..f.identity
        }
    )
    .is_err());
    assert_eq!(fs::read(path.with_extension("tmp")).unwrap(), bytes);
    fs::rename(path.with_extension("tmp"), &path).unwrap();
    fs::write(&path, vec![0; MAX_BYTES + 1]).unwrap();
    assert!(DrainOutbox::open(&f.path, f.identity).is_err());
    fs::remove_file(&path).unwrap();
    fs::write(path.parent().unwrap().join("unknown"), b"keep").unwrap();
    assert!(DrainOutbox::open(&f.path, f.identity).is_err());
}

#[test]
fn slots_are_reserved_before_io_and_reusable_only_after_release_or_durable_ack() {
    let f = Fixture::new();
    let outbox = f.open();
    let mut slots = Vec::new();
    for n in 1..=SLOTS {
        let mut token = f.binding.token();
        token.owner = OwnerId::new((n as u128).to_be_bytes()).unwrap();
        slots.push(outbox.reserve(token).unwrap());
    }
    let mut token = f.binding.token();
    token.owner = OwnerId::new([255; 16]).unwrap();
    assert!(outbox.reserve(token).is_err());
    slots[0].release_unused().unwrap();
    token = slots[0].executor;
    let newer = outbox.reserve(token).unwrap();
    drop(slots.remove(0));
    assert_eq!(
        outbox.snapshot().reserved,
        SLOTS,
        "old guard must not release a reused slot"
    );
    drop(newer);
    drop(slots);
    let ledger = Ledger::default();
    for _ in 0..SLOTS {
        assert!(!outbox.replay_one(&ledger).unwrap());
    }
    assert!(!outbox.has_pending());
    assert!(ledger.seen.lock().unwrap().is_empty());
}

#[test]
fn blocked_metadata_replay_does_not_hold_filesystem_lock_or_delete_a_reused_slot() {
    struct Blocked {
        entered: std::sync::mpsc::Sender<()>,
        release: Mutex<std::sync::mpsc::Receiver<()>>,
    }
    impl CheckpointLedger for Blocked {
        fn apply(&self, _: Vec<u8>) -> Result<()> {
            self.entered.send(()).unwrap();
            self.release
                .lock()
                .unwrap()
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            Ok(())
        }
        fn owner(&self, _: RootDigest, _: OwnerId) -> Result<Option<Vec<u8>>> {
            unreachable!()
        }
    }
    let mut f = Fixture::new();
    let outbox = f.open();
    let first = outbox.reserve(f.binding.token()).unwrap();
    let first_bytes = f.bytes(5, 1);
    first.persist(&first_bytes).unwrap();
    let (entered, receive) = std::sync::mpsc::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    let replay = outbox.clone();
    let job = std::thread::spawn(move || {
        replay.replay_one(&Blocked {
            entered,
            release: Mutex::new(blocked),
        })
    });
    receive
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap();
    // A concurrent live worker received the same metadata receipt and retires
    // the old file; another executor can now use that physical slot.
    first.acknowledge(&first_bytes).unwrap();
    f.binding.descriptor.id = OwnerId::new([19; 16]).unwrap();
    let second = outbox.reserve(f.binding.token()).unwrap();
    assert_eq!(first.index, second.index);
    let second_bytes = f.bytes(0, 0);
    second.persist(&second_bytes).unwrap();
    release.send(()).unwrap();
    assert!(job.join().unwrap().unwrap());
    assert_eq!(
        file(&outbox.path(second.index, false)).unwrap().unwrap(),
        second_bytes
    );
    assert_eq!(outbox.snapshot().ready, 1);
    assert!(outbox.replay_one(&Ledger::default()).unwrap());
}

#[test]
fn one_bad_report_does_not_starve_later_slots_and_cleanup_retries_after_ack() {
    let mut f = Fixture::new();
    let outbox = f.open();
    let first = outbox.reserve(f.binding.token()).unwrap();
    first.persist(&f.bytes(1, 0)).unwrap();
    f.binding.descriptor.id = OwnerId::new([9; 16]).unwrap();
    let second = outbox.reserve(f.binding.token()).unwrap();
    let second_bytes = f.bytes(0, 0);
    second.persist(&second_bytes).unwrap();
    fs::write(outbox.path(first.index, false), b"damaged").unwrap();
    let ledger = Ledger::default();
    assert!(outbox.replay_one(&ledger).is_err());
    assert!(outbox.replay_one(&ledger).unwrap());
    assert_eq!(*ledger.seen.lock().unwrap(), vec![second_bytes]);
    // Simulate the in-memory state after a confirmed metadata reply and unlink,
    // but before successful directory fsync. Retry needs no invented report.
    let digest = RootDigest::sha256(&f.bytes(1, 0));
    outbox.state.lock().unwrap().slots[first.index] = Slot::Acknowledged(first.executor, digest);
    fs::remove_file(outbox.path(first.index, false)).unwrap();
    assert!(outbox.replay_one(&ledger).unwrap());
    assert!(!outbox.has_pending());
    assert_eq!(ledger.seen.lock().unwrap().len(), 1);
}

#[cfg(unix)]
#[test]
fn symlinks_are_refused_without_following_or_deleting_targets() {
    let f = Fixture::new();
    let outbox = f.open();
    let target = f.path.join("keep");
    fs::write(&target, b"keep").unwrap();
    std::os::unix::fs::symlink(&target, outbox.path(0, true)).unwrap();
    assert!(DrainOutbox::open(&f.path, f.identity).is_err());
    let slot = outbox.reserve(f.binding.token()).unwrap();
    assert!(slot.persist(&f.bytes(0, 0)).is_err());
    assert_eq!(fs::read(&target).unwrap(), b"keep");
}
