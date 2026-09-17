use super::*;

struct ActiveCluster {
    base: PathBuf,
    root: RootDescriptor,
    addrs: Vec<std::net::SocketAddr>,
    runtimes: Vec<NodeRuntime>,
}

impl ActiveCluster {
    fn start() -> Self {
        let base = std::env::var_os("KV9_TEST_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join(format!(
                "kv9-group-active-{}",
                StoreIncarnation::mint().unwrap()
            ));
        let listeners: Vec<_> = (0..3).map(|_| bound_listener_for_e2e()).collect();
        let addrs: Vec<_> = listeners.iter().map(|l| l.local_addr().unwrap()).collect();
        let voters = (1..=3)
            .map(|id| kv9_common::RootVoter {
                node_id: NodeId(id),
                addr: addrs[id as usize - 1],
                store_incarnation: prepare_test_store(&base.join(format!("n{id}")), NodeId(id)),
            })
            .collect();
        let root = RootDescriptor::new(
            kv9_common::ClusterId::mint().unwrap(),
            kv9_common::BootstrapGeneration::mint().unwrap(),
            voters,
            b"active-groups",
        )
        .unwrap();
        let mut cluster = Self {
            base,
            root,
            addrs,
            runtimes: Vec::new(),
        };
        for (i, listener) in listeners.into_iter().enumerate() {
            cluster
                .runtimes
                .push(cluster.open(NodeId(i as u64 + 1), listener));
        }
        cluster.wait_serving();
        cluster
    }

    fn open(&self, id: NodeId, listener: std::net::TcpListener) -> NodeRuntime {
        NodeRuntime::start_core(
            id,
            Config {
                advertise_addr: None,
                addr: self.addrs[id.0 as usize - 1].to_string(),
                data_dir: self
                    .base
                    .join(format!("n{}", id.0))
                    .to_string_lossy()
                    .into_owned(),
                join: vec![],
                wal_streams: 1,
                data_workers: 2,
                replication_factor: 3,
            },
            RuntimeAuth {
                cluster_token: "active-groups-cluster".into(),
                client_tokens: vec![("active-test".into(), "active-groups-client".into())],
            },
            self.root.clone(),
            StoreIdentity::for_voter(&self.root, id).unwrap(),
            None,
            StartOverrides {
                listener: Some(listener),
                ..Default::default()
            },
        )
        .unwrap()
    }

    fn wait_serving(&mut self) {
        wait_for(
            &mut self.runtimes,
            30,
            "active fixture metadata serving",
            |rts| rts.iter().all(|r| r.endpoint_ready.load(Ordering::Acquire)),
        );
    }

    fn create(&mut self, operation: u8) -> kv9_meta::data_groups::CreationIntent {
        wait_for(
            &mut self.runtimes,
            20,
            "metadata leader before creation",
            |rts| cluster_leader(rts).is_some(),
        );
        let leader = cluster_leader(&self.runtimes).unwrap();
        let intent = self.runtimes[leader]
            .create_data_group_intent([operation; 16], &[NodeId(1), NodeId(2), NodeId(3)])
            .unwrap();
        wait_for(
            &mut self.runtimes,
            20,
            "creation applied before activation",
            |rts| {
                rts.iter().all(|r| {
                    kv9_meta::data_groups::committed_creation(
                        &r.node.meta_raft.store,
                        intent.task(),
                    )
                    .unwrap()
                    .is_some()
                })
            },
        );
        intent
    }
}

impl Drop for ActiveCluster {
    fn drop(&mut self) {
        self.runtimes.clear();
        if std::thread::panicking() {
            eprintln!("retained active group fixture: {}", self.base.display());
        } else {
            fs::remove_dir_all(&self.base).unwrap();
        }
    }
}

