use super::*;

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
