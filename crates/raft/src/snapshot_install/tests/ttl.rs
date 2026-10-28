use super::*;
use kv9_common::raw_ttl::{ClockSample, ClockWatermark, ExpiryRecord, CLOCK_KEY};
use kv9_engine::MemoryObjectStore;

fn floor(ms: u64) -> ClockWatermark {
    ClockSample::with_uncertainty(ms, 0)
        .unwrap()
        .advance(ClockWatermark::default())
        .unwrap()
}
fn physical(f: &Fixture, key: &[u8]) -> Vec<u8> {
    encode_key(KeyMode::Raw, f.range.keyspace, key).unwrap()
}
fn expiry(deadline: u64) -> ExpiryRecord {
    // A version inherited from an earlier group may have a larger source
    // index than this image's cut. Transfer must retain all these bytes.
    ExpiryRecord::new(
        RootDigest::sha256(b"previous-source-group"),
        AppliedPosition {
            term: 9,
            index: 500,
        },
        deadline,
    )
    .unwrap()
}
fn image(
    f: &Fixture,
    uploader: &RemoteUploader,
    index: u64,
    watermark: Option<ClockWatermark>,
    mutate: impl FnOnce(&mut WriteBatch),
) -> (Snapshot, HardState) {
    let path = f
        .directory
        .join(format!("ttl-source-{}", StoreIncarnation::mint().unwrap()));
    let (engine, _) = WalEngine::open(path).unwrap();
    let mut batch = WriteBatch::new();
    batch.put(ColumnFamily::Default, RANGE_KEY.to_vec(), f.range.encode());
    for (key, value) in [
        (b"e".as_slice(), b"expired\0\xff".as_slice()),
        (b"l", b"live"),
        (b"p", b""),
    ] {
        batch.put(ColumnFamily::Default, physical(f, key), value.to_vec());
    }
    if let Some(watermark) = watermark {
        batch.put(
            ColumnFamily::Default,
            CLOCK_KEY.to_vec(),
            watermark.encode().to_vec(),
        );
        for (key, expires) in [(b"e", expiry(400)), (b"l", expiry(2000))] {
            batch.put(
                ColumnFamily::Write,
                physical(f, key),
                expires.encode().to_vec(),
            );
        }
    }
    mutate(&mut batch);
    engine
        .write_applied(batch, AppliedPosition { term: 3, index })
        .unwrap();
    let manifest = uploader
        .upload(
            engine
                .freeze(FlushScope {
                    cluster: f.identity.cluster_id.to_string(),
                    region: f.range.region.0,
                    conf_ver: f.range.conf_ver,
                    version: f.range.version,
                })
                .unwrap(),
        )
        .unwrap()
        .into_manifest();
    let mut snapshot = Snapshot {
        data: data_image(&f.range, &manifest).unwrap().into(),
        ..Default::default()
    };
    snapshot.mut_metadata().index = index;
    snapshot.mut_metadata().term = 3;
    snapshot
        .mut_metadata()
        .set_conf_state(ConfState::from((vec![1, 2, 3], vec![4])));
    (
        snapshot,
        HardState {
            term: 5,
            vote: 2,
            commit: index,
            ..Default::default()
        },
    )
}

fn check_image(
    f: &Fixture,
    uploader: &RemoteUploader,
    installed: &InstalledImage,
    expected: ClockWatermark,
) {
    assert_eq!(
        installed.records, 3,
        "count values, not TTL/system metadata"
    );
    assert_eq!(installed.ttl_watermark, Some(expected));
    let path = f.directory.join(format!(
        "data-groups/100/install-{}/data.wal",
        installed.generation
    ));
    let (engine, replay) = WalEngine::open_with_uploader(path, Some(uploader)).unwrap();
    assert_eq!(replay.replayed_records, 0);
    assert_eq!(
        engine.applied_position().unwrap(),
        DurableAppliedPosition::AppliedThrough(installed.position)
    );
    assert_eq!(
        engine
            .get(ColumnFamily::Default, CLOCK_KEY)
            .unwrap()
            .unwrap(),
        expected.encode()
    );
    for (key, record) in [(b"e", expiry(400)), (b"l", expiry(2000))] {
        assert_eq!(
            engine
                .get(ColumnFamily::Write, &physical(f, key))
                .unwrap()
                .unwrap(),
            record.encode()
        );
    }
    assert_eq!(
        engine
            .get(ColumnFamily::Default, &physical(f, b"e"))
            .unwrap(),
        Some(b"expired\0\xff".to_vec())
    );
    assert_eq!(
        engine
            .get(ColumnFamily::Default, &physical(f, b"l"))
            .unwrap(),
        Some(b"live".to_vec())
    );
    assert_eq!(
        engine
            .get(ColumnFamily::Default, &physical(f, b"p"))
            .unwrap(),
        Some(Vec::new())
    );
    assert!(expiry(400).expired_at(expected));
}

