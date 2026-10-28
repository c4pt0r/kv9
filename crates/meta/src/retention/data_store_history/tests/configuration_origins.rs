use super::*;
use crate::data_groups::store_history::{configuration_origin, validate_configuration_origin};
use kv9_common::{data_range::DataRange, KeyspaceId, TenantId};
use kv9_raft::configuration_origin::Action;

#[test]
fn configuration_origins_reconcile_all_actions_with_exact_immutable_tasks() {
    let (s, root) = fixture();
    let creation = group(&s, 31);
    let range = DataRange {
        root: root.digest(),
        creation: creation.digest(),
        region: creation.region(),
        keyspace: KeyspaceId(100),
        tenant: TenantId(1),
        conf_ver: 1,
        version: 1,
        start: vec![],
        end: vec![],
        sealed: false,
    };
    let mut t = s.begin().unwrap();
    activation::plan_activation(&mut t, &creation).unwrap();
    let (old, _) = migration::plan_migration(&mut t, [32; 16], creation.task(), NodeId(4)).unwrap();
    let (aborted, _) = abort::plan_abort(&mut t, old.operation(), [9; 32]).unwrap();
    t.commit().unwrap();
    let mut t = s.begin().unwrap();
    let (new, _) = migration::plan_migration(&mut t, [33; 16], creation.task(), NodeId(5)).unwrap();
    let receipt = evidence::InstallEvidence::receipt(
        &new,
        StoreIncarnation::from_bytes([21; 16]),
        RootDigest::sha256(b"image"),
        [22; 32],
        AppliedPosition { term: 3, index: 12 },
    )
    .unwrap();
    let (installed, _) = evidence::plan_install_evidence(&mut t, &receipt).unwrap();
    let (removed, _) =
        removal::plan_removal(&mut t, new.operation(), creation.replicas()[0]).unwrap();
    t.commit().unwrap();
    let view = s.engine().snapshot().unwrap();
    for (task, op, replica, action) in [
        (
            old.task(),
            old.operation(),
            old.destination(),
            Action::Attach,
        ),
        (
            aborted.task(),
            old.operation(),
            old.destination(),
            Action::Detach,
        ),
        (
            installed.task(),
            new.operation(),
            new.destination(),
            Action::Promote,
        ),
        (
            removed.task(),
            new.operation(),
            removed.source(),
            Action::Remove,
        ),
    ] {
        let o = configuration_origin(view.as_ref(), &range, task, op, replica, action).unwrap();
        validate_configuration_origin(view.as_ref(), root.digest(), &o).unwrap();
        let placement = Origin {
            region: o.region,
            creation: o.creation,
            replica,
            task,
            kind: action.task_kind(),
            source_digest: o.source_digest,
        };
        let (native, digest) = crate::data_groups::store_history::initial_creation(
            view.as_ref(),
            root.digest(),
            &placement,
        )
        .unwrap();
        assert_eq!(native, creation);
        assert_ne!(digest.as_bytes(), &[0; 32]);
        let mut wrong_placement = placement;
        wrong_placement.source_digest = RootDigest::sha256(b"different original task");
        assert!(crate::data_groups::store_history::initial_creation(
            view.as_ref(),
            root.digest(),
            &wrong_placement
        )
        .is_err());
        for field in 0..8 {
            let mut wrong = o;
            match field {
                0 => wrong.store = StoreIncarnation::from_bytes([44; 16]),
                1 => wrong.node = NodeId(44),
                2 => wrong.operation = [44; 16],
                3 => wrong.source_digest = RootDigest::sha256(b"foreign row"),
                4 => wrong.creation = RootDigest::sha256(b"foreign creation"),
                5 => wrong.region = RegionId(444),
                6 => wrong.task = 444,
                _ => {
                    wrong.action = if action == Action::Attach {
                        Action::Promote
                    } else {
                        Action::Attach
                    }
                }
            }
            assert!(
                validate_configuration_origin(view.as_ref(), root.digest(), &wrong).is_err(),
                "{action:?} field {field}"
            );
        }
        assert!(validate_configuration_origin(
            view.as_ref(),
            RootDigest::sha256(b"foreign root"),
            &o
        )
        .is_err());
    }
    // Updating the current node registration cannot rewrite a historical origin.
    let before = configuration_origin(
        view.as_ref(),
        &range,
        old.task(),
        old.operation(),
        old.destination(),
        Action::Attach,
    )
    .unwrap();
    drop(view);
    let mut t = s.begin().unwrap();
    t.update(
        &NODES_DESC,
        &[memcmp_uint(4)],
        vec![(ColumnId(5), ColumnValue::Bytes(vec![44; 16]))],
    )
    .unwrap();
    t.commit().unwrap();
    validate_configuration_origin(
        s.engine().snapshot().unwrap().as_ref(),
        root.digest(),
        &before,
    )
    .unwrap();
    assert_eq!(before.store, StoreIncarnation::from_bytes([4; 16]));
}
