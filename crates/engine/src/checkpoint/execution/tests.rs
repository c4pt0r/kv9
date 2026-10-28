use super::*;
use crate::{Engine, MemoryObjectStore, WalEngine};
use std::sync::mpsc;
use std::time::Duration;

fn identity() -> ExecutionIdentity {
    ExecutionIdentity {
        root: kv9_common::RootDigest::from_bytes([1; 32]),
        pending: kv9_common::retention::OwnerId::new([2; 16]).unwrap(),
        executor: kv9_common::retention::OwnerToken {
            owner: kv9_common::retention::OwnerId::new([3; 16]).unwrap(),
            generation: std::num::NonZeroU64::new(1).unwrap(),
        },
    }
}

#[test]
fn read_child_has_its_own_report_and_does_not_drain_or_rebind_the_publisher() {
    let uploader = RemoteUploader::for_harness(Arc::new(MemoryObjectStore::new()));
    let planned = plan("read-child");
    let manifest = planned.manifest().clone();
    uploader.upload_planned(planned).unwrap();
    let mut read_id = identity();
    read_id.executor.owner = kv9_common::retention::OwnerId::new([9; 16]).unwrap();
    let ((), report) = uploader
        .with_local_execution_report(|outer| {
            outer.bind_execution(identity()).unwrap();
            let (restored, read) = outer
                .with_local_read_report(|reader| {
                    assert!(reader.restore(&manifest).is_err());
                    reader.bind_execution(read_id).unwrap();
                    reader.restore(&manifest)
                })
                .unwrap();
            assert_eq!(
                restored.unwrap().get(ColumnFamily::Default, b"k").unwrap(),
                Some(b"value".to_vec())
            );
            assert_eq!(read.identity(), Some(read_id));
            outer
                .store()
                .put(&ObjectKey::new("after-child").unwrap(), b"data")
                .unwrap();
        })
        .unwrap();
    assert_eq!(report.identity(), Some(identity()));
    assert_eq!((report.issued_puts(), report.resolved_puts()), (1, 0));
}

#[test]
fn read_report_waits_for_the_actual_get_job_after_the_caller_timeout() {
    let (store, pause) =
        MinioObjectStore::with_paused_memory_worker(Duration::from_millis(30)).unwrap();
    let uploader = RemoteUploader::new(Arc::new(store));
    let manifest = plan("read-timeout").manifest().clone();
    let (send, receive) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        let result = uploader
            .with_local_read_report(|reader| {
                reader.bind_execution(identity()).unwrap();
                reader.restore(&manifest)
            })
            .unwrap();
        send.send(result).unwrap();
    });
    assert!(pause.wait_for_closed_execution(Duration::from_secs(5)));
    assert!(matches!(receive.try_recv(), Err(mpsc::TryRecvError::Empty)));
    drop(pause);
    let (result, report) = receive.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(result.is_err());
    assert_eq!(report.identity(), Some(identity()));
    thread.join().unwrap();
}

#[test]
fn bound_report_waits_for_actual_queue_discard_and_nested_scope_cannot_report() {
    let (store, pause) =
        MinioObjectStore::with_paused_memory_worker(Duration::from_millis(30)).unwrap();
    let uploader = RemoteUploader::new(Arc::new(store));
    let planned = plan("drain-report-discard");
    let (send, receive) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        let result = uploader
            .with_local_execution_report(|scoped| {
                scoped.bind_execution(identity()).unwrap();
                assert!(scoped.bind_execution(identity()).is_err());
                assert!(scoped.with_local_execution_report(|_| ()).is_err());
                scoped.upload_planned(planned)
            })
            .unwrap();
        send.send(result).unwrap();
    });
    assert!(pause.wait_for_closed_execution(Duration::from_secs(5)));
    assert!(matches!(receive.try_recv(), Err(mpsc::TryRecvError::Empty)));
    drop(pause);
    let (result, report) = receive.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(result.is_err());
    assert_eq!(report.identity(), Some(identity()));
    assert_eq!((report.issued_puts(), report.resolved_puts()), (0, 0));
    thread.join().unwrap();
}

#[test]
fn opaque_backend_success_is_not_terminal_evidence_and_binding_after_put_is_refused() {
    let uploader = RemoteUploader::for_harness(Arc::new(MemoryObjectStore::new()));
    let ((), report) = uploader
        .with_local_execution_report(|scoped| {
            scoped.bind_execution(identity()).unwrap();
            scoped
                .store()
                .put(&ObjectKey::new("opaque").unwrap(), b"data")
                .unwrap();
        })
        .unwrap();
    assert_eq!((report.issued_puts(), report.resolved_puts()), (1, 0));
    uploader
        .with_local_execution_report(|scoped| {
            scoped
                .store()
                .put(&ObjectKey::new("unbound").unwrap(), b"data")
                .unwrap();
            assert!(scoped.bind_execution(identity()).is_err());
        })
        .unwrap();
    assert!(uploader.bind_execution(identity()).is_err());
}

