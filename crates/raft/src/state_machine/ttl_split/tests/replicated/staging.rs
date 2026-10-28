//! Staging image content and actual three-replica publication/WAL recovery.
use super::*;
use crate::ManifestChangePayload;
use kv9_engine::checkpoint::{CheckpointManifest, RemoteUploader};
use kv9_engine::{MemoryObjectStore, ReplicatedEngine, WriteBatch};

fn setup(path: PathBuf) -> (Group, Vec<ImportRow>) {
    let p = plan();
    let group = Group::open_with(
        path,
        p.low.clone(),
        ClusterId::mint().unwrap(),
        0,
        Some(Arc::new(RemoteUploader::for_harness(Arc::new(
            MemoryObjectStore::new(),
        )))),
    );
    let rows = [b"a", b"b"]
        .into_iter()
        .map(|k| ImportRow {
            key: key(k),
            value: k.to_vec(),
            expiry: Some(
                ExpiryRecord::new(
                    p.parent.creation,
                    AppliedPosition {
                        term: 3,
                        index: 500,
                    },
                    900,
                )
                .unwrap(),
            ),
        })
        .collect::<Vec<_>>();
    // Parent positions deliberately exceed the child's. Cross-group cuts are
    // not ordered; the original identity/deadline must remain unchanged.
    let seal = SplitSeal {
        plan: p.clone(),
        cut: AppliedPosition {
            term: 4,
            index: 1000,
        },
        watermark: ClockSample::with_uncertainty(1000, 0)
            .unwrap()
            .advance(ClockWatermark::default())
            .unwrap(),
    };
    group.submit(
        0,
        &split(RawTtlSplitAction::Begin {
            spec: spec(seal, false, &rows),
            empty_at: group.drivers[0]
                .inspect_empty_ttl_import(&p.low)
                .unwrap()
                .position(),
        }),
    );
    (group, rows)
}
fn identity(group: &Group) -> DataCheckpointIdentity {
    DataCheckpointIdentity {
        cluster: group.cluster,
        root: group.range.root,
        region: group.range.region,
        creation: group.range.creation,
    }
}
fn publish(group: &Group, leader: usize) -> CheckpointManifest {
    let engine = &group.engines[leader];
    let scope = data_group::checkpoint_state(engine.snapshot().unwrap().as_ref(), identity(group))
        .unwrap()
        .1;
    let plan = RemoteUploader::plan(engine.freeze(scope).unwrap()).unwrap();
    let manifest = plan.manifest().clone();
    let prepared = group
        .uploader
        .as_ref()
        .unwrap()
        .upload_planned(plan)
        .unwrap();
    let generation = group.drivers[leader]
        .manifest_pair(group.range.region.0)
        .unwrap()
        .generation;
    let payload = ManifestChangePayload::from_prepared(prepared, generation).unwrap();
    // Historical tag 7 stays a rejection for an unbound import, even after
    // upgrading and replaying. The new semantics have a distinct wire tag.
    assert!(matches!(
        group.submit(leader, &Command::ManifestChange(payload.clone())),
        ApplyWaitOutcome::Manifest {
            verdict: crate::ManifestVerdict::Invalid {
                reason: crate::ManifestInvalidReason::StaleEpoch,
                ..
            },
            ..
        }
    ));
    assert_eq!(
        group.drivers[leader]
            .manifest_pair(group.range.region.0)
            .unwrap()
            .generation,
        generation
    );
    let command = Command::StagingManifestChange(payload);
    assert_eq!(command.encode()[1], 13);
    assert_eq!(
        Command::decode_for_harness(&command.encode()).unwrap(),
        command
    );
    assert!(command.to_write_batch().is_err());
    assert!(matches!(
        group.submit(leader, &command),
        ApplyWaitOutcome::Manifest {
            verdict: crate::ManifestVerdict::Applied { .. },
            ..
        }
    ));
    for engine in &group.engines {
        assert!(engine.checkpoint_applied(&manifest).unwrap());
    }
    manifest
}
fn append(group: &Group, leader: usize, rows: &[ImportRow]) {
    assert!(matches!(
        group.submit(
            leader,
            &split(RawTtlSplitAction::Append {
                target: group.range.clone(),
                expected: state(&group.engines[leader]).digest(),
                rows: rows.to_vec()
            })
        ),
        ApplyWaitOutcome::Applied(_)
    ));
}

