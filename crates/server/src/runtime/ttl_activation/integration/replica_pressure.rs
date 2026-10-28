use super::*;
use crate::runtime::replica_pressure::Maintenance;
use kv9_common::log_budget::{headroom::DiskSpaceProbe, LogBudget};
use kv9_raft::replica_pressure::{PressureRoute, ReplicaPressure};
use std::sync::atomic::AtomicU64;

#[derive(Debug)]
struct Probe {
    free: AtomicU64,
    failed: AtomicBool,
}
impl DiskSpaceProbe for Probe {
    fn available_bytes(&self) -> std::io::Result<u64> {
        if self.failed.load(Ordering::SeqCst) {
            Err(std::io::Error::other("injected disk probe failure"))
        } else {
            Ok(self.free.load(Ordering::SeqCst))
        }
    }
    fn check_path(&self, _: &std::path::Path) -> std::io::Result<()> {
        Ok(())
    }
}
fn cache(f: &Fixture) -> Arc<ReplicaPressure> {
    let ctx = f.backends[0].ttl_context().unwrap();
    let cache = Arc::new(ReplicaPressure::new(ctx.local.identity()));
    cache
        .set_routes(
            ctx.root
                .voters
                .iter()
                .filter(|v| v.node_id != NodeId(1))
                .map(|v| PressureRoute {
                    identity: StoreIdentity::for_voter(&ctx.root, v.node_id).unwrap(),
                    address: v.addr,
                    generation: 0,
                })
                .collect(),
        )
        .unwrap();
    f.data[0].peer().set_replica_pressure(cache.clone());
    cache
}
async fn wait_until(condition: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("pressure observer made no progress");
}
fn put() -> Operation {
    Operation::Put {
        pairs: vec![(b"pressure".to_vec(), b"value".to_vec())],
        ttl_secs: Some(60),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn replica_pressure_poller_stops_public_ttl_on_low_space_follower_and_resumes_on_fresh_health(
) {
    let probes: Vec<_> = (0..3)
        .map(|_| {
            Arc::new(Probe {
                free: AtomicU64::new(1 << 30),
                failed: AtomicBool::new(false),
            })
        })
        .collect();
    let budgets: Vec<_> = probes
        .iter()
        .map(|p| Arc::new(LogBudget::with_disk_headroom(1 << 30, 1024, p.clone()).unwrap()))
        .collect();
    let f = Fixture::with_log_budgets(Some(budgets.clone())).await;
    assert!(matches!(
        f.client(0).activate().await.outcome,
        TtlOutcome::Success { .. }
    ));
    let pressure = cache(&f);
    let monitor = Maintenance::start(
        f.backends[0].clone(),
        pressure.clone(),
        &tokio::runtime::Handle::current(),
    );
    wait_until(|| {
        pressure
            .snapshot()
            .iter()
            .all(|s| s.available_bytes.is_some_and(|b| b > 0))
    })
    .await;
    assert!(matches!(
        f.client(0).call(put()).await.outcome,
        TtlOutcome::Success { .. }
    ));
    probes[2].free.store(1024, Ordering::SeqCst);
    wait_until(|| {
        pressure
            .snapshot()
            .iter()
            .any(|s| s.node == 3 && s.available_bytes == Some(0))
    })
    .await;
    let before = f.data[0].status().log_last_index;
    for transport in [TransportKind::TonicUnary, TransportKind::TonicStream] {
        let client = ScopedTtlClient::with_test_transport(
            f.range.clone(),
            transport,
            f.public[0].clone(),
            5000,
        );
        let report = client.call(put()).await;
        assert!(
            matches!(
                report.outcome,
                TtlOutcome::Refused {
                    failure: crate::client::ttl::Failure::Refused {
                        refusal: crate::raw_ttl::Refusal::Backpressure
                    }
                }
            ),
            "{report:?}"
        );
        assert_eq!(f.data[0].status().log_last_index, before);
        assert_eq!(f.data[0].async_apply_snapshot().in_flight, 0);
    }
    assert!(!pressure.refuses([1, 2], 1)); // other memberships remain writable
    let control = f.data[0].propose(&Command::Noop).unwrap();
    wait_until(|| {
        f.data
            .iter()
            .all(|d| d.status().applied_index >= control.index.0)
    })
    .await;
    let repair = f.meta[1]
        .propose(&Command::Put {
            cf: 0,
            key: b"pressure-repair-diagnostic".to_vec(),
            value: b"metadata write".to_vec(),
        })
        .unwrap();
    wait_until(|| {
        f.meta
            .iter()
            .all(|d| d.status().applied_index >= repair.index.0)
    })
    .await;
    probes[2].failed.store(true, Ordering::SeqCst);
    wait_until(|| {
        pressure
            .snapshot()
            .iter()
            .any(|s| s.node == 3 && s.uncertain)
    })
    .await;
    assert!(pressure.refuses([3], 1));
    // A transport/auth/unsupported error cannot turn the previous negative
    // observation into a healthy report. Gate off the actual store authority.
    f.discoveries[2].raft_receive.store(false, Ordering::SeqCst);
    let errors = pressure
        .snapshot()
        .iter()
        .find(|s| s.node == 3)
        .unwrap()
        .errors;
    probes[2].failed.store(false, Ordering::SeqCst);
    probes[2].free.store(1 << 30, Ordering::SeqCst);
    wait_until(|| {
        pressure
            .snapshot()
            .iter()
            .any(|s| s.node == 3 && s.errors > errors)
    })
    .await;
    assert!(pressure.refuses([3], 1));
    f.discoveries[2].authorize_raft();
    wait_until(|| {
        pressure
            .snapshot()
            .iter()
            .all(|s| s.available_bytes.is_some_and(|b| b > 0))
    })
    .await;
    assert!(matches!(
        f.client(0).call(put()).await.outcome,
        TtlOutcome::Success { .. }
    ));
    assert!(monitor.snapshot().peak <= 2);
    assert!(monitor.snapshot().errors > 0);
    assert!(!monitor.stopped_unexpectedly());
    monitor.drain().await;
    assert_eq!(f.backends[0].maintenance.snapshot().active, 0);
    assert_eq!(f.backends[0].maintenance.snapshot().waiting_classes, 0);
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn replica_pressure_unsupported_provider_stays_unknown_and_poller_drains() {
    let f = Fixture::new().await;
    let pressure = cache(&f);
    let monitor = Maintenance::start(
        f.backends[0].clone(),
        pressure.clone(),
        &tokio::runtime::Handle::current(),
    );
    wait_until(|| pressure.snapshot().iter().all(|s| s.errors > 0)).await;
    assert!(pressure.snapshot().iter().all(|s| !s.observed));
    assert!(!pressure.refuses([2, 3], u64::MAX));
    monitor.drain().await;
    assert_eq!(f.backends[0].maintenance.snapshot().active, 0);
    assert_eq!(f.backends[0].maintenance.snapshot().waiting_classes, 0);
    f.close().await;
}
