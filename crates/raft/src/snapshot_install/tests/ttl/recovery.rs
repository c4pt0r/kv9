use super::*;
use crate::rawnode::PersistentRaftStorage;
use kv9_engine::checkpoint::FlushJournal;
use raft::prelude::Entry;

fn adopt(f: &Fixture, uploader: &RemoteUploader) -> AdoptedGeneration {
    let (snapshot, hs) = image(f, uploader, 10, Some(floor(1000)), |_| {});
    f.open().install(&snapshot, &hs, uploader).unwrap();
    adopt_for_runtime(&f.guard, f.identity, f.range.region, uploader).unwrap()
}

fn append_history(storage: &DiskRaftStorage, last: u64, term: u64) {
    let entries: Vec<_> = (11..=last)
        .map(|index| Entry {
            index,
            term,
            ..Default::default()
        })
        .collect();
    let mut hs = storage.initial_state().unwrap().hard_state;
    hs.commit = last;
    storage.persist_ready(&entries, Some(&hs)).unwrap();
}
fn clock_batch(ms: u64) -> WriteBatch {
    let mut batch = WriteBatch::new();
    batch.put(
        ColumnFamily::Default,
        CLOCK_KEY.to_vec(),
        floor(ms).encode().to_vec(),
    );
    batch
}

#[test]
fn adopted_restart_rejects_uncommitted_wrong_term_and_illegal_intermediate_tail() {
    for fault in 0..10 {
        let f = Fixture::new();
        let uploader = RemoteUploader::for_harness(Arc::new(MemoryObjectStore::new()));
        let adopted = adopt(&f, &uploader);
        let mut storage = DiskRaftStorage::recover(&adopted.raft_directory).unwrap();
        let engine = adopted.open_engine(&mut storage, &uploader).unwrap().0;
        if fault != 0 {
            // Negative controls deliberately attach false effects to real
            // committed positions, without claiming command re-execution.
            append_history(&storage, 13, 5);
        }
        let at = AppliedPosition {
            term: if fault == 1 { 4 } else { 5 },
            index: 11,
        };
        let mut batch = clock_batch(2000);
        match fault {
            0 | 1 => {}
            2 => {
                engine.write(batch.clone()).unwrap();
            }
            3 => {
                batch = clock_batch(999);
            }
            4 => {
                batch.delete(ColumnFamily::Default, CLOCK_KEY.to_vec());
                batch.append(clock_batch(3000));
            }
            5 => {
                let mut foreign = f.range.clone();
                foreign.tenant = TenantId(2);
                batch.put(ColumnFamily::Default, RANGE_KEY.to_vec(), foreign.encode());
                batch.put(ColumnFamily::Default, RANGE_KEY.to_vec(), f.range.encode());
            }
            6 => {
                batch.append(clock_batch(1500));
                batch.append(clock_batch(3000));
            }
            7 => {
                engine.write_applied(batch.clone(), at).unwrap();
                engine
                    .write_applied(clock_batch(1500), AppliedPosition { term: 5, index: 12 })
                    .unwrap();
                engine
                    .write_applied(clock_batch(3000), AppliedPosition { term: 5, index: 13 })
                    .unwrap();
            }
            8 => {
                batch.delete(ColumnFamily::Default, RANGE_KEY.to_vec());
                batch.put(ColumnFamily::Default, RANGE_KEY.to_vec(), f.range.encode());
            }
            9 => {
                let mut sealed = f.range.clone();
                sealed.sealed = true;
                sealed.version += 1;
                batch.put(ColumnFamily::Default, RANGE_KEY.to_vec(), sealed.encode());
            }
            _ => unreachable!(),
        }
        if fault != 2 && fault != 7 {
            engine.write_applied(batch, at).unwrap();
        }
        drop(engine);
        drop(storage);
        drop(adopted);
        let adopted = adopt_for_runtime(&f.guard, f.identity, f.range.region, &uploader).unwrap();
        let mut storage = DiskRaftStorage::recover(&adopted.raft_directory).unwrap();
        let Err(error) = adopted.open_engine(&mut storage, &uploader) else {
            panic!("invalid tail accepted");
        };
        let expected = match fault {
            0 | 1 => "commit",
            2 => "position",
            3 | 6 | 7 => "regresses",
            4 | 8 => "removes",
            5 | 9 => "range transition",
            _ => unreachable!(),
        };
        assert!(
            error.to_string().contains(expected),
            "fault {fault}: {error}"
        );
    }
}

