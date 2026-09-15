//! Actual three-voter, disk-backed runtime tests through the public gRPC API.
//! Orderly runtime stop/reopen is separate from the required Chaos Mesh gate.
use super::*;
use crate::retention::{RetentionClient, RetentionRpcError};
use kv9_common::retention::{OwnerId, PinPhase, ResourceIdentity, ResourceKind};
use kv9_meta::retention::{LedgerOwner, LedgerRequest, OwnerBinding, OwnerDescriptor, OwnerKind};
use std::num::NonZeroU64;

struct Fixture {
    rts: Vec<NodeRuntime>,
    root: RootDescriptor,
    base: PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.rts.clear();
        if !std::thread::panicking() {
            fs::remove_dir_all(&self.base).unwrap();
        }
    }
}
impl Fixture {
    fn new(tag: &str) -> Self {
        let (rts, root, _, base) = serving_trio(tag);
        Self { rts, root, base }
    }
    fn client(&self) -> RetentionClient {
        let leader = cluster_leader(&self.rts).unwrap();
        RetentionClient::connect(&self.address(leader).to_string(), "acceptance").unwrap()
    }
    fn address(&self, index: usize) -> std::net::SocketAddr {
        self.root
            .voters
            .iter()
            .find(|v| v.node_id == self.rts[index].node.id)
            .unwrap()
            .addr
    }
    fn binding(&self, n: u8) -> OwnerBinding {
        OwnerBinding {
            descriptor: OwnerDescriptor {
                root: self.root.digest(),
                id: OwnerId::new([n; 16]).unwrap(),
                kind: OwnerKind::Snapshot,
                region: 1,
                conf_ver: 1,
                version: 1,
                operation: [n; 32],
                subject: [99; 32],
                subject_is_anchor: true,
                local: None,
            },
            generation: NonZeroU64::new(1).unwrap(),
            resources: (1..=2)
                .map(|n| {
                    ResourceIdentity::new(
                        self.root.digest(),
                        ResourceKind::Sst,
                        [n; 16],
                        [n + 8; 32],
                    )
                    .unwrap()
                })
                .collect(),
        }
    }
    fn initialize(&self, client: &mut RetentionClient) {
        client
            .apply(self.root.digest(), &LedgerRequest::Initialize)
            .unwrap();
        client
            .apply(
                self.root.digest(),
                &LedgerRequest::Register(self.binding(1).resources),
            )
            .unwrap();
    }
    fn all_applied(&mut self, at: AppliedPosition, owners: &[(&OwnerBinding, PinPhase)]) {
        wait_for(&mut self.rts, 10, "all retention replicas applied", |rts| {
            rts.iter().all(|rt| {
                rt.driver
                    .driver_applied()
                    .is_some_and(|p| p.index >= at.index)
            })
        });
        for rt in &self.rts {
            let view = rt.node.meta_raft.store.begin().unwrap().into_view();
            for (binding, phase) in owners {
                let got = kv9_meta::retention::retention_owner(
                    view.as_ref(),
                    &self.root,
                    binding.descriptor.id,
                )
                .unwrap()
                .unwrap();
                assert_eq!(
                    got,
                    LedgerOwner {
                        binding: (*binding).clone(),
                        phase: *phase
                    }
                );
            }
        }
    }
    fn reopen(&mut self) {
        self.rts.clear();
        self.rts = self
            .root
            .voters
            .iter()
            .map(|v| {
                NodeRuntime::start_core(
                    v.node_id,
                    Config {
                        advertise_addr: None,
                        addr: v.addr.to_string(),
                        data_dir: self
                            .base
                            .join(format!("n{}", v.node_id.0))
                            .to_string_lossy()
                            .into_owned(),
                        join: Vec::new(),
                        wal_streams: 1,
                        replication_factor: 3,
                    },
                    RuntimeAuth {
                        cluster_token: "establishing-read-cluster-token".into(),
                        client_tokens: vec![(
                            "acceptance".into(),
                            "establishing-read-token".into(),
                        )],
                    },
                    self.root.clone(),
                    StoreIdentity::for_voter(&self.root, v.node_id).unwrap(),
                    None,
                    StartOverrides::default(),
                )
                .unwrap()
            })
            .collect();
        wait_for(
            &mut self.rts,
            15,
            "retention replicas recovered and Serving",
            |rts| {
                rts.iter()
                    .all(|rt| rt.node.meta.lock().unwrap().bootstrap.is_serving())
            },
        );
    }
}

