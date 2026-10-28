use super::*;
use crate::snapshot_install::adoption::{Binding, Marker};
use kv9_engine::{ObjectKey, ObjectStore};

fn marker_path(f: &Fixture) -> PathBuf {
    f.directory
        .join("data-groups")
        .join(f.range.region.0.to_string())
        .join(ADOPTED)
}
fn legacy(bytes: &[u8]) -> Vec<u8> {
    let mut old = b"KV9ADP01".to_vec();
    old.extend_from_slice(&bytes[8..88]);
    checksum(old)
}
fn binding(bytes: &[u8]) -> Binding {
    Binding::new(
        RootDigest::from_bytes(bytes[8..40].try_into().unwrap()),
        StoreIncarnation::from_bytes(bytes[40..56].try_into().unwrap()),
        RootDigest::from_bytes(bytes[56..88].try_into().unwrap()),
    )
}
fn installed(
    f: &Fixture,
    uploader: &RemoteUploader,
    watermark: Option<ClockWatermark>,
) -> CheckpointManifest {
    let (snapshot, hs) = image(f, uploader, 10, watermark, |_| {});
    let (_, manifest) = parse_image(&snapshot).unwrap();
    f.open().install(&snapshot, &hs, uploader).unwrap();
    manifest
}

#[test]
fn adoption_v2_persists_verified_ttl_presence_and_upgrades_exact_legacy_receipts() {
    for watermark in [None, Some(floor(0)), Some(floor(1000))] {
        let f = Fixture::new();
        let uploader = RemoteUploader::for_harness(Arc::new(MemoryObjectStore::new()));
        installed(&f, &uploader, watermark);
        drop(adopt_for_runtime(&f.guard, f.identity, f.range.region, &uploader).unwrap());
        let path = marker_path(&f);
        let current = fs::read(&path).unwrap();
        assert_eq!(current.len(), 137);
        assert_eq!(&current[..8], b"KV9ADP02");
        assert!(
            matches!(binding(&current).decode(&current).unwrap(), Marker::Verified { watermark: actual } if actual == watermark)
        );
        // Frozen old-reader boundary: the previous implementation bounded the
        // entire read to 120 bytes and compared the exact V1 frame. Both refuse.
        assert!(read(&path, 120).is_err());
        let old = legacy(&current);
        assert_ne!(old, current);
        assert!(matches!(
            binding(&current).decode(&old).unwrap(),
            Marker::Legacy
        ));
        fs::write(&path, old).unwrap(); // Fixture representing a pre-V2 store.
        let adopted = adopt_for_runtime(&f.guard, f.identity, f.range.region, &uploader).unwrap();
        assert_eq!(fs::read(&path).unwrap(), current);
        let mut storage = DiskRaftStorage::recover(&adopted.raft_directory).unwrap();
        let (engine, _, facts) = adopted
            .open_engine_with_dependencies(&mut storage, &uploader)
            .unwrap();
        assert!(facts
            .installed()
            .unwrap()
            .verified_original_during_adoption());
        assert_eq!(facts.selected().unwrap().watermark(), watermark);
        assert!(facts.used_original_install_image());
        assert_eq!(
            engine.get(ColumnFamily::Default, CLOCK_KEY).unwrap(),
            watermark.map(|c| c.encode().to_vec())
        );
        drop(engine);
        drop(storage);
        drop(adopted);
        let adopted = adopt_for_runtime(&f.guard, f.identity, f.range.region, &uploader).unwrap();
        let mut storage = DiskRaftStorage::recover(&adopted.raft_directory).unwrap();
        let (_, _, facts) = adopted
            .open_engine_with_dependencies(&mut storage, &uploader)
            .unwrap();
        assert!(!facts
            .installed()
            .unwrap()
            .verified_original_during_adoption());
        assert!(facts.used_original_install_image());
    }
}

#[test]
fn adoption_receipt_corruption_and_noncanonical_or_foreign_fields_cannot_supply_ttl_facts() {
    for watermark in [None, Some(floor(1000))] {
        let f = Fixture::new();
        let uploader = RemoteUploader::for_harness(Arc::new(MemoryObjectStore::new()));
        installed(&f, &uploader, watermark);
        drop(adopt_for_runtime(&f.guard, f.identity, f.range.region, &uploader).unwrap());
        let original = fs::read(marker_path(&f)).unwrap();
        let expected = binding(&original);
        for end in 0..original.len() {
            assert!(
                expected.decode(&original[..end]).is_err(),
                "truncation {end}"
            );
        }
        for offset in 0..original.len() {
            let mut bad = original.clone();
            bad[offset] ^= 1;
            assert!(expected.decode(&bad).is_err(), "corruption {offset}");
        }
        let mut extra = original.clone();
        extra.push(0);
        assert!(expected.decode(&extra).is_err());
        // Correct checksums still cannot make unknown versions, foreign
        // store/generation/image bindings or noncanonical optional clocks valid.
        for offset in [7, 8, 40, 56, 88, 89] {
            let mut bad = original[..original.len() - 32].to_vec();
            bad[offset] = bad[offset].wrapping_add(2);
            let bad = checksum(bad);
            assert!(expected.decode(&bad).is_err(), "field {offset}");
            fs::write(marker_path(&f), &bad).unwrap();
            assert!(adopt_for_runtime(&f.guard, f.identity, f.range.region, &uploader).is_err());
            assert_eq!(
                fs::read(marker_path(&f)).unwrap(),
                bad,
                "must not repair corrupt receipt"
            );
        }
    }
}