#[test]
fn data_range_public_raw_routes_to_its_own_quorum_and_survives_restart() {
    use crate::data_groups::DataGroupClient;
    use crate::grpc::{RawClient, RawClientOutcome};
    let mut cluster = ActiveCluster::start();
    let first = cluster.create(81);
    let second = cluster.create(82);
    let metadata_leader = cluster_leader(&cluster.runtimes).unwrap();
    let mut admin = DataGroupClient::connect(
        &cluster.runtimes[metadata_leader].addr.to_string(),
        "active-test",
    )
    .unwrap();
    let a = admin
        .create_keyspace(
            cluster.root.digest(),
            first.task(),
            "public-first",
            TenantId::DEFAULT,
        )
        .unwrap();
    let b = admin
        .create_keyspace(
            cluster.root.digest(),
            second.task(),
            "public-second",
            TenantId::DEFAULT,
        )
        .unwrap();
    let retry = admin
        .create_keyspace(
            cluster.root.digest(),
            first.task(),
            "public-first",
            TenantId::DEFAULT,
        )
        .unwrap();
    assert!(!retry.changed);
    assert_eq!(retry.range, a.range);
    assert!(retry.applied.index > a.applied.index);
    let bindings = [a.range, b.range];
    // A pending/nonlocal dispatch entry must never send this namespace through
    // the legacy metadata engine, even with its currently advertised epoch.
    for range in &bindings {
        let node = &cluster.runtimes[metadata_leader].node;
        let txn = node.meta_raft.store.begin().unwrap();
        assert!(matches!(
            check_context_in(
                &node.meta_raft.store,
                &txn,
                range.keyspace,
                &kv9_region::RegionEpoch {
                    conf_ver: 1,
                    version: 1
                },
                KeySpan::Point(b"same-key")
            ),
            Err(Error::MetaNotReady(_))
        ));
        assert!(!kv9_raft::FenceAdjudicator::is_fresh(
            &crate::fence::CatalogFenceAdjudicator::new(node.clone()),
            &kv9_raft::RegionFence {
                region_id: range.region.0,
                conf_ver: 1,
                version: 1
            }
        )
        .unwrap());
    }
    wait_for(
        &mut cluster.runtimes,
        25,
        "public data bindings applied",
        |rts| {
            bindings.iter().all(|r| {
                rts.iter()
                    .all(|rt| rt.data_groups.raw_directory.any_unsealed(r.keyspace).is_some())
                    && data_leader(rts, r.region).is_some()
            })
        },
    );
    for (i, range) in bindings.iter().enumerate() {
        let leader = data_leader(&cluster.runtimes, range.region).unwrap();
        let client = RawClient::connect(
            &cluster.runtimes[leader].addr.to_string(),
            "active-test",
            range.keyspace.0,
        )
        .unwrap();
        let value = vec![i as u8 + 1; 4];
        let RawClientOutcome::Ok(receipt) =
            client.put(b"same-key".to_vec(), value.clone()).unwrap()
        else {
            panic!("data leader refused write")
        };
        assert!(receipt.applied_index > 0 && receipt.applied_term > 0);
        assert!(
            matches!(client.get(b"same-key".to_vec()).unwrap(), RawClientOutcome::Ok(Some(actual)) if actual == value)
        );
        // Exercise both async batch handlers through the actual public service.
        // More than one delete chunk checks the per-group selection/fence path.
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            use crate::grpc::proto;
            let mut peer = proto::kv9_client::Kv9Client::connect(format!(
                "http://{}",
                cluster.runtimes[leader].addr
            ))
            .await
            .unwrap();
            let context = Some(proto::RequestContext {
                keyspace_id: range.keyspace.0,
                region_epoch: Some(proto::RegionEpoch {
                    conf_ver: 1,
                    version: 1,
                }),
            });
            let pairs: Vec<_> = (0..RAW_DELETE_RANGE_CHUNK + 1)
                .map(|n| proto::KeyValue {
                    key: format!("batch-{n:05}").into_bytes(),
                    value: value.clone(),
                })
                .collect();
            let mut put = tonic::Request::new(proto::RawBatchPutRequest {
                context,
                pairs: pairs.clone(),
            });
            put.metadata_mut()
                .insert("authorization", "Bearer active-test".parse().unwrap());
            let applied = peer.raw_batch_put(put).await.unwrap().into_inner();
            assert!(applied.applied_term > 0 && applied.applied_index > receipt.applied_index);
            let mut get = tonic::Request::new(proto::RawBatchGetRequest {
                context,
                keys: pairs.iter().map(|p| p.key.clone()).collect(),
            });
            get.metadata_mut()
                .insert("authorization", "Bearer active-test".parse().unwrap());
            let got = peer.raw_batch_get(get).await.unwrap().into_inner().values;
            assert_eq!(got.len(), pairs.len());
            assert!(got.iter().all(|v| v.found && v.value == value));
        });
        assert!(
            matches!(client.scan(b"batch-".to_vec(), b"batch.".to_vec(), 2048).unwrap(),
            RawClientOutcome::Ok(rows) if rows.len() == RAW_DELETE_RANGE_CHUNK + 1)
        );
        assert!(
            matches!(client.delete_range(b"batch-".to_vec(), b"batch.".to_vec()).unwrap(),
            RawClientOutcome::Ok(progress) if progress.committed_chunks == 2 && progress.last_applied_term > 0)
        );
        assert!(
            matches!(client.scan(b"batch-".to_vec(), b"batch.".to_vec(), 2048).unwrap(),
            RawClientOutcome::Ok(rows) if rows.is_empty())
        );
        assert!(matches!(
            client.put(b"delete-me".to_vec(), vec![7]).unwrap(),
            RawClientOutcome::Ok(_)
        ));
        assert!(matches!(
            client.delete(b"delete-me".to_vec()).unwrap(),
            RawClientOutcome::Ok(_)
        ));
        assert!(matches!(
            client.get(b"delete-me".to_vec()).unwrap(),
            RawClientOutcome::Ok(None)
        ));
        let physical = kv9_common::codec::encode_key(
            kv9_common::codec::KeyMode::Raw,
            range.keyspace,
            b"same-key",
        )
        .unwrap();
        assert!(
            cluster.runtimes.iter().all(|rt| rt
                .node
                .meta_raft
                .store
                .engine()
                .get(kv9_engine::ColumnFamily::Default, &physical)
                .unwrap()
                .is_none()),
            "data leaked into metadata WAL"
        );
        let follower = cluster
            .runtimes
            .iter()
            .position(|rt| rt.data_group_status(range.region).unwrap().role != Role::Leader)
            .unwrap();
        let follower = RawClient::connect(
            &cluster.runtimes[follower].addr.to_string(),
            "active-test",
            range.keyspace.0,
        )
        .unwrap();
        assert!(matches!(
            follower.put(b"forbidden".to_vec(), vec![9]).unwrap(),
            RawClientOutcome::NotLeader { .. }
        ));
    }
    cluster.runtimes.clear();
    for id in 1..=3 {
        let listener = std::net::TcpListener::bind(cluster.addrs[id - 1]).unwrap();
        cluster
            .runtimes
            .push(cluster.open(NodeId(id as u64), listener));
    }
    cluster.wait_serving();
    wait_for(
        &mut cluster.runtimes,
        25,
        "public routes recovered",
        |rts| {
            bindings.iter().all(|r| {
                rts.iter()
                    .all(|rt| rt.data_groups.raw_directory.any_unsealed(r.keyspace).is_some())
                    && data_leader(rts, r.region).is_some()
            })
        },
    );
    for (i, range) in bindings.iter().enumerate() {
        let leader = data_leader(&cluster.runtimes, range.region).unwrap();
        let client = RawClient::connect(
            &cluster.runtimes[leader].addr.to_string(),
            "active-test",
            range.keyspace.0,
        )
        .unwrap();
        assert!(
            matches!(client.get(b"same-key".to_vec()).unwrap(), RawClientOutcome::Ok(Some(actual)) if actual == vec![i as u8 + 1; 4])
        );
    }
    // An already published handle cannot keep authorizing reads/writes after
    // the owning group's ordered seal. Metadata still advertises epoch 1 here.
    let range = &bindings[0];
    let leader = data_leader(&cluster.runtimes, range.region).unwrap();
    let driver = cluster.runtimes[leader]
        .data_groups
        .driver_for_tests(range.region)
        .unwrap();
    let mut sealed = range.clone();
    sealed.version += 1;
    sealed.sealed = true;
    let at = driver
        .propose(&Command::DataRange {
            expected: Some(range.digest()),
            next: sealed,
        })
        .unwrap();
    assert!(matches!(
        driver.wait_applied(at, Duration::from_secs(5)).unwrap(),
        ApplyWaitOutcome::Applied(_)
    ));
    let client = RawClient::connect(
        &cluster.runtimes[leader].addr.to_string(),
        "active-test",
        range.keyspace.0,
    )
    .unwrap();
    assert!(client.get(b"same-key".to_vec()).is_err());
    assert!(client.put(b"after-seal".to_vec(), vec![8]).is_err());
}