#[test]
fn installation_and_adoption_preserve_clock_and_foreign_source_expiry_bytes() {
    let f = Fixture::new();
    let uploader = RemoteUploader::for_harness(Arc::new(MemoryObjectStore::new()));
    let (snapshot, hs) = image(&f, &uploader, 10, Some(floor(1000)), |_| {});
    let mut installer = f.open();
    let installed = installer.install(&snapshot, &hs, &uploader).unwrap();
    check_image(&f, &uploader, &installed, floor(1000));
    assert!(!expiry(2000).expired_at(installed.ttl_watermark.unwrap()));
    drop(installer);
    let mut reopened = f.open();
    assert_eq!(
        reopened.recover(&uploader).unwrap(),
        Some(installed.clone())
    );
    drop(reopened);
    let adopted = adopt_for_runtime(&f.guard, f.identity, f.range.region, &uploader).unwrap();
    assert_eq!(adopted.generation, installed.generation);
    let engine = Arc::new(
        WalEngine::open_with_uploader(&adopted.engine_wal, Some(&uploader))
            .unwrap()
            .0,
    );
    let mut state = crate::MemStateMachine::with_engine(engine).unwrap();
    state
        .set_data_group(f.range.root, f.range.region, f.range.creation)
        .unwrap();
    assert_eq!(
        state.raw_ttl_state().unwrap(),
        (f.range.clone(), floor(1000))
    );
    let result = state
        .apply_at(
            AppliedPosition { term: 5, index: 11 },
            &crate::Command::Put {
                cf: 0,
                key: physical(&f, b"l"),
                value: b"legacy-bypass".to_vec(),
            },
        )
        .unwrap();
    assert!(matches!(
        result.outcome,
        crate::ApplyOutcome::RawTtlRejected { .. }
    ));
    assert_eq!(
        state
            .get(ColumnFamily::Default, &physical(&f, b"l"))
            .unwrap(),
        Some(b"live".to_vec())
    );
    drop(state);
    drop(adopted);
    assert!(JointInstaller::open(&f.guard, f.identity, f.range.clone()).is_err());
}

#[test]
fn newer_cut_cannot_remove_activation_or_roll_back_selected_watermark() {
    for next in [None, Some(floor(999))] {
        let f = Fixture::new();
        let uploader = RemoteUploader::for_harness(Arc::new(MemoryObjectStore::new()));
        let (old, old_hs) = image(&f, &uploader, 10, Some(floor(1000)), |_| {});
        let (new, new_hs) = image(&f, &uploader, 20, next, |_| {});
        let mut installer = f.open();
        let installed = installer.install(&old, &old_hs, &uploader).unwrap();
        let selector = fs::read(installer.directory.join(SELECTOR)).unwrap();
        let error = installer.install(&new, &new_hs, &uploader).unwrap_err();
        assert!(error.to_string().contains("regresses TTL"), "{error}");
        assert_eq!(
            fs::read(installer.directory.join(SELECTOR)).unwrap(),
            selector
        );
        assert!(
            installer.recover(&uploader).is_err(),
            "uncertain owner must reopen"
        );
        drop(installer);
        let mut reopened = f.open();
        assert_eq!(
            reopened.recover(&uploader).unwrap(),
            Some(installed.clone())
        );
        check_image(&f, &uploader, &installed, floor(1000));
    }
    let f = Fixture::new();
    let uploader = RemoteUploader::for_harness(Arc::new(MemoryObjectStore::new()));
    let mut installer = f.open();
    for (index, next) in [
        (5, None),
        (10, Some(floor(1000))),
        (20, Some(floor(1000))),
        (30, Some(floor(1500))),
    ] {
        let (snapshot, hs) = image(&f, &uploader, index, next, |_| {});
        assert_eq!(
            installer
                .install(&snapshot, &hs, &uploader)
                .unwrap()
                .ttl_watermark,
            next
        );
    }
}