#[test]
fn legacy_adoption_upgrade_requires_original_objects_and_preserves_old_receipt_on_failure() {
    let f = Fixture::new();
    let objects = Arc::new(MemoryObjectStore::new());
    let uploader = RemoteUploader::for_harness(objects.clone());
    let manifest = installed(&f, &uploader, Some(floor(1000)));
    drop(adopt_for_runtime(&f.guard, f.identity, f.range.region, &uploader).unwrap());
    let current = fs::read(marker_path(&f)).unwrap();
    let old = legacy(&current);
    fs::write(marker_path(&f), &old).unwrap();
    let key = ObjectKey::new(manifest.files[0].key.clone()).unwrap();
    let bytes = objects.get(&key).unwrap().unwrap();
    objects.delete(&key).unwrap();
    assert!(adopt_for_runtime(&f.guard, f.identity, f.range.region, &uploader).is_err());
    assert_eq!(fs::read(marker_path(&f)).unwrap(), old);
    objects.put(&key, &bytes).unwrap();
    drop(adopt_for_runtime(&f.guard, f.identity, f.range.region, &uploader).unwrap());
    assert_eq!(fs::read(marker_path(&f)).unwrap(), current);
    // V2 only removes the original closure dependency once a later checkpoint
    // is selected. Losing SSTs that are still the actual selected base refuses.
    objects.delete(&key).unwrap();
    let adopted = adopt_for_runtime(&f.guard, f.identity, f.range.region, &uploader).unwrap();
    let mut storage = DiskRaftStorage::recover(&adopted.raft_directory).unwrap();
    assert!(adopted
        .open_engine_with_dependencies(&mut storage, &uploader)
        .is_err());
}

#[test]
fn adoption_publication_faults_preserve_complete_old_or_new_facts_and_retry() {
    for upgrade in [false, true] {
        for cut in [
            AdoptStep::AfterMarkerFileSync,
            AdoptStep::AfterMarkerRename,
            AdoptStep::AfterMarkerDirectorySync,
        ] {
            let f = Fixture::new();
            let uploader = RemoteUploader::for_harness(Arc::new(MemoryObjectStore::new()));
            installed(&f, &uploader, Some(floor(1000)));
            let before = if upgrade {
                drop(adopt_for_runtime(&f.guard, f.identity, f.range.region, &uploader).unwrap());
                let old = legacy(&fs::read(marker_path(&f)).unwrap());
                fs::write(marker_path(&f), &old).unwrap();
                Some(old)
            } else {
                None
            };
            let mut reached = false;
            let result = adopt_for_runtime_observed(
                &f.guard,
                f.identity,
                f.range.region,
                &uploader,
                |step| {
                    if step == cut {
                        reached = true;
                        return Err(invalid("injected adoption EIO"));
                    }
                    Ok(())
                },
            );
            assert!(result.is_err() && reached, "upgrade={upgrade}, cut={cut:?}");
            drop(result);
            if cut == AdoptStep::AfterMarkerFileSync {
                assert_eq!(fs::read(marker_path(&f)).ok(), before);
            } else {
                let bytes = fs::read(marker_path(&f)).unwrap();
                assert!(
                    matches!(binding(&bytes).decode(&bytes).unwrap(), Marker::Verified { watermark: Some(w) } if w == floor(1000))
                );
                assert!(JointInstaller::open(&f.guard, f.identity, f.range.clone()).is_err());
            }
            let adopted =
                adopt_for_runtime(&f.guard, f.identity, f.range.region, &uploader).unwrap();
            let mut storage = DiskRaftStorage::recover(&adopted.raft_directory).unwrap();
            let (engine, _, _) = adopted.open_engine(&mut storage, &uploader).unwrap();
            assert_eq!(
                engine.get(ColumnFamily::Default, CLOCK_KEY).unwrap(),
                Some(floor(1000).encode().to_vec())
            );
        }
    }
}
