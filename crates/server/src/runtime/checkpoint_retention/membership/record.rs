use super::*;
use kv9_meta::retention::{data_membership as census, LedgerPlan, LedgerRequest};

pub(in crate::runtime::checkpoint_retention) struct PreparedMembership {
    request: census::Request,
    observation: Option<DataMembershipObservation>,
    cursor: Option<MembershipCursor>,
    seen: usize,
}
impl PreparedMembership {
    pub(in crate::runtime::checkpoint_retention) fn source(&self) -> Option<NodeEndpoint> {
        self.observation.as_ref().map(|v| v.endpoint)
    }
    pub(in crate::runtime::checkpoint_retention) fn plan(
        &self,
        view: &dyn kv9_engine::ReadView,
        root: &RootDescriptor,
        request: &LedgerRequest,
    ) -> Result<LedgerPlan> {
        if request != &LedgerRequest::RecordDataMembership(self.request) {
            return Err(Error::Config(
                "membership request differs from observation".into(),
            ));
        }
        match &self.observation {
            None => census::plan_retry(view, root, self.request),
            Some(o) => census::plan_verified(
                view,
                root,
                self.request,
                o.endpoint,
                self.cursor,
                self.seen,
                o.page()
                    .ok_or_else(|| Error::MetaNotReady("membership remains unknown".into()))?,
            ),
        }
    }
}
impl RuntimeBackend {
    pub(in crate::runtime::checkpoint_retention) fn prepare_data_membership(
        &self,
        expected_root: RootDigest,
        request: &LedgerRequest,
        deadline: Instant,
    ) -> Result<Option<PreparedMembership>> {
        let LedgerRequest::RecordDataMembership(q) = request else {
            return Ok(None);
        };
        self.driver.read_barrier(remaining(deadline)?)?;
        let txn = self.node.meta_raft.store.begin()?;
        let root = kv9_meta::root::certified_root(&txn)?
            .ok_or_else(|| Error::MetaNotReady("membership root unavailable".into()))?;
        if root.digest() != expected_root {
            return Err(Error::Config("membership root differs".into()));
        }
        let view = txn.into_view();
        if census::is_retry(view.as_ref(), &root, *q)? {
            return Ok(Some(PreparedMembership {
                request: *q,
                observation: None,
                cursor: None,
                seen: 0,
            }));
        }
        let (mut cursor, mut seen) = census::continuation(view.as_ref(), &root, *q)?;
        let old = census::observation(view.as_ref(), &root, q.source)?;
        drop(view);
        let txn = self.node.meta_raft.store.begin()?;
        let endpoint = node_endpoint(&txn, q.source.node)?;
        if old.is_some_and(|p| endpoint.is_none_or(|e| e.generation != p.endpoint_generation)) {
            cursor = None;
            seen = 0;
        }
        drop(txn);
        let ticket = MembershipTicket::resume(
            q.source.identity(&root),
            q.source.region,
            q.source.creation,
            cursor,
            seen,
        )?;
        let observation = self.read_data_membership_source(ticket, deadline);
        // A stale local-history cursor cannot permanently wedge durable progress.
        // Only a separately confirmed first page can restart; an RPC error or
        // Unknown is never an empty page or permission to skip the old prefix.
        let observation = match observation {
            Err(_) if cursor.is_some() => {
                cursor = None;
                seen = 0;
                self.read_data_membership_source(
                    MembershipTicket::new(
                        q.source.identity(&root),
                        q.source.region,
                        q.source.creation,
                    )?,
                    deadline,
                )?
            }
            other => other?,
        };
        if observation.page().is_none() {
            return Err(Error::MetaNotReady(
                "membership source remains unknown".into(),
            ));
        }
        Ok(Some(PreparedMembership {
            request: *q,
            observation: Some(observation),
            cursor,
            seen,
        }))
    }
}