#[test]
fn retention_rpc_transfer_survives_leader_stop_and_all_store_reopen() {
    let mut f = Fixture::new("retention-rpc-recovery");
    let a = f.binding(1);
    let b = f.binding(2);
    let root = f.root.digest();
    let mut c = f.client();
    f.initialize(&mut c);
    let first = c.apply(root, &LedgerRequest::Acquire(a.clone())).unwrap();
    assert!(first.changed);
    let repeat = c.apply(root, &LedgerRequest::Acquire(a.clone())).unwrap();
    assert!(!repeat.changed);
    assert_eq!(repeat.revision, first.revision);
    assert!(repeat.applied.index > first.applied.index);
    c.apply(root, &LedgerRequest::Publish(a.token())).unwrap();
    assert!(c.apply(root, &LedgerRequest::Release(a.token())).is_err());
    let share = c
        .apply(
            root,
            &LedgerRequest::Share {
                from: a.token(),
                to: b.clone(),
            },
        )
        .unwrap();
    f.all_applied(
        share.applied,
        &[(&a, PinPhase::Published), (&b, PinPhase::Held)],
    );
    assert!(c
        .apply(
            root,
            &LedgerRequest::QuiesceAfterTransfer {
                from: a.token(),
                to: b.token()
            }
        )
        .is_err());
    drop(c);
    let old = cluster_leader(&f.rts).unwrap();
    let old_term = f.rts[old].driver.status().term;
    drop(f.rts.remove(old));
    wait_for(
        &mut f.rts,
        15,
        "retention new leader after old leader stopped",
        |rts| cluster_leader(rts).is_some_and(|i| rts[i].driver.status().term > old_term),
    );
    let mut c = f.client();
    assert_eq!(
        c.get(root, b.descriptor.id).unwrap().unwrap().phase,
        PinPhase::Held
    );
    c.apply(root, &LedgerRequest::Publish(b.token())).unwrap();
    c.apply(
        root,
        &LedgerRequest::QuiesceAfterTransfer {
            from: a.token(),
            to: b.token(),
        },
    )
    .unwrap();
    let released = c.apply(root, &LedgerRequest::Release(a.token())).unwrap();
    f.all_applied(
        released.applied,
        &[(&a, PinPhase::Released), (&b, PinPhase::Published)],
    );
    drop(c);
    f.reopen();
    f.all_applied(
        released.applied,
        &[(&a, PinPhase::Released), (&b, PinPhase::Published)],
    );
    let mut next = a.clone();
    next.generation = NonZeroU64::new(2).unwrap();
    let mut c = f.client();
    c.apply(root, &LedgerRequest::Acquire(next.clone()))
        .unwrap();
    assert!(c.apply(root, &LedgerRequest::Release(a.token())).is_err());
    assert_eq!(
        c.get(root, a.descriptor.id).unwrap().unwrap(),
        LedgerOwner {
            binding: next,
            phase: PinPhase::Held
        }
    );
}

