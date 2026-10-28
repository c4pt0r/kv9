use super::*;
use kv9_raft::configuration_origin::Action;
mod census;
mod query;

async fn recorded(
    f: &Fixture,
    action: Action,
) -> Vec<kv9_raft::storage::configuration_origins::AppliedConfigurationOrigin> {
    let index = f.data[0].peer().status_snapshot().conf_applied;
    tokio::time::timeout(Duration::from_secs(10), async {
        while f
            .data
            .iter()
            .any(|d| d.peer().status_snapshot().conf_applied < index)
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let term = f.data[0].status().term;
    let records: Vec<_> = f
        .data
        .iter()
        .map(|d| {
            d.peer()
                .configuration_origin_at(AppliedPosition { term, index })
                .unwrap()
                .unwrap()
        })
        .collect();
    assert!(records
        .iter()
        .all(|r| *r == records[0] && r.origin.action == action));
    for backend in &f.backends {
        kv9_meta::data_groups::store_history::validate_configuration_origin(
            backend
                .node
                .meta_raft
                .store
                .engine()
                .snapshot()
                .unwrap()
                .as_ref(),
            f.range.root,
            &records[0].origin,
        )
        .unwrap();
    }
    records
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn runtime_configuration_origins_follow_committed_migration_and_survive_all_replica_restarts()
{
    let f = Fixture::new().await;
    let root = f.backends[0].ttl_context().unwrap().root.clone();
    let operation = [119; 16];
    let (identity, guard, _) = prepare_migration(&f, &root, operation).await;
    let attached = recorded(&f, Action::Attach).await;
    assert_eq!(attached[0].origin.store, identity.store_incarnation);
    let mut t = f.backends[1].node.meta_raft.store.begin().unwrap();
    let (abort, _) = kv9_meta::data_groups::abort::plan_abort(&mut t, operation, [19; 32]).unwrap();
    let at = f.meta[1]
        .propose(&Command::from_batch(&t.into_batch()))
        .unwrap();
    applied(&f.meta, at).await;
    let source = f.backends[0].clone();
    let digest = root.digest();
    tokio::task::spawn_blocking(move || source.detach_aborted_learner("test", digest, operation))
        .await
        .unwrap()
        .unwrap();
    let detached = recorded(&f, Action::Detach).await;
    assert_eq!(detached[0].origin.task, abort.task());
    assert_eq!(detached[0].origin.store, identity.store_incarnation);
    assert!(detached[0].at.index > attached[0].at.index);
    let source = f.backends[0].clone();
    tokio::task::spawn_blocking(move || source.detach_aborted_learner("test", digest, operation))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        recorded(&f, Action::Detach).await,
        detached,
        "idempotent retry must not invent another configuration change"
    );
    drop(guard);
    let path = f.shutdown_keep().await;
    for node in 1..=3 {
        let storage = DiskRaftStorage::recover(&path.join(format!("n{node}/data-raft"))).unwrap();
        for records in [&attached, &detached] {
            assert_eq!(
                storage.configuration_origin_at(records[0].at).unwrap(),
                Some(records[0])
            );
        }
    }
    std::fs::remove_dir_all(path).unwrap();
}

mod timeline;

mod intervals;
mod timeline_census;