fn data_leader(rts: &[NodeRuntime], group: RegionId) -> Option<usize> {
    rts.iter().position(|r| {
        r.data_group_status(group)
            .is_ok_and(|s| s.role == Role::Leader)
    })
}

fn data_write(rt: &NodeRuntime, group: RegionId, key: &[u8], value: &[u8]) {
    let driver = rt.data_groups.driver_for_tests(group).unwrap();
    let at = driver
        .propose(&Command::Put {
            cf: 0,
            key: key.to_vec(),
            value: value.to_vec(),
        })
        .unwrap();
    assert!(
        matches!(
            driver.wait_applied(at, Duration::from_secs(5)),
            Ok(ApplyWaitOutcome::Applied(_))
        ),
        "data write must have its exact applied receipt"
    );
}

fn has_data(rt: &NodeRuntime, group: RegionId, key: &[u8], value: &[u8]) -> bool {
    rt.data_groups.driver_for_tests(group).is_ok_and(|d| {
        d.get(kv9_engine::ColumnFamily::Default, key)
            .unwrap()
            .as_deref()
            == Some(value)
    })
}

#[test]
fn group_activation_runs_independent_durable_groups_and_recovers_all_voters() {
    let mut cluster = ActiveCluster::start();
    let first = cluster.create(71);
    let second = cluster.create(72);
    let groups = [first.region(), second.region()];
    for rt in &mut cluster.runtimes {
        // Dynamic registration on already live listeners; no static group list.
        rt.activate_data_group(first.task()).unwrap();
        rt.activate_data_group(second.task()).unwrap();
        rt.activate_data_group(first.task())
            .expect("activation retries are idempotent");
    }
    for (i, group) in groups.iter().enumerate() {
        cluster.runtimes[i]
            .data_groups
            .driver_for_tests(*group)
            .unwrap()
            .peer()
            .campaign()
            .unwrap();
        wait_for(&mut cluster.runtimes, 20, "data group elected", |rts| {
            data_leader(rts, *group).is_some()
        });
        let mut next_transfer = Instant::now();
        wait_for(
            &mut cluster.runtimes,
            20,
            "different leaders for independent groups",
            |rts| {
                let statuses: Vec<_> = rts
                    .iter()
                    .map(|r| r.data_group_status(*group).unwrap())
                    .collect();
                assert!(
                    statuses.iter().all(|s| s.fatal.is_none()),
                    "data group failed: {statuses:?}"
                );
                if statuses[i].role == Role::Leader {
                    return true;
                }
                // A local Leader observation may belong to an election that
                // is already superseded on another voter. Transfer is best
                // effort; re-address the current leader under one deadline.
                if Instant::now() >= next_transfer {
                    eprintln!("group {} leadership before transfer: {statuses:?}", group.0);
                    if let Some(leader) = data_leader(rts, *group) {
                        rts[leader]
                            .data_groups
                            .driver_for_tests(*group)
                            .unwrap()
                            .peer()
                            .transfer_leader_for_tests(NodeId(i as u64 + 1));
                    }
                    next_transfer = Instant::now() + Duration::from_millis(200);
                }
                false
            },
        );
        data_write(&cluster.runtimes[i], *group, b"same-key", &[i as u8]);
    }
    wait_for(
        &mut cluster.runtimes,
        20,
        "isolated values on every durable replica",
        |rts| {
            rts.iter().all(|rt| {
                groups
                    .iter()
                    .enumerate()
                    .all(|(i, g)| has_data(rt, *g, b"same-key", &[i as u8]))
            })
        },
    );
    for rt in &cluster.runtimes {
        rt.data_groups
            .driver_for_tests(groups[0])
            .unwrap()
            .pause_apply(true);
        assert!(
            rt.node
                .meta_raft
                .store
                .begin()
                .unwrap()
                .get(&kv9_meta::schema::REGIONS_DESC, &[memcmp_uint(groups[0].0)])
                .unwrap()
                .is_none(),
            "activation cannot publish a public range"
        );
    }
    let frozen = cluster.runtimes[0]
        .data_groups
        .driver_for_tests(groups[0])
        .unwrap()
        .propose(&Command::Put {
            cf: 0,
            key: b"paused".to_vec(),
            value: b"resume".to_vec(),
        })
        .unwrap();
    data_write(&cluster.runtimes[1], groups[1], b"during-pause", b"healthy");
    let third = cluster.create(73); // Metadata still commits beside a paused data group.
    for rt in &cluster.runtimes {
        assert!(!has_data(rt, groups[0], b"paused", b"resume"));
        rt.data_groups
            .driver_for_tests(groups[0])
            .unwrap()
            .pause_apply(false);
    }
    assert!(matches!(
        cluster.runtimes[0]
            .data_groups
            .driver_for_tests(groups[0])
            .unwrap()
            .wait_applied(frozen, Duration::from_secs(5)),
        Ok(ApplyWaitOutcome::Applied(_))
    ));

    // Lose the first data leader, retain the majority, and commit on its successor.
    let lost = cluster.runtimes.remove(0).node.id;
    wait_for(
        &mut cluster.runtimes,
        20,
        "surviving data quorum elected",
        |rts| groups.iter().all(|g| data_leader(rts, *g).is_some()),
    );
    for group in groups {
        let leader = data_leader(&cluster.runtimes, group).unwrap();
        data_write(
            &cluster.runtimes[leader],
            group,
            b"after-loss",
            b"committed",
        );
    }
    let listener = std::net::TcpListener::bind(cluster.addrs[lost.0 as usize - 1]).unwrap();
    cluster.runtimes.push(cluster.open(lost, listener));
    cluster.wait_serving();
    wait_for(
        &mut cluster.runtimes,
        20,
        "recovered replica catches up independently",
        |rts| {
            rts.iter().all(|rt| {
                groups.iter().enumerate().all(|(i, g)| {
                    has_data(rt, *g, b"same-key", &[i as u8])
                        && has_data(rt, *g, b"after-loss", b"committed")
                })
            })
        },
    );
    // Stop every voter and reopen every store. No in-memory peer/engine survives.
    cluster.runtimes.clear();
    for id in 1..=3 {
        let listener = std::net::TcpListener::bind(cluster.addrs[id - 1]).unwrap();
        cluster
            .runtimes
            .push(cluster.open(NodeId(id as u64), listener));
    }
    cluster.wait_serving();
    wait_for(
        &mut cluster.runtimes,
        20,
        "all stores recovered acknowledged data",
        |rts| {
            rts.iter().all(|rt| {
                groups.iter().enumerate().all(|(i, g)| {
                    has_data(rt, *g, b"same-key", &[i as u8])
                        && has_data(rt, *g, b"after-loss", b"committed")
                })
            })
        },
    );
    for rt in &cluster.runtimes {
        assert!(
            rt.data_group_status(third.region()).is_err(),
            "unactivated intent must stay inactive"
        );
    }
    let lost_index = cluster
        .runtimes
        .iter()
        .position(|r| r.node.id == NodeId(1))
        .unwrap();
    drop(cluster.runtimes.remove(lost_index));
    let missing = cluster
        .base
        .join("n1/data-groups")
        .join(groups[0].0.to_string())
        .join("raft/raft.log");
    fs::remove_file(&missing).unwrap();
    let listener = std::net::TcpListener::bind(cluster.addrs[0]).unwrap();
    cluster.runtimes.push(cluster.open(NodeId(1), listener));
    cluster.wait_serving();
    let recovered = cluster
        .runtimes
        .iter()
        .find(|r| r.node.id == NodeId(1))
        .unwrap();
    assert!(recovered.data_group_status(groups[0]).is_err());
    assert!(
        !missing.exists(),
        "missing voting log must not be recreated"
    );
    assert!(
        has_data(recovered, groups[1], b"after-loss", b"committed"),
        "bad group prevented healthy group recovery"
    );
    cluster.create(74); // A failed data store must not prevent metadata work.
}