#[test]
fn retention_rpc_concurrent_acquisitions_keep_every_resource_crosslink() {
    let mut f = Fixture::new("retention-rpc-concurrent");
    let root = f.root.digest();
    let mut c = f.client();
    f.initialize(&mut c);
    let leader = cluster_leader(&f.rts).unwrap();
    let address = f.address(leader).to_string();
    let owners: Vec<_> = (1..=8).map(|n| f.binding(n)).collect();
    let gate = Arc::new(std::sync::Barrier::new(owners.len()));
    let mut receipts = std::thread::scope(|scope| {
        let tasks: Vec<_> = owners
            .iter()
            .map(|binding| {
                let gate = gate.clone();
                let address = &address;
                scope.spawn(move || {
                    let mut client = RetentionClient::connect(address, "acceptance").unwrap();
                    gate.wait();
                    client
                        .apply(root, &LedgerRequest::Acquire(binding.clone()))
                        .unwrap()
                })
            })
            .collect();
        tasks
            .into_iter()
            .map(|task| task.join().unwrap())
            .collect::<Vec<_>>()
    });
    receipts.sort_by_key(|r| r.applied.index);
    assert_eq!(
        receipts.iter().map(|r| r.revision).collect::<Vec<_>>(),
        (2..=9).collect::<Vec<_>>()
    );
    assert!(receipts.iter().all(|r| r.changed));
    let expected: Vec<_> = owners.iter().map(|b| (b, PinPhase::Held)).collect();
    f.all_applied(receipts.last().unwrap().applied, &expected);
    let mut rebind = owners[0].clone();
    rebind.descriptor.operation = [200; 32];
    assert!(c.apply(root, &LedgerRequest::Acquire(rebind)).is_err());
    assert_eq!(
        c.get(root, owners[0].descriptor.id)
            .unwrap()
            .unwrap()
            .binding,
        owners[0]
    );
}

#[test]
fn retention_rpc_requires_authentication_exact_root_leader_and_quorum() {
    let mut f = Fixture::new("retention-rpc-quorum");
    let leader = cluster_leader(&f.rts).unwrap();
    let address = f.address(leader).to_string();
    let root = f.root.digest();
    let a = f.binding(1);
    let mut bad = RetentionClient::connect(&address, "invalid-token").unwrap();
    assert!(bad.apply(root, &LedgerRequest::Initialize).is_err());
    let mut c = f.client();
    f.initialize(&mut c);
    assert!(c
        .apply(RootDigest::from_bytes([99; 32]), &LedgerRequest::Initialize)
        .is_err());
    assert!(c
        .get(RootDigest::from_bytes([99; 32]), a.descriptor.id)
        .is_err());
    let follower = (leader + 1) % 3;
    let mut wrong_leader =
        RetentionClient::connect(&f.address(follower).to_string(), "acceptance").unwrap();
    assert!(matches!(
        wrong_leader.apply(root, &LedgerRequest::Acquire(a.clone())),
        Err(RetentionRpcError::NotLeader { .. })
    ));
    let held = c.apply(root, &LedgerRequest::Acquire(a.clone())).unwrap();
    f.all_applied(held.applied, &[(&a, PinPhase::Held)]);
    let survivor = f.rts.remove(leader);
    f.rts.clear();
    f.rts.push(survivor);
    assert!(
        c.apply(root, &LedgerRequest::Publish(a.token())).is_err(),
        "minority acknowledged a retention mutation"
    );
    assert!(
        c.get(root, a.descriptor.id).is_err(),
        "minority served a fresh retention observation"
    );
    let local = f.rts[0].node.meta_raft.store.begin().unwrap().into_view();
    assert_eq!(
        kv9_meta::retention::retention_owner(local.as_ref(), &f.root, a.descriptor.id)
            .unwrap()
            .unwrap()
            .phase,
        PinPhase::Held
    );
}

