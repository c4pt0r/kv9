use super::*;
use crate::store_lifecycle::{StoreGuard, STORE_LIFECYCLE_FILE};
use crate::{BootstrapGeneration, ClusterId, RootDescriptor, RootVoter};

struct Fixture {
    path: PathBuf,
    guard: Option<StoreGuard>,
    identity: StoreIdentity,
}
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "kv9-data-read-fence-{}",
            StoreIncarnation::mint().unwrap()
        ));
        let mut guard = StoreGuard::lock(&path).unwrap();
        let record = guard.prepare(NodeId(1)).unwrap();
        let root = RootDescriptor::new(
            ClusterId::mint().unwrap(),
            BootstrapGeneration::mint().unwrap(),
            vec![RootVoter {
                node_id: record.node_id,
                store_incarnation: record.incarnation,
                addr: "127.0.0.1:20160".parse().unwrap(),
            }],
            b"data-read-fence-test",
        )
        .unwrap();
        let identity = StoreIdentity::for_voter(&root, NodeId(1)).unwrap();
        guard.bind(&root, &identity).unwrap();
        guard.activate(&identity).unwrap();
        Self {
            path,
            guard: Some(guard),
            identity,
        }
    }
    fn floor(&self) -> ReadFloor {
        ReadFloor {
            identity: self.identity,
            creation: RootDigest::from_bytes([4; 32]),
            generation: 1,
            image: ReadImage {
                region: RegionId(100),
                digest: RootDigest::from_bytes([5; 32]),
                cut: AppliedPosition { term: 2, index: 20 },
                conf_ver: 1,
                version: 1,
            },
        }
    }
    fn provider(&self) -> std::sync::Arc<DataReadFences> {
        self.guard
            .as_ref()
            .unwrap()
            .data_read_fences(&self.identity)
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
fn newer(mut f: ReadFloor) -> ReadFloor {
    f.image.digest = RootDigest::from_bytes([6; 32]);
    f.image.cut.index += 10;
    f.generation += 1;
    f
}

#[test]
fn canonical_fixed_floor_rejects_every_truncation_corruption_and_extension() {
    let f = Fixture::new();
    let floor = f.floor();
    let bytes = floor.encode().unwrap();
    assert_eq!(bytes.len(), MAX_BYTES);
    assert_eq!(ReadFloor::decode(&bytes).unwrap(), floor);
    for at in 0..bytes.len() {
        assert!(ReadFloor::decode(&bytes[..at]).is_err());
        let mut damaged = bytes.clone();
        damaged[at] ^= 1;
        assert!(ReadFloor::decode(&damaged).is_err());
    }
    let mut extra = bytes.clone();
    extra.push(0);
    assert!(ReadFloor::decode(&extra).is_err());
    // A valid checksum cannot turn a metadata group or zero identity into DATA.
    for (at, len) in [
        (8, 16),
        (24, 32),
        (56, 8),
        (64, 16),
        (80, 8),
        (88, 32),
        (120, 32),
        (152, 8),
        (160, 8),
        (168, 8),
        (176, 8),
        (184, 8),
    ] {
        let mut damaged = bytes.clone();
        damaged[at..at + len].fill(0);
        let sum = RootDigest::sha256(&damaged[..MAX_BYTES - 32]);
        damaged[MAX_BYTES - 32..].copy_from_slice(sum.as_bytes());
        assert!(ReadFloor::decode(&damaged).is_err(), "zero field at {at}");
    }
}

#[test]
fn monotone_floor_is_store_creation_and_image_exact_with_idempotent_retry() {
    let f = Fixture::new();
    let floors = f.provider();
    let old = f.floor();
    let next = newer(old);
    assert!(floors.floor(old.image.region).unwrap().is_none());
    assert!(floors.advance(old).unwrap());
    assert!(!floors
        .advance_observed(old, &mut |_| panic!("retry wrote a file"))
        .unwrap());
    assert!(floors.advance(next).unwrap());
    assert!(floors.check_read(&old.image).is_err());
    floors.check_read(&next.image).unwrap();
    let mut equal_cut = next.image;
    equal_cut.digest = old.image.digest;
    assert!(floors.check_read(&equal_cut).is_err());
    for bad in [
        old,
        ReadFloor {
            creation: RootDigest::from_bytes([7; 32]),
            ..next
        },
        ReadFloor {
            generation: 1,
            ..next
        },
        ReadFloor {
            image: equal_cut,
            ..next
        },
        ReadFloor {
            identity: StoreIdentity {
                node_id: NodeId(2),
                ..next.identity
            },
            ..next
        },
    ] {
        assert!(floors.advance(bad).is_err());
    }
    assert_eq!(floors.floor(old.image.region).unwrap(), Some(next));
    assert_eq!(
        fs::read(f.path.join(DIRECTORY).join("100")).unwrap(),
        next.encode().unwrap()
    );
}

#[test]
fn each_publication_failure_poison_is_shared_and_reopen_sees_only_a_whole_floor() {
    for step in [
        Step::Create,
        Step::Write,
        Step::SyncFile,
        Step::Rename,
        Step::SyncDirectory,
    ] {
        let mut f = Fixture::new();
        let floors = f.provider();
        let other = f.provider();
        let old = f.floor();
        let next = newer(old);
        assert!(std::sync::Arc::ptr_eq(&floors, &other));
        floors.advance(old).unwrap();
        assert!(floors
            .advance_observed(next, &mut |at| if at == step {
                Err(invalid("injected publication failure"))
            } else {
                Ok(())
            })
            .is_err());
        assert!(floors.floor(old.image.region).is_err());
        assert!(other.check_read(&next.image).is_err());
        assert!(other.advance(next).is_err());
        // Release the exclusive owner only after every provider user stopped.
        drop(floors);
        drop(other);
        drop(f.guard.take());
        f.guard = Some(StoreGuard::lock(&f.path).unwrap());
        let reopened = f.provider();
        let observed = reopened.floor(old.image.region).unwrap().unwrap();
        assert_eq!(
            observed,
            if matches!(step, Step::Rename | Step::SyncDirectory) {
                next
            } else {
                old
            }
        );
        assert!(!reopened.advance(observed).unwrap());
        reopened.advance(next).unwrap();
        assert!(reopened.check_read(&old.image).is_err());
    }
}

#[test]
fn missing_active_directory_never_becomes_an_empty_inventory() {
    let mut f = Fixture::new();
    let floors = f.provider();
    floors.advance(f.floor()).unwrap();
    let lifecycle = fs::read(f.path.join(STORE_LIFECYCLE_FILE)).unwrap();
    fs::remove_dir_all(f.path.join(DIRECTORY)).unwrap();
    assert!(floors.floor(RegionId(100)).is_err());
    assert!(f
        .guard
        .as_ref()
        .unwrap()
        .require_current_writer_format()
        .is_err());
    assert!(f
        .guard
        .as_ref()
        .unwrap()
        .data_read_fences(&f.identity)
        .is_err());
    assert_eq!(
        fs::read(f.path.join(STORE_LIFECYCLE_FILE)).unwrap(),
        lifecycle
    );
    drop(floors);
    drop(f.guard.take());
    assert!(StoreGuard::lock(&f.path).is_err());
    assert!(!f.path.join(DIRECTORY).exists());
}

#[test]
fn failed_first_publication_never_promotes_an_incomplete_temporary() {
    for step in [
        Step::Create,
        Step::Write,
        Step::SyncFile,
        Step::Rename,
        Step::SyncDirectory,
    ] {
        let mut f = Fixture::new();
        let floor = f.floor();
        let provider = f.provider();
        assert!(provider
            .advance_observed(floor, &mut |at| {
                if at == step {
                    Err(invalid("first publication failure"))
                } else {
                    Ok(())
                }
            })
            .is_err());
        assert!(provider.check_read(&floor.image).is_err());
        drop(provider);
        drop(f.guard.take());
        f.guard = Some(StoreGuard::lock(&f.path).unwrap());
        let reopened = f.provider();
        assert_eq!(
            reopened.floor(floor.image.region).unwrap(),
            matches!(step, Step::Rename | Step::SyncDirectory).then_some(floor)
        );
        reopened.advance(floor).unwrap();
        assert_eq!(reopened.floor(floor.image.region).unwrap(), Some(floor));
    }
}

#[test]
fn recovery_requires_same_creation_and_covering_selected_image() {
    let f = Fixture::new();
    let floors = f.provider();
    let old = f.floor();
    let next = newer(old);
    floors.advance(next).unwrap();
    for image in [None, Some(&old.image)] {
        assert!(floors
            .check_recovery(old.image.region, old.creation, image)
            .is_err());
    }
    assert!(floors
        .check_recovery(
            old.image.region,
            RootDigest::from_bytes([8; 32]),
            Some(&next.image)
        )
        .is_err());
    floors
        .check_recovery(old.image.region, old.creation, Some(&next.image))
        .unwrap();
    let mut regressed = newer(next).image;
    regressed.cut.term = 1;
    assert!(floors.check_read(&regressed).is_err());
    regressed.cut.term = next.image.cut.term;
    regressed.version = 0;
    assert!(floors.check_read(&regressed).is_err());
    floors
        .check_recovery(RegionId(101), old.creation, None)
        .unwrap();
}

#[test]
fn invalid_disk_files_cannot_be_interpreted_as_absent() {
    let f = Fixture::new();
    let floors = f.provider();
    let floor = f.floor();
    let path = f.path.join(DIRECTORY).join("100");
    for bytes in [
        vec![],
        vec![0; MAX_BYTES - 1],
        vec![0; MAX_BYTES + 1],
        ReadFloor {
            image: ReadImage {
                region: RegionId(101),
                ..floor.image
            },
            ..floor
        }
        .encode()
        .unwrap(),
        ReadFloor {
            identity: StoreIdentity {
                node_id: NodeId(2),
                ..floor.identity
            },
            ..floor
        }
        .encode()
        .unwrap(),
    ] {
        fs::write(&path, bytes).unwrap();
        assert!(floors.floor(RegionId(100)).is_err());
        assert!(floors.advance(floor).is_err());
    }
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(floors.floor(RegionId(100)).is_err());
}

#[test]
fn a_live_provider_keeps_exclusive_store_ownership_after_guard_drop() {
    let mut f = Fixture::new();
    let floors = f.provider();
    let other = floors.clone();
    drop(f.guard.take());
    assert!(StoreGuard::lock(&f.path).is_err());
    floors.advance(f.floor()).unwrap();
    drop(floors);
    assert!(StoreGuard::lock(&f.path).is_err());
    drop(other);
    f.guard = Some(StoreGuard::lock(&f.path).unwrap());
    assert_eq!(f.provider().floor(RegionId(100)).unwrap(), Some(f.floor()));
}
