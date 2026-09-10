//! Linux syscall-fault probe for the production segmented WAL interfaces.
//!
//! The Python runner owns every directory and runs one owner at a time. This
//! probe simulates checkpoint authority/state only; actual Raft and MinIO
//! certification are separate integration gates. No storage fault hooks are
//! compiled into the library or production server.
use std::path::Path;

use kv9_common::{metrics::WalIoMetrics, AppliedPosition};
use kv9_engine::checkpoint::{CheckpointManifest, FlushScope, RemoteUploader, SstReference};
use kv9_engine::wal_stream::{RecoveryPlan, SegmentedWal};
use kv9_engine::{ColumnFamily, Engine, MemEngine, ReplicatedEngine, WalEngine, WriteBatch};
use serde_json::json;

fn at(index: u64) -> AppliedPosition {
    AppliedPosition { term: 2, index }
}

fn batch(index: u64) -> WriteBatch {
    let mut batch = WriteBatch::new();
    for cf in ColumnFamily::ALL {
        batch.put(
            cf,
            index.to_be_bytes().to_vec(),
            format!("value-{index}").into_bytes(),
        );
    }
    batch
}

fn checkpoint() -> CheckpointManifest {
    let sha256 = "a".repeat(64);
    CheckpointManifest {
        scope: FlushScope {
            cluster: "publication-probe".into(),
            region: 1,
            conf_ver: 1,
            version: 1,
        },
        term: 2,
        index: 7,
        files: vec![SstReference {
            key: format!("clusters/publication-probe/regions/1/sst/{sha256}"),
            sha256,
            cf: 0,
            smallest: vec![0],
            largest: vec![255],
            size: 100,
            count: 2,
        }],
    }
}

fn stream(path: &Path) -> (SegmentedWal, MemEngine) {
    let index = MemEngine::new();
    let (wal, _) = RecoveryPlan::read(path)
        .unwrap()
        .recover(
            4096,
            WalIoMetrics::shared(),
            |manifest| {
                if let Some(manifest) = manifest {
                    assert_eq!(
                        *manifest,
                        checkpoint(),
                        "unexpected simulated checkpoint authority"
                    );
                    for n in [1, 7] {
                        index.write_applied(batch(n), at(n))?;
                    }
                }
                Ok(())
            },
            |batch, at| match at {
                Some(at) => index.write_applied(batch, at),
                None => index.write(batch),
            },
        )
        .unwrap();
    (wal, index)
}