#[test]
fn group_preparation_real_metadata_quorum_survives_leader_loss_and_store_restart() {
    let base = std::env::var_os("KV9_TEST_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!(
            "kv9-group-runtime-{}",
            StoreIncarnation::mint().unwrap()
        ));
    let listeners: Vec<_> = (0..3).map(|_| bound_listener_for_e2e()).collect();
    let addrs: Vec<_> = listeners.iter().map(|l| l.local_addr().unwrap()).collect();
    let voters = (1..=3)
        .map(|id| kv9_common::RootVoter {
            node_id: NodeId(id),
            addr: addrs[(id - 1) as usize],
            store_incarnation: prepare_test_store(&base.join(format!("n{id}")), NodeId(id)),
        })
        .collect();
    let root = RootDescriptor::new(
        kv9_common::ClusterId::mint().unwrap(),
        kv9_common::BootstrapGeneration::mint().unwrap(),
        voters,
        b"group-creation-root",
    )
    .unwrap();
    let auth = || RuntimeAuth {
        cluster_token: "group-creation-cluster-token".into(),
        client_tokens: vec![("group-test".into(), "group-creation-client-token".into())],
    };
    let config = |id: u64| Config {
        advertise_addr: None,
        addr: addrs[(id - 1) as usize].to_string(),
        data_dir: base.join(format!("n{id}")).to_string_lossy().into_owned(),
        join: vec![],
        wal_streams: 1,
        data_workers: 2,
        replication_factor: 3,
    };
    let mut rts: Vec<_> = listeners
        .into_iter()
        .enumerate()
        .map(|(i, listener)| {
            let id = NodeId(i as u64 + 1);
            NodeRuntime::start_core(
                id,
                config(id.0),
                auth(),
                root.clone(),
                StoreIdentity::for_voter(&root, id).unwrap(),
                None,
                StartOverrides {
                    listener: Some(listener),
                    ..Default::default()
                },
            )
            .unwrap()
        })
        .collect();
    wait_for(&mut rts, 30, "group fixture metadata serving", |rts| {
        rts.iter()
            .all(|rt| rt.node.meta.lock().unwrap().bootstrap.is_serving())
    });
    let leader = cluster_leader(&rts).unwrap();
    let nodes = [NodeId(1), NodeId(2), NodeId(3)];
    let first = rts[leader]
        .create_data_group_intent([61; 16], &nodes)
        .unwrap();
    let second = rts[leader]
        .create_data_group_intent([62; 16], &nodes)
        .unwrap();
    assert_ne!(first.region(), second.region());
    wait_for(
        &mut rts,
        20,
        "both creation intents applied everywhere",
        |rts| {
            rts.iter().all(|rt| {
                kv9_meta::data_groups::committed_creation(&rt.node.meta_raft.store, second.task())
                    .unwrap()
                    .is_some()
            })
        },
    );
    for rt in &mut rts {
        assert_eq!(
            rt.prepare_data_group(first.task()).unwrap().intent_digest,
            first.digest()
        );
        assert_eq!(
            rt.prepare_data_group(second.task()).unwrap().intent_digest,
            second.digest()
        );
        assert_eq!(rt.data_group_preparations().len(), 2);
        assert!(
            rt.node
                .meta_raft
                .store
                .begin()
                .unwrap()
                .get(
                    &kv9_meta::schema::REGIONS_DESC,
                    &[memcmp_uint(first.region().0)]
                )
                .unwrap()
                .is_none(),
            "prepared group must not be publicly routable"
        );
    }
    let lost = rts[leader].node.id;
    drop(rts.remove(leader));
    wait_for(&mut rts, 30, "replacement metadata leader", |rts| {
        cluster_leader(rts).is_some()
    });
    let replacement = cluster_leader(&rts).unwrap();
    assert_eq!(
        rts[replacement]
            .create_data_group_intent([61; 16], &nodes)
            .unwrap(),
        first,
        "a replacement coordinator must resume the same creation intent"
    );
    let listener = std::net::TcpListener::bind(addrs[(lost.0 - 1) as usize]).unwrap();
    let recovered = NodeRuntime::start_core(
        lost,
        config(lost.0),
        auth(),
        root.clone(),
        StoreIdentity::for_voter(&root, lost).unwrap(),
        None,
        StartOverrides {
            listener: Some(listener),
            ..Default::default()
        },
    )
    .unwrap();
    let observations = recovered.data_group_preparations();
    assert_eq!(
        observations.len(),
        2,
        "restart must discover both durable group preparations"
    );
    assert!(
        observations.iter().all(|(_, r)| r.is_ok()),
        "both independent groups must recover their exact metadata bindings"
    );
    rts.push(recovered);
    wait_for(&mut rts, 30, "recovered metadata voter serving", |rts| {
        rts.iter()
            .all(|rt| rt.node.meta.lock().unwrap().bootstrap.is_serving())
    });
    drop(rts);
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn routed_client_keeps_its_lifetime_across_each_endpoint_loss() {
    use crate::client::routed::{RoutedConfig, RoutedOutcome, RoutedRawClient};
    use crate::client::{Peer, RawOperation, Value};
    let mut cluster = ActiveCluster::start();
    let intent = cluster.create(93);
    let metadata_leader = cluster_leader(&cluster.runtimes).unwrap();
    let binding = crate::data_groups::DataGroupClient::connect(
        &cluster.runtimes[metadata_leader].addr.to_string(),
        "active-test",
    )
    .unwrap()
    .create_keyspace(
        cluster.root.digest(),
        intent.task(),
        "routed-client",
        TenantId::DEFAULT,
    )
    .unwrap()
    .range;
    wait_for(&mut cluster.runtimes, 25, "routed group ready", |rts| {
        rts.iter()
            .all(|r| r.data_groups.raw_directory.any_unsealed(binding.keyspace).is_some())
            && data_leader(rts, binding.region).is_some()
    });
    let executor = tokio::runtime::Runtime::new().unwrap();
    let config = RoutedConfig {
        version: 1,
        root_digest: *cluster.root.digest().as_bytes(),
        tenant_id: binding.tenant.0,
        keyspace_id: binding.keyspace.0,
        seeds: cluster
            .addrs
            .iter()
            .enumerate()
            .map(|(i, address)| Peer {
                node_id: i as u64 + 1,
                address: *address,
            })
            .collect(),
        max_in_flight: 4,
        max_attempts: 16,
        deadline_ms: 5000,
        probe_timeout_ms: 300,
        // The 16-attempt budget must span an election, including immediately
        // refused TCP connects and stale follower hints during leader loss.
        retry_backoff_ms: 150,
        cache_capacity: 8,
    };
    let client = executor.block_on(async { RoutedRawClient::new(config, "active-test").unwrap() });
    let first = executor.block_on(client.call(RawOperation::Put {
        key: b"persist".to_vec(),
        value: b"before".to_vec(),
    }));
    assert!(
        matches!(first.outcome, RoutedOutcome::Success { .. }),
        "{first:?}"
    );
    let batch = executor.block_on(client.call(RawOperation::BatchPut {
        pairs: vec![
            (b"batch-a".to_vec(), vec![1]),
            (b"batch-b".to_vec(), vec![2]),
        ],
    }));
    assert!(
        matches!(batch.outcome, RoutedOutcome::Success { .. }),
        "{batch:?}"
    );
    let batch = executor.block_on(client.call(RawOperation::BatchGet {
        keys: vec![
            b"batch-b".to_vec(),
            b"batch-a".to_vec(),
            b"batch-b".to_vec(),
        ],
    }));
    assert!(
        matches!(batch.outcome, RoutedOutcome::Success { value: Value::BatchGet { ref values } } if values == &vec![Some(vec![2]), Some(vec![1]), Some(vec![2])]),
        "{batch:?}"
    );
    for victim in 1..=3 {
        let position = cluster
            .runtimes
            .iter()
            .position(|r| r.node.id == NodeId(victim))
            .unwrap();
        drop(cluster.runtimes.remove(position));
        // No harness leader selection, client reconstruction or fixed seed retry.
        let read = executor.block_on(client.call(RawOperation::Get {
            key: b"persist".to_vec(),
        }));
        assert!(
            matches!(read.outcome, RoutedOutcome::Success { value: Value::Get { value: Some(ref v) } } if v == b"before"),
            "victim={victim}: {read:?}"
        );
        let write = executor.block_on(client.call(RawOperation::Put {
            key: format!("after-{victim}").into_bytes(),
            value: vec![victim as u8],
        }));
        assert!(
            matches!(write.outcome, RoutedOutcome::Success { .. }),
            "victim={victim}: {write:?}"
        );
        assert!(read.attempts.len() <= 16 && write.attempts.len() <= 16);
        let listener = std::net::TcpListener::bind(cluster.addrs[victim as usize - 1]).unwrap();
        cluster
            .runtimes
            .push(cluster.open(NodeId(victim), listener));
        cluster.wait_serving();
        wait_for(&mut cluster.runtimes, 25, "returned routed owner", |rts| {
            rts.iter()
                .all(|r| r.data_groups.raw_directory.any_unsealed(binding.keyspace).is_some())
        });
    }
    // A wrong scope must fail at the actual data endpoint, even if a caller
    // bypasses directory discovery and supplies a valid-looking epoch.
    let leader = data_leader(&cluster.runtimes, binding.region).unwrap();
    executor.block_on(async {
        let mut peer = crate::proto::kv9_client::Kv9Client::connect(format!(
            "http://{}",
            cluster.runtimes[leader].addr
        ))
        .await
        .unwrap();
        for mutate in 0..3 {
            let mut bad = binding.clone();
            match mutate {
                0 => bad.root = RootDigest::from_bytes([71; 32]),
                1 => bad.tenant = TenantId(7171),
                _ => bad.creation = RootDigest::from_bytes([72; 32]),
            }
            let mut request = tonic::Request::new(crate::proto::RoutedRawRequest {
                binding: bad.encode(),
                operation: Some(crate::proto::routed_raw_request::Operation::Put(
                    crate::proto::RoutedPairs {
                        pairs: vec![crate::proto::KeyValue {
                            key: b"forbidden".to_vec(),
                            value: vec![9],
                        }],
                    },
                )),
            });
            request
                .metadata_mut()
                .insert("authorization", "Bearer active-test".parse().unwrap());
            let status = peer.routed_raw(request).await.unwrap_err();
            assert_eq!(status.metadata().get("kv9-route-refused").unwrap(), "scope");
        }
    });
    let read = executor.block_on(client.call(RawOperation::Get {
        key: b"forbidden".to_vec(),
    }));
    assert!(
        matches!(
            read.outcome,
            RoutedOutcome::Success {
                value: Value::Get { value: None }
            }
        ),
        "{read:?}"
    );
    drop(client);
    drop(executor);
}

