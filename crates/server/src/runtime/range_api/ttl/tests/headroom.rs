use super::*;
use kv9_common::log_budget::{headroom::DiskSpaceProbe, LogBudget};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

#[derive(Debug)]
struct Space {
    free: AtomicU64,
    failed: AtomicBool,
}
impl DiskSpaceProbe for Space {
    fn available_bytes(&self) -> std::io::Result<u64> {
        if self.failed.load(Ordering::SeqCst) {
            Err(std::io::Error::other("fixture probe failure"))
        } else {
            Ok(self.free.load(Ordering::SeqCst))
        }
    }
    fn check_path(&self, _: &std::path::Path) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn low_filesystem_headroom_and_probe_errors_refuse_public_ttl_without_losing_control_progress(
) {
    let path = std::env::temp_dir().join(format!(
        "kv9-public-headroom-{}",
        StoreIncarnation::mint().unwrap()
    ));
    std::fs::create_dir_all(&path).unwrap();
    let space = Arc::new(Space {
        free: AtomicU64::new(64 * 1024 * 1024),
        failed: AtomicBool::new(false),
    });
    let budget = Arc::new(
        LogBudget::with_disk_headroom(1024 * 1024 * 1024, 1024 * 1024, space.clone()).unwrap(),
    );
    let f = Fixture::open_with_log_budget(path, Some(Sample::at(1000)), range(), budget.clone());
    f.put(b"kept", b"value", None).await;
    space.free.store(1024 * 1024, Ordering::SeqCst);
    let before = f.group.driver.status().log_last_index;
    for operation in [
        Operation::Put {
            pairs: vec![(b"refused".to_vec(), b"v".to_vec())],
            ttl_secs: Some(10),
        },
        Operation::Get {
            keys: vec![b"kept".to_vec()],
        },
        Operation::Delete {
            key: b"kept".to_vec(),
        },
    ] {
        let error = f.call(operation).await.unwrap_err();
        assert!(matches!(
            crate::client::ttl::classify(&error),
            crate::client::ttl::Failure::Refused {
                refusal: crate::raw_ttl::Refusal::Backpressure
            }
        ));
        assert_eq!(f.group.driver.status().log_last_index, before);
        assert_eq!(f.group.driver.async_apply_snapshot().in_flight, 0);
    }
    f.call(Operation::Capabilities).await.unwrap();
    let control = f.group.driver.propose(&Command::Noop).unwrap();
    assert!(matches!(
        f.group
            .driver
            .wait_applied(control, Duration::from_secs(1))
            .unwrap(),
        ApplyWaitOutcome::Applied(_)
    ));
    space.failed.store(true, Ordering::SeqCst);
    let error = f
        .call(Operation::Delete {
            key: b"kept".to_vec(),
        })
        .await
        .unwrap_err();
    assert!(matches!(
        crate::client::ttl::classify(&error),
        crate::client::ttl::Failure::Refused {
            refusal: crate::raw_ttl::Refusal::Backpressure
        }
    ));
    assert_eq!(budget.snapshot().disk_headroom.unwrap().probe_errors, 1);
    space.failed.store(false, Ordering::SeqCst);
    space.free.store(64 * 1024 * 1024, Ordering::SeqCst);
    assert_eq!(
        f.values(&[b"kept", b"refused"]).await,
        vec![Some(b"value".to_vec()), None]
    );
    assert!(budget
        .snapshot()
        .disk_headroom
        .unwrap()
        .last_probe_error
        .is_none());
    f.close();
}
