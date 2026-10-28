//! A bounded fallback search across admitted stores, not a placement oracle.
//! Store addresses locate readers; only correlated exact-window evidence from
//! their actual group drivers can settle an attempt. Unknown never settles it.
use super::*;
use kv9_meta::codec::{decode_uint_component, memcmp_uint};
use kv9_meta::endpoint::{node_endpoint, NodeEndpoint};
use kv9_meta::retention::checkpoint_intent::CheckpointIntent;
use kv9_raft::grpc::checkpoint_history::{inspect_on_channel, HistoryTicket};

const NODES_PER_PAGE: usize = 8;
const RPC_BUDGET: Duration = Duration::from_secs(1);

fn route(
    state: &mut Reconciler,
    backend: &RuntimeBackend,
    root: &RootDescriptor,
) -> Result<Option<NodeEndpoint>> {
    let txn = backend.node.meta_raft.store.begin()?;
    if kv9_meta::root::certified_root(&txn)?.as_ref() != Some(root) {
        return Err(Error::Config(
            "checkpoint history directory root differs".into(),
        ));
    }
    if state.remote_node.is_none() {
        let after = state.remote_after.map(|node| vec![memcmp_uint(node.0)]);
        let rows = txn.scan_after(
            &kv9_meta::schema::NODES_DESC,
            after.as_deref(),
            NODES_PER_PAGE,
        )?;
        if rows.is_empty() {
            state.remote_after = None;
            return Ok(None);
        }
        for row in rows {
            let [pk] = row.pk.as_slice() else {
                return Err(Error::Config("invalid checkpoint history node key".into()));
            };
            let node = NodeId(decode_uint_component(pk)?);
            state.remote_after = Some(node);
            if node == backend.node.id {
                continue;
            }
            if node_endpoint(&txn, node)?.is_some_and(|route| route.active) {
                state.remote_node = Some(node);
                break;
            }
        }
    }
    let Some(node) = state.remote_node else {
        return Ok(None);
    };
    let Some(route) = node_endpoint(&txn, node)?.filter(|r| r.active) else {
        return Ok(None);
    };
    if route.incarnation.as_bytes() == &[0; 16]
        || root
            .voter(node)
            .is_some_and(|v| v.store_incarnation != route.incarnation)
        || route.address.port() == 0
        || route.address.ip().is_unspecified()
    {
        return Err(Error::Config(
            "invalid checkpoint history store route".into(),
        ));
    }
    Ok(Some(route))
}

pub(super) fn inspect(
    state: &mut Reconciler,
    backend: &RuntimeBackend,
    root: &RootDescriptor,
    intent: &CheckpointIntent,
    deadline: Instant,
    stats: &mut Stats,
) -> Result<Option<settlement::Settlement>> {
    let Some(route) = route(state, backend, root)? else {
        return Ok(None);
    };
    let manifest = intent.manifest()?;
    let ticket = HistoryTicket::new(
        StoreIdentity {
            root_digest: root.digest(),
            cluster_id: root.cluster_id,
            node_id: route.node,
            store_incarnation: route.incarnation,
        },
        RegionId(manifest.scope.region),
        intent.predecessor(),
    )?;
    let context = backend.ttl_context()?;
    let end = deadline.min(Instant::now() + RPC_BUDGET);
    stats.remote_queries = stats.remote_queries.saturating_add(1);
    let result = tokio::runtime::Handle::current().block_on(async {
        let channel = context.channel(route.address, end).await?;
        let budget = end
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| Error::MetaNotReady("checkpoint history query expired".into()))?;
        inspect_on_channel(
            channel,
            &ticket,
            backend.node.id,
            Some(context.token.clone()),
            budget,
        )
        .await
    });
    let reply = match result {
        Ok(reply) => reply,
        Err(_) => {
            stats.remote_errors = stats.remote_errors.saturating_add(1);
            return Ok(None);
        }
    };
    let Some(winner) = reply.winner_change_id() else {
        return Ok(None);
    };
    // A changed route, generation or store incarnation invalidates this pass's
    // evidence. No connection cache or earlier positive reply bypasses recheck.
    let budget = deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or_else(|| Error::MetaNotReady("checkpoint history recheck expired".into()))?;
    backend
        .driver
        .read_barrier(budget.min(Duration::from_secs(1)))?;
    let txn = backend.node.meta_raft.store.begin()?;
    if kv9_meta::root::certified_root(&txn)?.as_ref() != Some(root)
        || node_endpoint(&txn, route.node)? != Some(route)
    {
        stats.remote_errors = stats.remote_errors.saturating_add(1);
        return Ok(None);
    }
    Ok(Some(
        if winner.as_slice() == manifest.change_id(intent.predecessor())? {
            settlement::Settlement::Applied
        } else {
            settlement::Settlement::WindowRefused {
                winner,
                source: route.node,
            }
        },
    ))
}
