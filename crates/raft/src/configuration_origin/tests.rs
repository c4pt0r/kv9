use super::*;
use raft::prelude::ConfChangeSingle;

pub(crate) fn origin() -> ConfigurationOrigin {
    ConfigurationOrigin {
        root: RootDigest::sha256(b"root"),
        creation: RootDigest::sha256(b"creation"),
        region: RegionId(100),
        operation: [1; 16],
        task: 104,
        node: NodeId(4),
        store: StoreIncarnation::from_bytes([4; 16]),
        source_digest: RootDigest::sha256(b"task row"),
        action: Action::Attach,
    }
}
pub(crate) fn change(o: ConfigurationOrigin) -> ConfChangeV2 {
    ConfChangeV2 {
        changes: vec![ConfChangeSingle {
            node_id: o.node.0,
            change_type: o.action.change_type(),
            ..Default::default()
        }]
        .into(),
        context: o.encode().unwrap().into(),
        ..Default::default()
    }
}
#[test]
fn configuration_origin_canonical_context_binds_node_action_and_task_kind() {
    for action in [
        Action::Attach,
        Action::Promote,
        Action::Remove,
        Action::Detach,
    ] {
        let o = ConfigurationOrigin { action, ..origin() };
        let bytes = o.encode().unwrap();
        assert_eq!(bytes.len(), ORIGIN_BYTES);
        assert_eq!(ConfigurationOrigin::decode(&bytes).unwrap(), o);
        for end in 0..bytes.len() {
            assert!(ConfigurationOrigin::decode(&bytes[..end]).is_err());
        }
        for field in [
            8..40,
            40..72,
            72..80,
            80..96,
            96..104,
            104..112,
            112..120,
            120..136,
            136..168,
            168..169,
        ] {
            let mut bad = bytes.clone();
            bad[field].fill(0);
            assert!(ConfigurationOrigin::decode(&bad).is_err());
        }
        let mut cc = change(o);
        assert_eq!(ConfigurationOrigin::from_change(&cc).unwrap(), Some(o));
        cc.changes[0].node_id += 1;
        assert!(ConfigurationOrigin::from_change(&cc).is_err());
        cc = change(o);
        cc.changes.clear();
        assert!(ConfigurationOrigin::from_change(&cc).is_err());
        cc = change(o);
        cc.transition = ConfChangeTransition::Explicit;
        assert!(ConfigurationOrigin::from_change(&cc).is_err());
    }
    assert_eq!(
        ConfigurationOrigin::from_change(&ConfChangeV2::default()).unwrap(),
        None
    );
}

#[test]
fn configuration_origin_foreign_group_refuses_before_proposal_or_membership_apply() {
    use protobuf::Message as _;
    let peer = crate::RaftPeer::new(NodeId(1), RegionId(100), &[NodeId(1)]).unwrap();
    let before = peer.status_snapshot();
    let cc = change(ConfigurationOrigin {
        region: RegionId(200),
        ..origin()
    });
    assert!(peer.propose_conf_change_traced(cc.clone()).is_err());
    assert!(peer
        .apply_conf_change_bytes(
            crate::EntryKind::ConfChangeV2,
            &cc.write_to_bytes().unwrap(),
            1
        )
        .is_err());
    let after = peer.status_snapshot();
    assert_eq!(before.log_last_index, after.log_last_index);
    assert_eq!(before.voters, after.voters);
    assert_eq!(before.learners, after.learners);
    assert_eq!(before.conf_applied, after.conf_applied);
}