#[test]
fn migration_intents_bind_committed_image_owners_only() {
    use crate::data_groups::DataGroupClient;
    use kv9_common::retention::PinPhase;
    use kv9_engine::checkpoint::{CheckpointManifest, FlushScope, SstReference};
    use kv9_meta::retention::OwnerKind;

    let mut cluster = ActiveCluster::start();
    let root = cluster.root.digest();
    let leader = cluster_leader(&cluster.runtimes).unwrap();
    let mut admin =
        DataGroupClient::connect(&cluster.runtimes[leader].addr.to_string(), "active-test")
            .unwrap();
    let creation = admin
        .create(root, [91; 16], &[NodeId(1), NodeId(2), NodeId(3)])
        .unwrap()
        .intent;

    // A migration destination must be a genuinely admitted, registered store
    // outside the initial replica set: run the production join flow for node 4.
    let listener = bound_listener_for_e2e();
    let addr4 = listener.local_addr().unwrap();
    let admitted = backend_view(&cluster.runtimes[leader], &cluster.root)
        .admit_node("active-test", NodeId(4), &addr4.to_string(), 600)
        .unwrap();
    let ticket = admitted.join_ticket.unwrap();
    let incarnation4 = prepare_test_store(&cluster.base.join("n4"), NodeId(4));
    let identity4 = StoreIdentity::for_joiner(&cluster.root, NodeId(4), incarnation4).unwrap();
    let node4 = NodeRuntime::start_core(
        NodeId(4),
        Config {
            advertise_addr: None,
            addr: addr4.to_string(),
            data_dir: cluster.base.join("n4").to_string_lossy().into_owned(),
            join: vec![],
            wal_streams: 1,
            data_workers: 2,
            replication_factor: 3,
        },
        RuntimeAuth {
            cluster_token: "active-groups-cluster".into(),
            client_tokens: vec![("active-test".into(), "active-groups-client".into())],
        },
        cluster.root.clone(),
        identity4,
        Some(&ticket),
        StartOverrides {
            listener: Some(listener),
            ..Default::default()
        },
    )
    .unwrap();
    cluster.runtimes.push(node4);
    wait_for(
        &mut cluster.runtimes,
        45,
        "joiner endpoint active before migration",
        |rts| {
            rts[..3].iter().all(|rt| {
                let txn = rt.node.meta_raft.store.begin().unwrap();
                kv9_meta::endpoint::node_endpoint(&txn, NodeId(4))
                    .unwrap()
                    .is_some_and(|e| e.active && e.incarnation == incarnation4)
            })
        },
    );

    // Intent refusals: unknown creation, initial-replica destination.
    let leader = cluster_leader(&cluster.runtimes).unwrap();
    let mut admin =
        DataGroupClient::connect(&cluster.runtimes[leader].addr.to_string(), "active-test")
            .unwrap();
    assert!(admin
        .migrate(root, [7; 16], creation.task() + 999, NodeId(4))
        .is_err());
    assert!(admin
        .migrate(root, [7; 16], creation.task(), NodeId(1))
        .is_err());
    let first = admin
        .migrate(root, [7; 16], creation.task(), NodeId(4))
        .unwrap();
    assert!(first.changed);
    assert_eq!(first.intent.region(), creation.region());
    assert_eq!(first.intent.destination().incarnation, incarnation4);
    let retry = admin
        .migrate(root, [7; 16], creation.task(), NodeId(4))
        .unwrap();
    assert!(!retry.changed, "an exact retry is a confirmation receipt");
    assert_eq!(retry.intent, first.intent);
    assert!(retry.applied.index > first.applied.index);
    assert!(
        admin
            .migrate(root, [8; 16], creation.task(), NodeId(4))
            .is_err(),
        "one live migration per group"
    );

    // Image binding requires the committed range; build the exact manifest.
    let sst = |content: &[u8], smallest: &[u8], largest: &[u8]| {
        let sha = RootDigest::sha256(content).to_string();
        SstReference {
            key: format!(
                "clusters/{}/regions/{}/sst/{sha}",
                cluster.root.cluster_id,
                creation.region().0
            ),
            sha256: sha,
            cf: 0,
            smallest: smallest.to_vec(),
            largest: largest.to_vec(),
            size: 1024,
            count: 3,
        }
    };
    let manifest = |index: u64, conf_ver: u64, version: u64| {
        CheckpointManifest {
            scope: FlushScope {
                cluster: cluster.root.cluster_id.to_string(),
                region: creation.region().0,
                conf_ver,
                version,
            },
            term: 3,
            index,
            files: vec![sst(b"migration-sst", b"a", b"m")],
        }
        .encode()
        .unwrap()
    };
    assert!(
        admin
            .bind_image(root, [7; 16], &manifest(10, 1, 1))
            .is_err(),
        "binding before the committed range must refuse"
    );
    let keyspace = admin
        .create_keyspace(root, creation.task(), "migrate-src", TenantId::DEFAULT)
        .unwrap();
    let image = manifest(10, keyspace.range.conf_ver, keyspace.range.version);
    assert!(
        admin.bind_image(root, [9; 16], &image).is_err(),
        "an uncommitted operation cannot bind owners"
    );
    let bound = admin.bind_image(root, [7; 16], &image).unwrap();
    let owner = |id| {
        let leader = cluster_leader(&cluster.runtimes).unwrap();
        let txn = cluster.runtimes[leader]
            .node
            .meta_raft
            .store
            .begin()
            .unwrap();
        kv9_meta::retention::retention_owner(txn.into_view().as_ref(), &cluster.root, id)
            .unwrap()
            .expect("bound owner must be committed")
    };
    let source = owner(bound.source_owner);
    let destination = owner(bound.destination_owner);
    assert_eq!(source.phase, PinPhase::Published);
    assert_eq!(destination.phase, PinPhase::Published);
    assert_eq!(source.binding.descriptor.kind, OwnerKind::Snapshot);
    assert_eq!(destination.binding.descriptor.kind, OwnerKind::Migration);
    assert_eq!(
        source.binding.descriptor.subject,
        *RootDigest::sha256(&image).as_bytes()
    );
    assert_eq!(
        source.binding.descriptor.subject,
        destination.binding.descriptor.subject
    );
    assert_eq!(source.binding.resources, destination.binding.resources);
    assert_eq!(source.binding.resources.len(), 1);

    // Rebinding the same image is idempotent; a different image, a foreign
    // scope or release-without-settlement must refuse.
    let again = admin.bind_image(root, [7; 16], &image).unwrap();
    assert_eq!(again, bound);
    assert!(
        admin
            .bind_image(
                root,
                [7; 16],
                &manifest(11, keyspace.range.conf_ver, keyspace.range.version)
            )
            .is_err(),
        "the same operation can never name a second image"
    );
    assert!(
        admin
            .bind_image(root, [7; 16], &manifest(10, 7, 7))
            .is_err(),
        "a manifest scope differing from the committed range must refuse"
    );
    for request in [
        kv9_meta::retention::LedgerRequest::QuiesceAfterTransfer {
            from: source.binding.token(),
            to: destination.binding.token(),
        },
        kv9_meta::retention::LedgerRequest::Release(source.binding.token()),
    ] {
        let encoded = kv9_meta::retention::encode_request(root, &request).unwrap();
        assert!(
            backend_view(
                &cluster.runtimes[cluster_leader(&cluster.runtimes).unwrap()],
                &cluster.root
            )
            .apply_retention("active-test", encoded)
            .is_err(),
            "no admin path quiesces or releases a migration pin in this increment"
        );
    }
    assert_eq!(owner(bound.source_owner).phase, PinPhase::Published);
    assert_eq!(owner(bound.destination_owner).phase, PinPhase::Published);

    // Attach: uncommitted operation refuses; the committed one turns the
    // destination into a LEARNER (never a voter) idempotently; a voter
    // destination is structurally impossible here because plan_migration
    // already refuses initial replicas.
    wait_for(
        &mut cluster.runtimes,
        30,
        "migration group active, routed and led before attach",
        |rts| {
            data_leader(rts, creation.region()).is_some()
                && rts[..3].iter().all(|rt| {
                    rt.data_groups
                        .raw_directory
                        .by_region(creation.region())
                        .is_some()
                })
        },
    );
    let attach_backend = || {
        backend_view(
            &cluster.runtimes[data_leader(&cluster.runtimes, creation.region()).unwrap()],
            &cluster.root,
        )
    };
    assert!(attach_backend()
        .attach_migration_learner("active-test", root, [9; 16])
        .is_err());
    let attached = attach_backend()
        .attach_migration_learner("active-test", root, [7; 16])
        .unwrap();
    assert!(attached.changed);
    assert_eq!(attached.destination, NodeId(4));
    let confirmed = attach_backend()
        .attach_migration_learner("active-test", root, [7; 16])
        .unwrap();
    assert!(!confirmed.changed, "attach retry must confirm");
    assert!(
        confirmed.cut.index > attached.cut.index,
        "each receipt advances a fresh cut"
    );
    let leader_status = cluster.runtimes
        [data_leader(&cluster.runtimes, creation.region()).unwrap()]
    .data_groups
    .raw_directory
    .by_region(creation.region())
    .unwrap()
    .capture_parts()
    .0
    .status();
    assert!(leader_status.learners.contains(&4));
    assert!(!leader_status.voters.contains(&4));
}