fn plan(name: &str) -> PlannedFlush {
    let dir = std::env::temp_dir().join(format!("kv9-execution-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (engine, _) = WalEngine::open(dir.join("data.wal")).unwrap();
    let mut batch = WriteBatch::new();
    batch.put(ColumnFamily::Default, b"k".to_vec(), b"value".to_vec());
    engine
        .write_applied(batch, AppliedPosition { term: 1, index: 1 })
        .unwrap();
    RemoteUploader::plan(
        engine
            .freeze(FlushScope {
                cluster: "execution".into(),
                region: 100,
                conf_ver: 1,
                version: 1,
            })
            .unwrap(),
    )
    .unwrap()
}

#[test]
fn checkpoint_put_restore_and_prepared_get_wait_for_the_actual_timed_out_job() {
    for mode in 0..3 {
        let planned = plan(&format!("queued-{mode}"));
        let manifest = planned.manifest().clone();
        let (store, pause) =
            MinioObjectStore::with_paused_memory_worker(Duration::from_millis(30)).unwrap();
        let store = Arc::new(store);
        let uploader = Arc::new(RemoteUploader::new(store.clone()));
        let (returned, observe) = mpsc::channel();
        let thread = {
            let uploader = uploader.clone();
            let manifest = manifest.clone();
            std::thread::spawn(move || {
                let result = uploader.with_local_execution(|scoped| match mode {
                    0 => scoped.upload_planned(planned).map(|_| ()),
                    1 => scoped.restore(&manifest).map(|_| ()),
                    _ => scoped.recover_prepared(&manifest).map(|_| ()),
                });
                returned.send(result).unwrap();
            })
        };
        assert!(pause.wait_for_closed_execution(Duration::from_secs(5)));
        assert!(matches!(observe.try_recv(), Err(mpsc::TryRecvError::Empty)));
        // Scope draining is not a global client barrier: unrelated work that
        // submitted no object request can return while this job is still held.
        assert_eq!(uploader.with_local_execution(|_| 42), 42);
        drop(pause);
        assert!(observe
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .is_err());
        thread.join().unwrap();
        assert_eq!(store.object_memory_snapshot().retained_bytes, 0);
        for file in &manifest.files {
            assert!(store
                .get(&ObjectKey::new(file.key.clone()).unwrap())
                .unwrap()
                .is_none());
        }
    }
}

#[test]
fn unwind_drains_a_job_even_after_the_callers_timeout_was_observed() {
    let (store, pause) =
        MinioObjectStore::with_paused_memory_worker(Duration::from_millis(30)).unwrap();
    let uploader = RemoteUploader::new(Arc::new(store));
    let planned = plan("unwind");
    let (finished, observe) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            uploader.with_local_execution(|scoped| {
                assert!(scoped.upload_planned(planned).is_err());
                panic!("injected checkpoint unwind after object timeout");
            });
        }));
        finished.send(outcome.is_err()).unwrap();
    });
    assert!(pause.wait_for_closed_execution(Duration::from_secs(5)));
    assert!(matches!(observe.try_recv(), Err(mpsc::TryRecvError::Empty)));
    drop(pause);
    assert!(observe.recv_timeout(Duration::from_secs(5)).unwrap());
    thread.join().unwrap();
}

#[test]
fn a_full_queue_refusal_does_not_leak_a_scope_lease() {
    let (store, pause) =
        MinioObjectStore::with_paused_memory_worker(Duration::from_millis(10)).unwrap();
    let store = Arc::new(store);
    let uploader = RemoteUploader::new(store.clone());
    let (finished, observe) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        uploader.with_local_execution(|scoped| {
            // One held worker job + the actual 32-entry queue + refusal. Every
            // accepted caller times out, so all admitted leases remain live.
            for n in 0..34 {
                assert!(scoped
                    .store()
                    .put(&ObjectKey::new(format!("full/{n}")).unwrap(), b"v")
                    .is_err());
            }
        });
        finished.send(()).unwrap();
    });
    assert!(pause.wait_for_closed_execution(Duration::from_secs(5)));
    assert_eq!(store.object_memory_snapshot().active_buffers, 33);
    assert!(matches!(observe.try_recv(), Err(mpsc::TryRecvError::Empty)));
    drop(pause);
    observe.recv_timeout(Duration::from_secs(5)).unwrap();
    thread.join().unwrap();
    assert_eq!(store.object_memory_snapshot().retained_bytes, 0);
    assert!(store.list("full/").unwrap().is_empty());
}

#[test]
fn successful_nested_work_drains_jobs_without_waiting_for_backend_payload_ownership() {
    let (store, pause) =
        MinioObjectStore::with_paused_memory_worker(Duration::from_secs(2)).unwrap();
    drop(pause);
    let store = Arc::new(store);
    let uploader = RemoteUploader::new(store.clone());
    let planned = plan("success");
    let expected = planned.manifest().clone();
    uploader.with_local_execution(|scoped| {
        scoped.with_local_execution(|nested| {
            assert!(Arc::ptr_eq(
                scoped.execution.as_ref().unwrap(),
                nested.execution.as_ref().unwrap()
            ));
            let prepared = nested.upload_planned(planned).unwrap();
            assert_eq!(prepared.descriptor(), expected);
            assert_eq!(
                nested
                    .restore(&expected)
                    .unwrap()
                    .get(ColumnFamily::Default, b"k")
                    .unwrap(),
                Some(b"value".to_vec())
            );
            assert_eq!(
                nested.recover_prepared(&expected).unwrap().descriptor(),
                expected
            );
        });
    });
    // Upstream InMemory retains PUT payload bytes after the job is finished.
    assert!(store.object_memory_snapshot().retained_bytes > 0);
    for file in expected.files {
        store.delete(&ObjectKey::new(file.key).unwrap()).unwrap();
    }
    assert_eq!(store.object_memory_snapshot().retained_bytes, 0);
    // The ordinary synchronous harness path also preserves closure errors.
    assert!(
        RemoteUploader::for_harness(Arc::new(MemoryObjectStore::new()))
            .with_local_execution(|s| s.recover_prepared(&plan("absent").manifest))
            .is_err()
    );
}
