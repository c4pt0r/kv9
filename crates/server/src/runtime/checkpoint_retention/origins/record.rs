use super::*;
use kv9_meta::retention::{data_configuration_origins as census, LedgerPlan, LedgerRequest};

pub(in crate::runtime::checkpoint_retention) struct PreparedOrigins {
    request: census::Request,
    observation: Option<ConfigurationOriginObservation>,
    cursor: Option<OriginCursor>,
}
impl PreparedOrigins {
    pub(in crate::runtime::checkpoint_retention) fn source(&self) -> Option<NodeEndpoint> {
        self.observation.as_ref().map(|v| v.endpoint)
    }
    pub(in crate::runtime::checkpoint_retention) fn plan(
        &self,
        view: &dyn kv9_engine::ReadView,
        root: &RootDescriptor,
        request: &LedgerRequest,
    ) -> Result<LedgerPlan> {
        if request != &LedgerRequest::RecordConfigurationOrigins(self.request) {
            return Err(Error::Config(
                "origin request differs from observation".into(),
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
                o.page().ok_or_else(|| {
                    Error::MetaNotReady("configuration origins remain unknown".into())
                })?,
            ),
        }
    }
}
impl RuntimeBackend {
    pub(in crate::runtime::checkpoint_retention) fn prepare_configuration_origins(
        &self,
        expected_root: RootDigest,
        request: &LedgerRequest,
        deadline: Instant,
    ) -> Result<Option<PreparedOrigins>> {
        let LedgerRequest::RecordConfigurationOrigins(q) = request else {
            return Ok(None);
        };
        self.driver.read_barrier(remaining(deadline)?)?;
        let txn = self.node.meta_raft.store.begin()?;
        let root = kv9_meta::root::certified_root(&txn)?
            .ok_or_else(|| Error::MetaNotReady("configuration origin root unavailable".into()))?;
        if root.digest() != expected_root {
            return Err(Error::Config("configuration origin root differs".into()));
        }
        let view = txn.into_view();
        if census::is_retry(view.as_ref(), &root, *q)? {
            return Ok(Some(PreparedOrigins {
                request: *q,
                observation: None,
                cursor: None,
            }));
        }
        let mut cursor = census::continuation(view.as_ref(), &root, *q)?;
        let old = census::observation(view.as_ref(), &root, q.source)?;
        drop(view);
        let txn = self.node.meta_raft.store.begin()?;
        let endpoint = node_endpoint(&txn, q.source.node)?;
        if old.is_some_and(|p| endpoint.is_none_or(|e| e.generation != p.endpoint_generation)) {
            cursor = None;
        }
        drop(txn);
        let ticket = OriginTicket::resume(
            q.source.identity(&root),
            q.source.region,
            q.source.creation,
            cursor,
        )?;
        let observation = self.read_configuration_origins_source(ticket, deadline);
        // A changed history cannot wedge a persisted cursor. Restart only from
        // another independently confirmed prefix, within the original deadline.
        let observation = match observation {
            Err(_) if cursor.is_some() => {
                cursor = None;
                self.read_configuration_origins_source(
                    OriginTicket::new(
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
                "configuration origin source remains unknown".into(),
            ));
        }
        Ok(Some(PreparedOrigins {
            request: *q,
            observation: Some(observation),
            cursor,
        }))
    }
}