#[test]
fn malformed_ttl_or_foreign_cf_images_never_replace_the_selected_generation() {
    for defect in 0..8 {
        let f = Fixture::new();
        let uploader = RemoteUploader::for_harness(Arc::new(MemoryObjectStore::new()));
        let (old, old_hs) = image(&f, &uploader, 10, Some(floor(1000)), |_| {});
        let (bad, bad_hs) = image(&f, &uploader, 20, Some(floor(1500)), |batch| {
            match defect {
                0 => batch.put(
                    ColumnFamily::Default,
                    CLOCK_KEY.to_vec(),
                    b"bad-clock".to_vec(),
                ),
                1 => batch.put(
                    ColumnFamily::Write,
                    physical(&f, b"e"),
                    b"bad-expiry".to_vec(),
                ),
                2 => batch.delete(ColumnFamily::Default, CLOCK_KEY.to_vec()),
                3 => batch.delete(ColumnFamily::Default, physical(&f, b"e")),
                4 => batch.put(
                    ColumnFamily::Write,
                    physical(&f, b"z"),
                    expiry(400).encode().to_vec(),
                ),
                5 => batch.put(
                    ColumnFamily::Lock,
                    physical(&f, b"l"),
                    b"unsupported-lock".to_vec(),
                ),
                6 => batch.put(
                    ColumnFamily::Default,
                    b"s\0\0\0unknown-system".to_vec(),
                    vec![1],
                ),
                _ => {
                    // Deliberately corrupt storage: a generation-one pair without
                    // its corresponding history. Do not widen the private writer API.
                    let mut key = b"\0kv9\0manifest_pair\0".to_vec();
                    key.extend_from_slice(&100_u64.to_be_bytes());
                    let mut value = 1_u64.to_be_bytes().to_vec();
                    value.push(1);
                    batch.put(ColumnFamily::Default, key, value)
                }
            };
        });
        let mut installer = f.open();
        let installed = installer.install(&old, &old_hs, &uploader).unwrap();
        let selected = fs::read(installer.directory.join(SELECTOR)).unwrap();
        assert!(
            installer.install(&bad, &bad_hs, &uploader).is_err(),
            "defect {defect}"
        );
        assert_eq!(
            fs::read(installer.directory.join(SELECTOR)).unwrap(),
            selected
        );
        drop(installer);
        assert_eq!(f.open().recover(&uploader).unwrap(), Some(installed));
    }
}

#[test]
fn every_install_publication_cut_recovers_a_complete_ttl_image() {
    for prior in [false, true] {
        for cut in [
            InstallStep::AfterEngine,
            InstallStep::AfterProtocol,
            InstallStep::BeforeSelector,
            InstallStep::AfterSelectorRename,
            InstallStep::AfterSelectorSync,
        ] {
            let f = Fixture::new();
            let uploader = RemoteUploader::for_harness(Arc::new(MemoryObjectStore::new()));
            let (old, old_hs) = image(&f, &uploader, 10, Some(floor(1000)), |_| {});
            let (new, new_hs) = image(&f, &uploader, 20, Some(floor(2000)), |_| {});
            let mut installer = f.open();
            if prior {
                installer.install(&old, &old_hs, &uploader).unwrap();
            }
            let mut reached = false;
            assert!(installer
                .install_observed(&new, &new_hs, &uploader, &mut |at, _| {
                    if at == cut {
                        reached = true;
                        Err(invalid("injected TTL installation EIO"))
                    } else {
                        Ok(())
                    }
                })
                .is_err());
            assert!(reached);
            assert!(installer.recover(&uploader).is_err());
            drop(installer);
            let mut reopened = f.open();
            let selected = reopened.recover(&uploader).unwrap();
            let new_selected = matches!(
                cut,
                InstallStep::AfterSelectorRename | InstallStep::AfterSelectorSync
            );
            match selected {
                Some(installed) => {
                    assert!(prior || new_selected);
                    assert_eq!(installed.position.index, if new_selected { 20 } else { 10 });
                    check_image(
                        &f,
                        &uploader,
                        &installed,
                        floor(if new_selected { 2000 } else { 1000 }),
                    );
                }
                None => assert!(!prior && !new_selected),
            }
            let completed = reopened.install(&new, &new_hs, &uploader).unwrap();
            check_image(&f, &uploader, &completed, floor(2000));
        }
    }
}

mod adoption;
mod live;
mod recovery;
