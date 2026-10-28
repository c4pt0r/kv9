use super::*;
use crate::grpc::{log_pressure::ConfirmedPressure, pb};
use crate::replica_pressure::{PressureRoute, ReplicaPressure};
use kv9_common::{ClusterId, RootDigest, StoreIdentity, StoreIncarnation};
use raft::prelude::{ConfChangeSingle, ConfChangeType, ConfChangeV2, ConfState};

fn identity(node: u64) -> StoreIdentity {
    StoreIdentity {
        cluster_id: ClusterId::from_bytes([1; 16]),
        root_digest: RootDigest::from_bytes([2; 32]),
        node_id: NodeId(node),
        store_incarnation: StoreIncarnation::from_bytes([3; 16]),
    }
}
fn cache() -> Arc<ReplicaPressure> {
    let cache = Arc::new(ReplicaPressure::new(identity(1)));
    cache
        .set_routes(
            (2..=5)
                .map(|node| PressureRoute {
                    identity: identity(node),
                    address: "127.0.0.1:20001".parse().unwrap(),
                    generation: 1,
                })
                .collect(),
        )
        .unwrap();
    cache
}
fn report(cache: &ReplicaPressure, node: u64, capacity: u64) {
    let ticket = cache.begin(NodeId(node)).unwrap().unwrap();
    let id = ticket.route().identity;
    let reply = ConfirmedPressure::from_response(
        &ticket,
        pb::LogPressureResponse {
            root_digest: id.root_digest.as_bytes().to_vec(),
            cluster_id: id.cluster_id.as_bytes().to_vec(),
            node_id: node,
            store_incarnation: id.store_incarnation.as_bytes().to_vec(),
            challenge: ticket.challenge.to_vec(),
            observation: Some(pb::log_pressure_response::Observation::AvailableBytes(
                capacity,
            )),
        },
    )
    .unwrap();
    cache.finish(ticket, Ok(reply));
}
fn pump(drivers: &[Arc<Driver>]) {
    for _ in 0..30 {
        for driver in drivers {
            driver.step().unwrap();
        }
    }
}

#[test]
fn actual_append_refuses_current_replica_without_claim_or_wait_and_removed_member_stops_blocking() {
    let dir = directory();
    let hub = InProcHub::new();
    let budgets: Vec<_> = (0..3)
        .map(|_| Arc::new(LogBudget::new(1 << 30).unwrap()))
        .collect();
    let drivers: Vec<_> = (1..=3)
        .map(|id| {
            open(
                &dir.join(id.to_string()),
                NodeId(id),
                RegionId(9),
                &[1, 2, 3],
                &hub,
                &budgets[id as usize - 1],
            )
            .0
        })
        .collect();
    drivers[0].peer().campaign().unwrap();
    pump(&drivers);
    assert_eq!(drivers[0].status().role, Role::Leader);
    let pressure = cache();
    for driver in &drivers {
        driver.peer().set_replica_pressure(pressure.clone());
    }
    report(&pressure, 4, 0); // unrelated group member has no veto
    drivers[0].propose(&command()).unwrap();
    pump(&drivers);
    report(&pressure, 2, 0);
    let before = drivers[0].status().log_last_index;
    let reserved = budgets[0].snapshot().reserved_bytes;
    assert!(matches!(
        drivers[0].propose_with_async_wait(&command(), Instant::now() + Duration::from_secs(1)),
        Err(Error::WriteBackpressure {
            region: RegionId(9)
        })
    ));
    assert_eq!(drivers[0].status().log_last_index, before);
    assert_eq!(drivers[0].async_apply_snapshot().in_flight, 0);
    assert_eq!(budgets[0].snapshot().reserved_bytes, reserved);
    let control = drivers[0].propose(&Command::Noop).unwrap();
    pump(&drivers);
    for driver in &drivers {
        assert!(matches!(
            driver.wait_applied(control, Duration::ZERO).unwrap(),
            ApplyWaitOutcome::Applied(_)
        ));
    }
    drivers[0]
        .peer()
        .propose_conf_change_traced(ConfChangeV2 {
            changes: vec![ConfChangeSingle {
                node_id: 2,
                change_type: ConfChangeType::RemoveNode,
                ..Default::default()
            }]
            .into(),
            ..Default::default()
        })
        .unwrap();
    pump(&drivers);
    assert!(pressure.refuses([2], 1)); // cached negative is still there
    assert!(!drivers[0]
        .peer()
        .lock()
        .raw
        .raft
        .prs()
        .conf()
        .to_conf_state()
        .voters
        .contains(&2));
    drivers[0].propose(&command()).unwrap();
    pump(&drivers);
    drop(drivers);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn append_gate_uses_incoming_outgoing_learners_and_learners_next() {
    let pressure = cache();
    let conf = ConfState {
        voters: vec![1, 2],
        voters_outgoing: vec![1, 3],
        learners: vec![4],
        learners_next: vec![3],
        ..Default::default()
    };
    let storage = MemStorage::new_with_conf_state(conf);
    let peer = RaftPeer::with_storage(NodeId(1), RegionId(9), storage).unwrap();
    peer.set_replica_pressure(pressure.clone());
    for node in [2, 3, 4] {
        report(&pressure, node, 0);
        assert!(matches!(
            peer.lock().reserve_log_bytes(RegionId(9), 10, true),
            Err(Error::WriteBackpressure { .. })
        ));
        assert!(peer
            .lock()
            .reserve_log_bytes(RegionId(9), 10, false)
            .unwrap()
            .is_none());
        report(&pressure, node, u64::MAX);
    }
    report(&pressure, 5, 0);
    assert!(peer
        .lock()
        .reserve_log_bytes(RegionId(9), 10, true)
        .unwrap()
        .is_none());
}
