use super::*;
use kv9_common::{BootstrapGeneration, ClusterId, NodeId, RootVoter, StoreIncarnation};
use kv9_engine::checkpoint::{FlushScope, SstReference};
use kv9_engine::{MemoryObjectStore, ObjectKey, ObjectStore};
use kv9_meta::retention::{decode_request, encode_owner_observation, LedgerOwner, LocalLifetime};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

struct Fixture {
    dir: PathBuf,
    root: RootDescriptor,
    identity: StoreIdentity,
    manifest: CheckpointManifest,
}
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "kv9-bootstrap-reader-{}",
            ClusterId::mint().unwrap()
        ));
        fs::create_dir_all(&dir).unwrap();
        let root = RootDescriptor::new(
            ClusterId::mint().unwrap(),
            BootstrapGeneration::from_bytes([2; 16]),
            vec![RootVoter {
                node_id: NodeId(1),
                addr: "127.0.0.1:20160".parse().unwrap(),
                store_incarnation: StoreIncarnation::from_bytes([3; 16]),
            }],
            b"bootstrap-reader-test",
        )
        .unwrap();
        let identity = StoreIdentity::for_voter(&root, NodeId(1)).unwrap();
        let hash = "ab".repeat(32);
        let manifest = CheckpointManifest {
            scope: FlushScope {
                cluster: root.cluster_id.to_string(),
                region: 1,
                conf_ver: 1,
                version: 1,
            },
            term: 1,
            index: 2,
            files: vec![SstReference {
                key: format!("clusters/{}/regions/1/sst/{hash}", root.cluster_id),
                sha256: hash,
                cf: 0,
                smallest: vec![1],
                largest: vec![1],
                size: 1,
                count: 1,
            }],
        };
        Self {
            dir,
            root,
            identity,
            manifest,
        }
    }
    fn ledger(&self) -> Ledger {
        let ledger = Ledger {
            owners: Mutex::new(BTreeMap::new()),
            identity: self.identity,
            fault: AtomicUsize::new(0),
            history: Mutex::new(BTreeMap::new()),
            root: self.root.clone(),
            directory: self.dir.clone(),
        };
        ledger.version(&self.root, &self.manifest, 0);
        ledger
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

// Receipt-order fault fixture only; actual ledger safety is exercised in meta
// tests and by the real three-voter runtime test, not by this map.
struct Ledger {
    root: RootDescriptor,
    directory: PathBuf,
    history: Mutex<BTreeMap<u64, CheckpointManifest>>,
    owners: Mutex<BTreeMap<kv9_common::retention::OwnerId, LedgerOwner>>,
    identity: StoreIdentity,
    fault: AtomicUsize,
}
impl Ledger {
    fn version(&self, root: &RootDescriptor, m: &CheckpointManifest, predecessor: u64) {
        self.history.lock().unwrap().insert(predecessor, m.clone());
        let binding = CheckpointIntent::new(root, m, predecessor)
            .unwrap()
            .binding(OwnerKind::Version)
            .unwrap();
        self.owners.lock().unwrap().insert(
            binding.descriptor.id,
            LedgerOwner {
                binding,
                phase: PinPhase::Published,
            },
        );
    }
}
impl CheckpointLedger for Ledger {
    fn apply(&self, bytes: Vec<u8>) -> Result<()> {
        let mut owners = self.owners.lock().unwrap();
        match decode_request(&bytes, self.identity.root_digest)? {
            LedgerRequest::BeginBootstrapRead {
                pending,
                source,
                store,
            } => {
                assert_eq!(source, self.identity.node_id);
                assert_eq!(store, self.identity.store_incarnation);
                let history = self.history.lock().unwrap();
                let (predecessor, manifest) = history
                    .iter()
                    .find(|(p, m)| {
                        CheckpointIntent::new(&self.root, m, **p)
                            .unwrap()
                            .pending_id()
                            == pending.owner
                    })
                    .unwrap();
                let to =
                    bootstrap_reader::binding(&self.root, manifest, *predecessor, self.identity)?;
                if self.fault.load(Ordering::SeqCst) == 6 {
                    return Err(error("admission refused"));
                }
                owners.entry(to.descriptor.id).or_insert(LedgerOwner {
                    binding: to,
                    phase: PinPhase::Held,
                });
                if self.fault.load(Ordering::SeqCst) == 5 {
                    return Err(error("admission receipt lost"));
                }
            }
            LedgerRequest::Publish(token) => {
                if self.fault.load(Ordering::SeqCst) == 1 {
                    return Err(error("publish refused"));
                }
                owners.get_mut(&token.owner).unwrap().phase = PinPhase::Published;
                if self.fault.load(Ordering::SeqCst) == 2 {
                    return Err(error("publish receipt lost"));
                }
            }
            LedgerRequest::RetireBootstrapRead {
                pending,
                source,
                store,
            } => {
                assert_eq!(source, self.identity.node_id);
                assert_eq!(store, self.identity.store_incarnation);
                let history = self.history.lock().unwrap();
                let (predecessor, m) = history
                    .iter()
                    .find(|(p, m)| {
                        CheckpointIntent::new(&self.root, m, **p)
                            .unwrap()
                            .pending_id()
                            == pending.owner
                    })
                    .unwrap();
                assert_eq!(
                    fenced_image(&self.directory, self.identity, *predecessor, None)
                        .unwrap()
                        .map(|f| f.image),
                    Some(image(m).unwrap())
                );
                if self.fault.load(Ordering::SeqCst) == 3 {
                    return Err(error("retirement refused"));
                }
                let binding =
                    bootstrap_reader::binding(&self.root, m, *predecessor, self.identity).unwrap();
                owners.insert(
                    binding.descriptor.id,
                    LedgerOwner {
                        binding,
                        phase: PinPhase::Released,
                    },
                );
                if self.fault.load(Ordering::SeqCst) == 4 {
                    return Err(error("retirement receipt lost"));
                }
            }
            _ => panic!("unexpected request"),
        }
        Ok(())
    }
    fn owner(&self, _: RootDigest, id: kv9_common::retention::OwnerId) -> Result<Option<Vec<u8>>> {
        self.owners
            .lock()
            .unwrap()
            .get(&id)
            .map(encode_owner_observation)
            .transpose()
    }
    fn local_lifetime(&self) -> Result<LocalLifetime> {
        Ok(LocalLifetime {
            node: self.identity.node_id.0,
            store: *self.identity.store_incarnation.as_bytes(),
            process: [7; 16],
        })
    }
}

fn before_selection(
    dir: &Path,
    root: &RootDescriptor,
    ledger: &Ledger,
    selected: Option<&CheckpointManifest>,
    manifest: &CheckpointManifest,
    predecessor: u64,
) -> Result<()> {
    super::before_selection(dir, root, ledger, selected, manifest, predecessor, |p| {
        ledger
            .history
            .lock()
            .unwrap()
            .get(&p)
            .cloned()
            .ok_or_else(|| error("missing test history"))
    })
}

#[test]
fn selection_never_saves_an_unconfirmed_grant_and_keeps_both_sides_of_a_crash() {
    let f = Fixture::new();
    let ledger = f.ledger();
    for fault in [1, 2] {
        ledger.fault.store(fault, Ordering::SeqCst);
        assert!(before_selection(&f.dir, &f.root, &ledger, None, &f.manifest, 0).is_err());
        assert!(load(&f.dir, f.identity)
            .unwrap()
            .active
            .iter()
            .all(|g| !g.authorized));
    }
    ledger.fault.store(0, Ordering::SeqCst);
    before_selection(&f.dir, &f.root, &ledger, None, &f.manifest, 0).unwrap();
    let mut next = f.manifest.clone();
    for n in 1..20 {
        let previous = next.clone();
        next.index += 1;
        ledger.version(&f.root, &next, n);
        before_selection(&f.dir, &f.root, &ledger, Some(&previous), &next, n).unwrap();
        let saved = load(&f.dir, f.identity).unwrap().active;
        assert_eq!(saved.len(), 2);
        assert!(saved.iter().any(|g| g.image == image(&previous).unwrap()));
        assert!(saved.iter().any(|g| g.image == image(&next).unwrap()));
        assert!(fs::metadata(path(&f.dir)).unwrap().len() <= MAX_BYTES as u64);
    }
    // An unselected candidate can be replaced, retaining the actual old selection.
    let old = next.clone();
    next.index += 1;
    ledger.version(&f.root, &next, 21);
    before_selection(&f.dir, &f.root, &ledger, Some(&old), &next, 21).unwrap();
    next.index += 1;
    ledger.version(&f.root, &next, 22);
    before_selection(&f.dir, &f.root, &ledger, Some(&old), &next, 22).unwrap();
    assert!(load(&f.dir, f.identity)
        .unwrap()
        .active
        .iter()
        .any(|g| g.image == image(&old).unwrap()));
    let owners = ledger.owners.lock().unwrap();
    assert!(owners
        .values()
        .filter(|o| o.binding.descriptor.kind == OwnerKind::Version)
        .all(|o| o.phase == PinPhase::Published));
    assert_eq!(
        owners
            .values()
            .filter(|o| o.binding.descriptor.kind == OwnerKind::BootstrapReader
                && o.phase != PinPhase::Released)
            .count(),
        2
    );
}

#[derive(Debug, Default)]
struct Objects {
    memory: MemoryObjectStore,
    gets: AtomicUsize,
}
impl ObjectStore for Objects {
    fn get(&self, k: &ObjectKey) -> Result<Option<Vec<u8>>> {
        self.gets.fetch_add(1, Ordering::SeqCst);
        self.memory.get(k)
    }
    fn put(&self, _: &ObjectKey, _: &[u8]) -> Result<()> {
        panic!("bootstrap cannot PUT")
    }
    fn delete(&self, _: &ObjectKey) -> Result<()> {
        panic!("no GC")
    }
    fn list(&self, _: &str) -> Result<Vec<ObjectKey>> {
        panic!("no listing")
    }
}

#[test]
fn missing_wrong_store_corrupt_oversized_and_wrong_image_receipts_refuse_before_get() {
    let f = Fixture::new();
    let ledger = f.ledger();
    let objects = Arc::new(Objects::default());
    let uploader = RemoteUploader::for_harness(objects.clone());
    let reader = BootstrapReader {
        directory: &f.dir,
        root: &f.root,
        identity: f.identity,
        uploader: &uploader,
    };
    assert!(reader.restore(&f.manifest).is_err());
    before_selection(&f.dir, &f.root, &ledger, None, &f.manifest, 0).unwrap();
    let original = fs::read(path(&f.dir)).unwrap();
    for cut in [0, 8, 64, original.len() - 1] {
        fs::write(path(&f.dir), &original[..cut]).unwrap();
        assert!(reader.restore(&f.manifest).is_err());
    }
    fs::write(path(&f.dir), vec![0; MAX_BYTES + 1]).unwrap();
    assert!(reader.restore(&f.manifest).is_err());
    let mut bad = original.clone();
    bad[48] ^= 1;
    let end = bad.len() - 32;
    let hash = RootDigest::sha256(&bad[..end]);
    bad[end..].copy_from_slice(hash.as_bytes());
    fs::write(path(&f.dir), bad).unwrap();
    assert!(reader.restore(&f.manifest).is_err());
    // A checksum-valid but noncanonical Pending cannot authorize this image.
    let mut bad = original.clone();
    bad[106] ^= 1;
    let end = bad.len() - 32;
    let hash = RootDigest::sha256(&bad[..end]);
    bad[end..].copy_from_slice(hash.as_bytes());
    fs::write(path(&f.dir), bad).unwrap();
    assert!(reader.restore(&f.manifest).is_err());
    fs::write(path(&f.dir), original).unwrap();
    let mut wrong = f.manifest.clone();
    wrong.index += 1;
    assert!(reader.restore(&wrong).is_err());
    assert_eq!(objects.gets.load(Ordering::SeqCst), 0);
    assert!(reader.restore(&f.manifest).is_err()); // Missing SST, authorized GET only.
    assert!(objects.gets.load(Ordering::SeqCst) > 0);
}

#[test]
fn bootstrap_timeout_keeps_the_store_lock_until_the_actual_get_job_drains() {
    let f = Fixture::new();
    let ledger = f.ledger();
    before_selection(&f.dir, &f.root, &ledger, None, &f.manifest, 0).unwrap();
    let (objects, pause) = kv9_engine::MinioObjectStore::with_paused_memory_worker(
        std::time::Duration::from_millis(100),
    )
    .unwrap();
    let uploader = RemoteUploader::new(Arc::new(objects));
    let dir = f.dir.clone();
    let root = f.root.clone();
    let identity = f.identity;
    let manifest = f.manifest.clone();
    let job = std::thread::spawn(move || {
        let _guard = kv9_common::store_lifecycle::StoreGuard::lock(&dir).unwrap();
        BootstrapReader {
            directory: &dir,
            root: &root,
            identity,
            uploader: &uploader,
        }
        .restore(&manifest)
    });
    assert!(pause.wait_for_closed_execution(std::time::Duration::from_secs(5)));
    assert!(!job.is_finished());
    assert!(kv9_common::store_lifecycle::StoreGuard::lock(&f.dir).is_err());
    drop(pause);
    assert!(job.join().unwrap().is_err());
    assert!(kv9_common::store_lifecycle::StoreGuard::lock(&f.dir).is_ok());
    assert!(load(&f.dir, f.identity).unwrap().active.len() == 1);
}

#[test]
fn durable_retirement_replays_after_lost_receipt_and_blocks_further_candidates_or_old_get() {
    let f = Fixture::new();
    let ledger = f.ledger();
    before_selection(&f.dir, &f.root, &ledger, None, &f.manifest, 0).unwrap();
    let mut next = f.manifest.clone();
    next.index += 1;
    ledger.version(&f.root, &next, 1);
    before_selection(&f.dir, &f.root, &ledger, Some(&f.manifest), &next, 1).unwrap();
    ledger.fault.store(4, Ordering::SeqCst);
    assert!(after_selection(&f.dir, &f.root, &ledger, Some(&next)).is_err());
    let saved = fs::read(path(&f.dir)).unwrap();
    let state = load(&f.dir, f.identity).unwrap();
    assert_eq!(state.retiring.len(), 1);
    assert_eq!(state.active.len(), 1);
    assert_eq!(
        fenced_image(&f.dir, f.identity, 0, Some(&next))
            .unwrap()
            .map(|f| f.image),
        Some(image(&f.manifest).unwrap())
    );
    assert!(fenced_image(&f.dir, f.identity, 0, Some(&f.manifest)).is_err());
    let old = bootstrap_reader::binding(&f.root, &f.manifest, 0, f.identity).unwrap();
    assert_eq!(
        ledger.owners.lock().unwrap()[&old.descriptor.id].phase,
        PinPhase::Released
    );
    let objects = Arc::new(Objects::default());
    let uploader = RemoteUploader::for_harness(objects.clone());
    let reader = BootstrapReader {
        directory: &f.dir,
        root: &f.root,
        identity: f.identity,
        uploader: &uploader,
    };
    assert!(reader.restore(&f.manifest).is_err());
    assert_eq!(objects.gets.load(Ordering::SeqCst), 0);
    let mut third = next.clone();
    third.index += 1;
    ledger.version(&f.root, &third, 2);
    assert!(before_selection(&f.dir, &f.root, &ledger, Some(&next), &third, 2).is_err());
    assert_eq!(fs::read(path(&f.dir)).unwrap(), saved);
    let third_owner = bootstrap_reader::binding(&f.root, &third, 2, f.identity).unwrap();
    assert!(!ledger
        .owners
        .lock()
        .unwrap()
        .contains_key(&third_owner.descriptor.id));
    ledger.fault.store(0, Ordering::SeqCst);
    // Fresh call has no in-memory state from the failed operation.
    before_selection(&f.dir, &f.root, &ledger, Some(&next), &third, 2).unwrap();
    assert!(load(&f.dir, f.identity).unwrap().retiring.is_empty());
    assert_eq!(
        ledger.owners.lock().unwrap()[&old.descriptor.id].phase,
        PinPhase::Released
    );
}

#[test]
fn prospective_grant_survives_uncertain_share_and_can_be_fenced_before_any_get() {
    for fault in [5, 6] {
        let f = Fixture::new();
        let ledger = f.ledger();
        ledger.fault.store(fault, Ordering::SeqCst);
        assert!(before_selection(&f.dir, &f.root, &ledger, None, &f.manifest, 0).is_err());
        let saved = load(&f.dir, f.identity).unwrap();
        assert_eq!(saved.active.len(), 1);
        assert!(!saved.active[0].authorized);
        ledger.fault.store(0, Ordering::SeqCst);
        let mut next = f.manifest.clone();
        next.index += 1;
        ledger.version(&f.root, &next, 1);
        before_selection(&f.dir, &f.root, &ledger, None, &next, 1).unwrap();
        let old = bootstrap_reader::binding(&f.root, &f.manifest, 0, f.identity).unwrap();
        assert_eq!(
            ledger.owners.lock().unwrap()[&old.descriptor.id].phase,
            PinPhase::Released
        );
        assert_eq!(load(&f.dir, f.identity).unwrap().active.len(), 1);
    }
}

#[test]
fn v1_upgrade_requires_exact_history_and_retains_existing_grant_identity() {
    let f = Fixture::new();
    let ledger = f.ledger();
    before_selection(&f.dir, &f.root, &ledger, None, &f.manifest, 0).unwrap();
    let mut legacy = fs::read(path(&f.dir)).unwrap()[..64].to_vec();
    legacy[..8].copy_from_slice(b"KV9MBR01");
    legacy.push(1);
    legacy.extend_from_slice(&0u64.to_be_bytes());
    legacy.extend_from_slice(&image(&f.manifest).unwrap());
    legacy.extend_from_slice(RootDigest::sha256(&legacy).as_bytes());
    fs::write(path(&f.dir), &legacy).unwrap();
    assert!(load(&f.dir, f.identity).unwrap().active[0]
        .pending
        .is_none());
    let mut next = f.manifest.clone();
    next.index += 1;
    ledger.version(&f.root, &next, 1);
    ledger.history.lock().unwrap().remove(&0);
    assert!(before_selection(&f.dir, &f.root, &ledger, Some(&f.manifest), &next, 1).is_err());
    assert_eq!(fs::read(path(&f.dir)).unwrap(), legacy);
    ledger.history.lock().unwrap().insert(0, next.clone());
    assert!(before_selection(&f.dir, &f.root, &ledger, Some(&f.manifest), &next, 1).is_err());
    assert_eq!(fs::read(path(&f.dir)).unwrap(), legacy);
    ledger.history.lock().unwrap().insert(0, f.manifest.clone());
    before_selection(&f.dir, &f.root, &ledger, Some(&f.manifest), &next, 1).unwrap();
    assert!(fs::read(path(&f.dir)).unwrap().starts_with(b"KV9MBR02"));
    assert_eq!(
        load(&f.dir, f.identity).unwrap().active[0].pending,
        Some(
            CheckpointIntent::new(&f.root, &f.manifest, 0)
                .unwrap()
                .pending_id()
        )
    );
}

mod startup;

#[test]
fn selected_metadata_receipt_requires_exact_authorization_and_retains_original_window() {
    let f = Fixture::new();
    let digest = image(&f.manifest).unwrap();
    let pending = CheckpointIntent::new(&f.root, &f.manifest, 0)
        .unwrap()
        .pending_id();
    assert!(selected_grant(&f.dir, &f.root, f.identity, &f.manifest)
        .unwrap()
        .is_none());
    let mut state = State {
        active: vec![Grant {
            predecessor: 0,
            image: digest,
            pending: Some(pending),
            authorized: false,
        }],
        retiring: vec![],
    };
    save(&f.dir, f.identity, &state).unwrap();
    assert!(selected_grant(&f.dir, &f.root, f.identity, &f.manifest)
        .unwrap()
        .is_none());
    state.active[0].authorized = true;
    save(&f.dir, f.identity, &state).unwrap();
    let original = selected_grant(&f.dir, &f.root, f.identity, &f.manifest)
        .unwrap()
        .unwrap();
    assert_eq!(original.pending.owner, pending);
    assert_eq!(original.predecessor, 0);
    let mut legacy = fs::read(path(&f.dir)).unwrap()[..64].to_vec();
    legacy[..8].copy_from_slice(b"KV9MBR01");
    legacy.push(1);
    legacy.extend_from_slice(&0u64.to_be_bytes());
    legacy.extend_from_slice(&digest);
    legacy.extend_from_slice(RootDigest::sha256(&legacy).as_bytes());
    fs::write(path(&f.dir), legacy).unwrap();
    assert_eq!(
        selected_grant(&f.dir, &f.root, f.identity, &f.manifest).unwrap(),
        Some(original)
    );
    state.active[0].pending = Some(kv9_common::retention::OwnerId::new([91; 16]).unwrap());
    save(&f.dir, f.identity, &state).unwrap();
    assert!(selected_grant(&f.dir, &f.root, f.identity, &f.manifest).is_err());
    state.active[0].pending = Some(pending);
    state.active[0].authorized = false;
    state.retiring.push(state.active.remove(0));
    save(&f.dir, f.identity, &state).unwrap();
    assert!(selected_grant(&f.dir, &f.root, f.identity, &f.manifest)
        .unwrap()
        .is_none());
}