#[test]
fn adoption_exception_is_bound_to_original_selected_image_and_configuration() {
    for fault in 0..6 {
        let f = Fixture::new();
        let uploader = RemoteUploader::for_harness(Arc::new(MemoryObjectStore::new()));
        let adopted = adopt(&f, &uploader);
        let mut storage = DiskRaftStorage::recover(&adopted.raft_directory).unwrap();
        match fault {
            0 => std::fs::remove_file(adopted.engine_wal.with_extension("checkpoint")).unwrap(),
            1..=4 => {
                let cut = if fault == 1 { 10 } else { 20 };
                let watermark = if fault == 3 { 999 } else { 2000 };
                let (snapshot, _) = image(
                    &f,
                    &uploader,
                    cut,
                    (fault != 4).then_some(floor(watermark)),
                    |_| {},
                );
                let (_, manifest) = parse_image(&snapshot).unwrap();
                append_history(&storage, 20, 3);
                if fault == 3 || fault == 4 {
                    // Keep these controls focused on TTL image validation:
                    // the newer selection now needs an actual winning WAL
                    // batch and exact committed command before any SST GET.
                    use crate::state_machine::{manifest_gen_key, manifest_pair_key};
                    use crate::{Command, ManifestChangePayload};
                    let region = f.range.region.0;
                    let change_id = manifest.change_id(0).unwrap();
                    let command = Command::ManifestChange(ManifestChangePayload::for_harness(
                        region,
                        change_id.clone(),
                        0,
                        manifest.encode().unwrap(),
                        manifest.term,
                        manifest.index,
                    ));
                    let mut hs = storage.initial_state().unwrap().hard_state;
                    hs.commit = 21;
                    storage
                        .persist_ready(
                            &[Entry {
                                index: 21,
                                term: 3,
                                data: command.encode().into(),
                                ..Default::default()
                            }],
                            Some(&hs),
                        )
                        .unwrap();
                    let engine = adopted.open_engine(&mut storage, &uploader).unwrap().0;
                    let mut installed = manifest.term.to_be_bytes().to_vec();
                    installed.extend(manifest.index.to_be_bytes());
                    installed.extend(manifest.encode().unwrap());
                    let mut batch = WriteBatch::new();
                    batch.put(
                        ColumnFamily::Default,
                        manifest_gen_key(region, 1),
                        installed,
                    );
                    let mut pair = 1u64.to_be_bytes().to_vec();
                    pair.extend(change_id);
                    batch.put(ColumnFamily::Default, manifest_pair_key(region), pair);
                    engine
                        .write_applied(batch, AppliedPosition { term: 3, index: 21 })
                        .unwrap();
                }
                std::fs::write(
                    adopted.engine_wal.with_extension("checkpoint"),
                    manifest.encode().unwrap(),
                )
                .unwrap();
                if fault == 3 || fault == 4 {
                    let plan = crate::state_machine::checkpoint_recovery::inspect_checkpoint_read(
                        &mut storage,
                        &adopted.engine_wal,
                        f.identity.cluster_id.to_string(),
                        f.range.region.0,
                    )
                    .unwrap();
                    assert_eq!(
                        plan.publication(),
                        Some(AppliedPosition { term: 3, index: 21 })
                    );
                }
            }
            5 => storage
                .set_conf_state(&ConfState::from((vec![1, 2], vec![4])), 0)
                .unwrap(),
            _ => unreachable!(),
        }
        let Err(error) = adopted.open_engine(&mut storage, &uploader) else {
            panic!("invalid selected image accepted");
        };
        let expected = match fault {
            0 => "checkpoint missing",
            1 => "contradicts adopted",
            2 => "publication",
            3 | 4 => "regresses adopted",
            5 => "configuration",
            _ => unreachable!(),
        };
        assert!(
            error.to_string().contains(expected),
            "fault {fault}: {error}"
        );
    }
}

#[test]
fn adopted_restart_restores_pending_history_before_returning_engine() {
    let f = Fixture::new();
    let uploader = RemoteUploader::for_harness(Arc::new(MemoryObjectStore::new()));
    let mut adopted = adopt(&f, &uploader);
    let mut storage = DiskRaftStorage::recover(&adopted.raft_directory).unwrap();
    let engine = adopted.open_engine(&mut storage, &uploader).unwrap().0;
    let (frozen, mut candidate) = engine
        .freeze_checkpoint_candidate(|_| {
            Ok(FlushScope {
                cluster: f.identity.cluster_id.to_string(),
                region: f.range.region.0,
                conf_ver: f.range.conf_ver,
                version: f.range.version,
            })
        })
        .unwrap();
    let plan = RemoteUploader::plan(frozen).unwrap();
    let mut journal = FlushJournal::new(adopted.engine_wal.with_extension("pending"));
    candidate.stage(&mut journal, &plan, 0).unwrap();
    drop(candidate);
    drop(engine);
    // Public diagnostics cannot alter the private recovery anchor.
    adopted.engine_wal = f.directory.join("must-not-create.wal");
    adopted.range.tenant = TenantId(99);
    adopted.cut.index = 999;
    adopted.generation = StoreIncarnation::from_bytes([99; 16]);
    adopted.image_digest = RootDigest::from_bytes([99; 32]);
    let (engine, _, facts) = adopted
        .open_engine_with_dependencies(&mut storage, &uploader)
        .unwrap();
    assert!(facts.selected().unwrap().publication().is_none());
    assert!(facts.used_original_install_image());
    assert_eq!(
        facts.selected().unwrap().cut(),
        AppliedPosition { term: 3, index: 10 }
    );
    let original = facts.installed().unwrap();
    assert_eq!(original.store(), f.identity);
    assert_eq!(original.image().watermark(), Some(floor(1000)));
    assert_ne!(original.generation(), adopted.generation);
    assert_ne!(original.image_record(), adopted.image_digest);
    assert!(original.verified_original_during_adoption());
    assert!(!adopted.engine_wal.exists());
    assert!(engine
        .freeze_checkpoint_candidate(|_| unreachable!("existing pending blocks freeze"))
        .is_err());
    let pending = journal.load().unwrap().unwrap();
    let candidate = engine.claim_checkpoint_candidate(&pending).unwrap();
    assert!(candidate.is_durable());
    assert!(engine.claim_checkpoint_candidate(&pending).is_err());
}