#[test]
fn staging_images_at_begin_and_partial_copy_recover_then_validate_append_and_finish_tail() {
    let directory = temp("staging-tail");
    let (group, rows) = setup(directory.join("child"));
    let begun = state(&group.engines[0]);
    publish(&group, 0);
    let group = group.reopen(1);
    assert_eq!(state(&group.engines[1]), begun);
    assert!(read(&group.engines[1], ColumnFamily::Default, RANGE_KEY).is_none());
    // Import checkpoint authority does not enable arbitrary scoped Raw writes.
    assert!(matches!(
        group.submit(
            1,
            &Command::Fenced {
                fence: crate::RegionFence {
                    region_id: group.range.region.0,
                    conf_ver: 1,
                    version: 1
                },
                inner: crate::FencedInner::Write {
                    ops: vec![crate::KvOp::Put {
                        cf: 0,
                        key: key(b"c"),
                        value: b"forbidden".to_vec()
                    }]
                },
            }
        ),
        ApplyWaitOutcome::RawTtlRejected { .. }
    ));
    assert!(read(&group.engines[1], ColumnFamily::Default, &key(b"c")).is_none());
    append(&group, 1, &rows[..1]);
    let manifest = publish(&group, 1);
    let partial = state(&group.engines[1]);
    group.drivers[1]
        .check_ttl_import_checkpoint(&partial)
        .unwrap();
    for fault in 0..4 {
        let mut wrong = partial.clone();
        match fault {
            0 => wrong.spec.seal.plan.operation[0] ^= 1,
            1 => wrong.chain = RootDigest::from_bytes([9; 32]),
            2 => wrong.copied += 1,
            _ => wrong.last = Some(rows[1].key.clone()),
        }
        assert!(
            group.drivers[1]
                .check_ttl_import_checkpoint(&wrong)
                .is_err(),
            "foreign or future checkpoint {fault} accepted"
        );
    }
    append(&group, 1, &rows[1..]);
    group.submit(
        1,
        &split(RawTtlSplitAction::Finish {
            target: group.range.clone(),
            expected: state(&group.engines[1]).digest(),
        }),
    );
    let group = group.reopen(2);
    group.drivers[2]
        .check_ttl_import_checkpoint(&partial)
        .unwrap();
    for engine in &group.engines {
        assert_eq!(
            read(engine, ColumnFamily::Default, RANGE_KEY).unwrap(),
            group.range.encode()
        );
        assert!(state(engine).complete);
        for row in &rows {
            assert_eq!(
                read(engine, ColumnFamily::Write, &row.key),
                row.expiry.map(|e| e.encode().to_vec())
            );
        }
    }
    let image = group.uploader.as_ref().unwrap().restore(&manifest).unwrap();
    let view = image.snapshot().unwrap();
    let observed =
        data_group::inspect_checkpoint(view.as_ref(), &manifest, identity(&group)).unwrap();
    assert_eq!(
        observed.state(),
        &data_group::DataCheckpointState::Import(Box::new(partial))
    );
    assert!(data_group::inspect_base(view.as_ref(), &manifest, identity(&group)).is_err());
    drop(group);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn checksummed_staging_images_reject_wrong_rows_expiry_cursor_count_clock_and_ownership() {
    let directory = temp("staging-corruption");
    let (group, rows) = setup(directory.join("child"));
    append(&group, 0, &rows[..1]);
    let original = group.engines[0].snapshot().unwrap();
    let import = state(&group.engines[0]);
    let scope = data_group::checkpoint_state(original.as_ref(), identity(&group))
        .unwrap()
        .1;
    for fault in 0..10 {
        let (engine, _) = WalEngine::open(directory.join(format!("fault-{fault}"))).unwrap();
        let mut batch = WriteBatch::new();
        for cf in ColumnFamily::ALL {
            for row in original.iter(cf, b"", &[255]).unwrap() {
                let (key, value) = row.unwrap();
                batch.put(cf, key, value);
            }
        }
        match fault {
            0 => {
                batch.put(
                    ColumnFamily::Default,
                    rows[0].key.clone(),
                    b"altered".to_vec(),
                );
            }
            1 => {
                batch.delete(ColumnFamily::Write, rows[0].key.clone());
            }
            2 => {
                batch.put(
                    ColumnFamily::Write,
                    rows[1].key.clone(),
                    rows[1].expiry.unwrap().encode().to_vec(),
                );
            }
            3 => {
                batch.put(
                    ColumnFamily::Default,
                    rows[1].key.clone(),
                    rows[1].value.clone(),
                );
            }
            4 => {
                batch.delete(ColumnFamily::Default, rows[0].key.clone());
                batch.delete(ColumnFamily::Write, rows[0].key.clone());
            }
            5..=7 => {
                let mut altered = import.clone();
                if fault == 5 {
                    altered.chain = RootDigest::from_bytes([9; 32]);
                }
                if fault == 6 {
                    altered.last = Some(rows[1].key.clone());
                }
                if fault == 7 {
                    altered.copied += 1;
                }
                batch.put(ColumnFamily::Default, IMPORT_KEY.to_vec(), altered.encode());
            }
            8 => {
                batch.put(
                    ColumnFamily::Default,
                    CLOCK_KEY.to_vec(),
                    ClockWatermark::default().encode().to_vec(),
                );
            }
            _ => {
                batch.put(
                    ColumnFamily::Default,
                    RANGE_KEY.to_vec(),
                    group.range.encode(),
                );
            }
        }
        ReplicatedEngine::write_applied(&engine, batch, AppliedPosition { term: 1, index: 20 })
            .unwrap();
        let uploader = group.uploader.as_ref().unwrap();
        let prepared = uploader
            .upload(engine.freeze(scope.clone()).unwrap())
            .unwrap();
        let manifest = prepared.into_manifest();
        let restored = uploader.restore(&manifest).unwrap();
        assert!(
            data_group::inspect_checkpoint(
                restored.snapshot().unwrap().as_ref(),
                &manifest,
                identity(&group)
            )
            .is_err(),
            "fault {fault} accepted"
        );
    }
    drop(original);
    drop(group);
    std::fs::remove_dir_all(directory).unwrap();
}