fn values(engine: &dyn Engine) -> Vec<u64> {
    let mut found = Vec::new();
    for n in [1u64, 7, 11, 21] {
        let expected = format!("value-{n}").into_bytes();
        let present = engine
            .get(ColumnFamily::Default, &n.to_be_bytes())
            .unwrap()
            .is_some();
        for cf in ColumnFamily::ALL {
            assert_eq!(
                engine.get(cf, &n.to_be_bytes()).unwrap(),
                present.then(|| expected.clone()),
                "recovery split a cross-column-family batch"
            );
        }
        if present {
            found.push(n);
        }
    }
    found
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(args.len(), 5, "usage: probe seed|attempt|recover rotation|checkpoint|migration|migration-empty directory success|fenced|housekeeping");
    let action = args[1].as_str();
    let operation = args[2].as_str();
    let directory = Path::new(&args[3]);
    let expected = args[4].as_str();
    assert!(matches!(
        operation,
        "rotation" | "checkpoint" | "migration" | "migration-empty" | "migration-checkpoint"
    ));
    let uploader = (operation == "migration-checkpoint").then(|| {
        RemoteUploader::new(std::sync::Arc::new(
            kv9_engine::MinioObjectStore::connect(
                kv9_engine::MinioConfig::from_env().expect("real MinIO required"),
            )
            .unwrap(),
        ))
    });
    let empty = operation == "migration-empty";
    let acknowledged = if empty { Vec::new() } else { vec![1, 7, 11] };
    let mut with_new = acknowledged.clone();
    with_new.push(21);
    let before_expected = if expected == "fenced" {
        &acknowledged
    } else {
        &with_new
    };
    let path = directory.join("catalog.wal");
    match action {
        "seed" => {
            assert!(!path.exists(), "seed cannot overwrite existing authority");
            if operation.starts_with("migration") {
                let (engine, _) = WalEngine::open_with_uploader(&path, uploader.as_ref()).unwrap();
                for &n in &acknowledged {
                    engine.write_applied(batch(n), at(n)).unwrap();
                }
                if let Some(uploader) = &uploader {
                    let manifest = uploader
                        .upload(engine.freeze(checkpoint().scope).unwrap())
                        .unwrap()
                        .into_manifest();
                    assert!(engine.checkpoint_applied(&manifest).unwrap());
                    assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
                }
            } else {
                let mut wal =
                    SegmentedWal::create_new(&path, [71; 16], 4096, WalIoMetrics::shared())
                        .unwrap();
                for n in [1, 7, 11] {
                    wal.append(&batch(n), Some(at(n))).unwrap();
                    if operation == "checkpoint" && n != 11 {
                        wal.rotate().unwrap();
                    }
                }
            }
            println!(
                "{}",
                json!({"action": action, "operation": operation, "acknowledged": acknowledged})
            );
        }
        "attempt" => {
            let (outcome, write) = if operation.starts_with("migration") {
                let (engine, _) = WalEngine::open_with_uploader(&path, uploader.as_ref()).unwrap();
                let outcome = engine.enable_segmentation();
                let write = engine.write_applied(batch(21), at(21));
                if expected == "fenced" {
                    assert!(
                        write.is_err(),
                        "failed publication acknowledged a subsequent write"
                    );
                    assert!(
                        engine.applied_position().is_err(),
                        "ambiguous migration exposed applied authority"
                    );
                    assert!(
                        engine.freeze(checkpoint().scope).is_err(),
                        "ambiguous migration allowed a new freeze"
                    );
                }
                (outcome, write)
            } else {
                let (mut wal, _) = stream(&path);
                let outcome = if operation == "rotation" {
                    wal.rotate()
                } else {
                    wal.checkpoint_applied(&checkpoint()).map(|_| ())
                };
                let write = wal.append(&batch(21), Some(at(21)));
                if expected == "fenced" {
                    assert!(
                        write.is_err(),
                        "failed publication acknowledged a subsequent write"
                    );
                    assert!(
                        wal.rotate().is_err(),
                        "failed publication allowed another rotation"
                    );
                    assert!(
                        wal.checkpoint_applied(&checkpoint()).is_err(),
                        "failed publication allowed another checkpoint"
                    );
                }
                (outcome, write)
            };
            match expected {
                "success" => {
                    assert!(outcome.is_ok(), "baseline operation failed: {outcome:?}");
                    assert!(write.is_ok());
                }
                "fenced" => {
                    assert!(
                        outcome.is_err(),
                        "fault did not reach the selected publication operation"
                    );
                    assert!(
                        write.is_err(),
                        "failed publication acknowledged a subsequent write"
                    );
                }
                "housekeeping" => {
                    assert_eq!(operation, "checkpoint");
                    assert!(
                        outcome.is_err(),
                        "fault did not reach post-publication housekeeping"
                    );
                    assert!(
                        write.is_ok(),
                        "durably adopted checkpoint unnecessarily fenced writes"
                    );
                }
                _ => panic!("invalid expected outcome"),
            }
            println!(
                "{}",
                json!({"action": action, "operation": operation, "expect": expected,
                "operation_ok": outcome.is_ok(), "operation_error": outcome.err().map(|e| e.to_string()), "write_acknowledged": write.is_ok()})
            );
        }
        "recover" => {
            let before = if operation.starts_with("migration") {
                let (engine, _) = WalEngine::open_with_uploader(&path, uploader.as_ref()).unwrap();
                engine.enable_segmentation().unwrap();
                let before = values(&engine);
                assert_eq!(before, *before_expected);
                if expected == "fenced" {
                    engine.write_applied(batch(21), at(21)).unwrap();
                }
                drop(engine);
                let (engine, _) = WalEngine::open_with_uploader(&path, uploader.as_ref()).unwrap();
                assert_eq!(values(&engine), with_new);
                assert_eq!(
                    engine.applied_position().unwrap(),
                    kv9_engine::DurableAppliedPosition::AppliedThrough(at(21))
                );
                drop(engine);
                if empty {
                    let (mut wal, _) = stream(&path);
                    wal.rotate().unwrap();
                    assert!(
                        wal.closed_segments()
                            .iter()
                            .all(|segment| !segment.summary().has_unpositioned),
                        "empty-source migration permanently pinned the stream"
                    );
                }
                before
            } else {
                let (mut wal, index) = stream(&path);
                let before = values(&index);
                assert_eq!(before, *before_expected);
                assert_eq!(wal.applied_position(), Some(at(*before.last().unwrap())));
                if expected == "fenced" {
                    wal.append(&batch(21), Some(at(21))).unwrap();
                }
                drop(wal);
                let (wal, index) = stream(&path);
                assert_eq!(values(&index), with_new);
                assert_eq!(wal.applied_position(), Some(at(21)));
                before
            };
            println!(
                "{}",
                json!({"action": action, "operation": operation, "recovered_before_new_write": before,
                "acknowledged_after_second_reopen": with_new})
            );
        }
        _ => panic!("invalid action"),
    }
}
