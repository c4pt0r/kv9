//! Exercise the normal library build, where cfg(test) does not expose leases.
#![cfg(not(feature = "experimental-leader-lease"))]

use kv9_common::{NodeId, RegionId};
use kv9_raft::lease_policy::LeasePolicy;
use kv9_raft::rawnode::{PersistentRaftStorage, RaftPeer};
use kv9_raft::storage::DiskRaftStorage;

#[test]
fn feature_disabled_library_refuses_a_durably_marked_lease_voter() {
    let path =
        std::env::temp_dir().join(format!("kv9-lease-default-reopen-{}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    let (storage, _) = DiskRaftStorage::open(&path, &[1]).unwrap();
    let peer = RaftPeer::with_storage(NodeId(1), RegionId(0), storage).unwrap();
    drop(peer);
    let storage = DiskRaftStorage::recover(&path).unwrap();
    let policy = LeasePolicy {
        node: 1,
        group: 0,
        configuration: 7,
        voters: vec![1],
        promise_ns: 100,
        drift_ppb: 100_000,
        margin_ns: 1,
    };
    let epoch = storage.begin_lease_incarnation(&policy).unwrap();
    drop(storage);
    let storage = DiskRaftStorage::recover(&path).unwrap();
    assert_eq!(storage.recovered_lease_epoch(), Some(epoch));
    let result = RaftPeer::with_storage(NodeId(1), RegionId(0), storage);
    assert!(matches!(result, Err(kv9_common::Error::Raft(ref cause))
        if cause.contains("recovery quarantine")));
    std::fs::remove_dir_all(path).unwrap();
}