#[test]
fn retention_planner_drains_an_unconfirmed_acquisition_before_checking_identity() {
    let mut f = Fixture::new("retention-ambiguous-plan");
    let mut c = f.client();
    f.initialize(&mut c);
    let root = f.root.digest();
    let a = f.binding(1);
    let leader = cluster_leader(&f.rts).unwrap();
    let backend = backend_view(&f.rts[leader], &f.root);
    let driver = backend.driver.clone();
    let earlier = {
        let _guard = backend.node.meta_raft.lock_catalog_txn();
        let term = backend.prepare_catalog().unwrap();
        let view = backend.node.meta_raft.store.begin().unwrap().into_view();
        let plan = kv9_meta::retention::plan_retention(
            view.as_ref(),
            &f.root,
            &LedgerRequest::Acquire(a.clone()),
        )
        .unwrap();
        driver.pause_apply(true);
        driver
            .propose_in_term(&Command::from_batch(&plan.into_batch()), term)
            .unwrap()
    };
    wait_for(
        &mut f.rts,
        5,
        "acquisition committed with local apply paused",
        |rts| rts[leader].driver.status().raft_committed >= earlier.index.0,
    );
    assert!(matches!(
        driver.wait_applied(earlier, Duration::from_millis(20)),
        Err(ApplyWaitError::Unconfirmed { .. })
    ));
    let mut conflicting = a.clone();
    conflicting.descriptor.operation = [200; 32];
    let bytes =
        kv9_meta::retention::encode_request(root, &LedgerRequest::Acquire(conflicting)).unwrap();
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let waiter = std::thread::spawn(move || {
        tx.send(backend.apply_retention("acceptance", bytes))
            .unwrap();
    });
    // The new planner holds the catalog mutex while waiting for this barrier;
    // calling step_cluster here would wait on that mutex until its timeout.
    let deadline = Instant::now() + Duration::from_secs(5);
    while driver.status().raft_committed <= earlier.index.0 {
        assert!(driver.status().fatal.is_none());
        assert!(
            Instant::now() < deadline,
            "next retention planner did not append its drain barrier"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let premature = rx.try_recv();
    driver.pause_apply(false);
    assert!(
        premature.is_err(),
        "planner returned before ordered apply: {premature:?}"
    );
    let outcome = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    waiter.join().unwrap();
    assert!(
        matches!(outcome, Err(Error::WriteConflict(_))),
        "ambiguous acquisition was treated as absent: {outcome:?}"
    );
    assert_eq!(c.get(root, a.descriptor.id).unwrap().unwrap().binding, a);
}

#[test]
fn retention_plan_cannot_cross_terms_even_when_its_original_node_leads_again() {
    let f = Fixture::new("retention-term-fence");
    let mut c = f.client();
    f.initialize(&mut c);
    let a = f.binding(1);
    let old = cluster_leader(&f.rts).unwrap();
    let new = (old + 1) % 3;
    let backend = backend_view(&f.rts[old], &f.root);
    let _guard = backend.node.meta_raft.lock_catalog_txn();
    let term = backend.prepare_catalog().unwrap();
    let view = backend.node.meta_raft.store.begin().unwrap().into_view();
    let plan = kv9_meta::retention::plan_retention(
        view.as_ref(),
        &f.root,
        &LedgerRequest::Acquire(a.clone()),
    )
    .unwrap();
    let command = Command::from_batch(&plan.into_batch());
    let old_id = f.rts[old].node.id;
    let new_id = f.rts[new].node.id;
    // The planner deliberately holds the catalog mutex. The generic cluster
    // helper synchronizes catalog routes and would try to reacquire it. Observe
    // the independently running Raft drivers directly with a bounded deadline.
    let wait_leader = |node: NodeId| {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            assert!(f.rts.iter().all(|rt| rt.driver.status().fatal.is_none()));
            if f.rts
                .iter()
                .all(|rt| rt.driver.status().leader_id == Some(node))
                && f.rts
                    .iter()
                    .any(|rt| rt.node.id == node && rt.driver.status().role == Role::Leader)
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "retention test leader transfer did not complete"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    f.rts[old].driver.peer().transfer_leader_for_tests(new_id);
    wait_leader(new_id);
    f.rts[new].driver.peer().transfer_leader_for_tests(old_id);
    wait_leader(old_id);
    assert!(f.rts[old].driver.status().term > term);
    assert!(
        backend.commit_catalog(&command, term).is_err(),
        "old retention plan entered a later term"
    );
    assert!(c.get(f.root.digest(), a.descriptor.id).unwrap().is_none());
}
