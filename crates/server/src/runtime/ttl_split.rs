//! TTL branches of the existing split coordinator. Applied catalog authority,
//! actual sealed source snapshots, full V8 confirmations and typed apply receipts.
use super::*;
use futures_util::{stream, StreamExt};
use kv9_common::raw_ttl::split::{SplitPlan, SplitSeal, SEAL_KEY};
use kv9_engine::ColumnFamily;
use kv9_meta::data_groups::split::{committed_split_plan, CommittedSplitPlan};
use kv9_raft::rawnode::ttl_split::TtlSplitProbe;

type Driver = Arc<NodeDriver<DiskRaftStorage, WalEngine>>;
const BUDGET: Duration = Duration::from_secs(30);
fn remaining(deadline: Instant) -> Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or_else(|| Error::MetaNotReady("TTL split control deadline expired".into()))
}
impl RuntimeBackend {
    fn ttl_split_plan(&self, root: RootDigest, operation: [u8; 16]) -> Result<CommittedSplitPlan> {
        let committed = committed_split_plan(&self.node.meta_raft.store, operation)?;
        if committed.plan().parent.root != root || self.ttl_context()?.root.digest() != root {
            return Err(Error::Config("TTL split metadata root differs".into()));
        }
        Ok(committed)
    }
    fn ttl_split_driver(&self, region: RegionId) -> Result<Driver> {
        let from_handles = {
            self.group_handles
                .lock()
                .expect("group handles poisoned")
                .get(&region)
                .map(|(_, d)| d.clone())
        };
        from_handles
            // Existing runtime fixtures and the live Raw directory also own a
            // bound parent; unbound children must always use group_handles.
            .or_else(|| {
                self.raw_directory
                    .by_region(region)
                    .map(|g| g.capture_parts().0.clone())
            })
            .ok_or_else(|| Error::MetaNotReady("TTL split group is not active locally".into()))
    }
    pub(super) fn submit_ttl_split(
        &self,
        committed: &CommittedSplitPlan,
        driver: &Driver,
        probe: TtlSplitProbe,
    ) -> Result<AppliedPosition> {
        self.ensure_serving()?;
        let context = self.ttl_context()?;
        let deadline = probe.deadline();
        let target = probe.target().clone();
        let ids = probe.replicas().collect::<Vec<_>>();
        let mut expected = committed.replicas().to_vec();
        expected.sort_unstable();
        if ids != expected
            || ![
                &committed.plan().parent,
                &committed.plan().low,
                &committed.plan().high,
            ]
            .contains(&&target)
        {
            return Err(Error::MetaNotReady(
                "TTL split full configuration or target differs from committed plan".into(),
            ));
        }
        let stores = ttl_activation::endpoints(
            &self.node.meta_raft.store.begin()?,
            &context.root,
            &target,
            &ids,
        )?;
        let identities = stores.iter().map(|(id, _)| *id).collect::<Vec<_>>();
        let mut confirmations = probe.bind_stores(&identities)?;
        let collect_stores = stores.clone();
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        // This is read-only collection. A timed-out caller aborts it; no task
        // owns permission to append in the background after the caller returns.
        let task = self.transport.runtime_handle().spawn(async move {
            let result = async {
                let replies =
                    stream::iter(collect_stores.into_iter().map(|(identity, endpoint)| {
                        let context = context.clone();
                        async move {
                            if identity.node_id == context.local.identity().node_id {
                                if identity != context.local.identity() {
                                    return Err(Error::MetaNotReady(
                                        "TTL split local store incarnation differs".into(),
                                    ));
                                }
                                Ok(context.local)
                            } else {
                                kv9_raft::grpc::confirm_raw_ttl_format_on_channel(
                                    context.channel(endpoint.address, deadline).await?,
                                    identity,
                                    context.local.identity().node_id,
                                    Some(context.token.clone()),
                                    remaining(deadline)?,
                                )
                                .await
                            }
                        }
                    }))
                    .buffer_unordered(8);
                tokio::pin!(replies);
                while let Some(reply) = replies.next().await {
                    confirmations.confirm(reply?)?;
                }
                confirmations.finish()
            }
            .await;
            let _ = sender.send(result);
        });
        let result = receiver.recv_timeout(deadline.saturating_duration_since(Instant::now()));
        task.abort();
        let prepared = result
            .map_err(|_| Error::MetaNotReady("TTL split format collection incomplete".into()))??;
        ttl_activation::verify_endpoints(&self.node.meta_raft.store.begin()?, &stores)?;
        let now = self.ttl_split_plan(committed.plan().parent.root, committed.plan().operation)?;
        if now.plan() != committed.plan()
            || now.published()
            || !Arc::ptr_eq(&self.ttl_split_driver(target.region)?, driver)
        {
            return Err(Error::StaleEpoch {
                region: target.region,
            });
        }
        self.ensure_serving()?;
        let submission = driver.propose_ttl_split(prepared)?;
        let at = submission.proposed_at();
        // Keep the accepted position in all unknown/replaced outcomes. The
        // asynchronous reservation is dropped; the established synchronous
        // exact-position receipt path remains responsible for confirmation.
        drop(submission);
        settle_proposal(
            at,
            driver.wait_applied(at, deadline.saturating_duration_since(Instant::now())),
            true,
        )?
        .ok_or_else(|| Error::Raft("TTL split exact apply receipt missing".into()))
    }
    pub(super) fn seal_ttl_split(
        &self,
        root: RootDigest,
        operation: [u8; 16],
    ) -> Result<crate::api::SealSplitParentResult> {
        let committed = self.ttl_split_plan(root, operation)?;
        let plan = committed.plan();
        let driver = self.ttl_split_driver(plan.parent.region)?;
        let deadline = Instant::now() + BUDGET;
        let engine = self
            .raw_directory
            .by_region(plan.parent.region)
            .ok_or_else(|| Error::MetaNotReady("TTL split parent is not active locally".into()))?
            .capture_parts()
            .1
            .clone();
        let probe = driver.begin_ttl_split_seal(plan.clone(), deadline)?;
        let applied = self.submit_ttl_split(&committed, &driver, probe)?;
        let seal = engine
            .get(ColumnFamily::Default, SEAL_KEY)?
            .map(|b| SplitSeal::decode(&b))
            .transpose()?
            .filter(|s| s.plan == *plan)
            .ok_or_else(|| Error::Raft("TTL source seal receipt differs".into()))?;
        Ok(crate::api::SealSplitParentResult {
            sealed_version: plan.sealed_range()?.version,
            changed: seal.cut == applied,
            cut: seal.cut,
        })
    }
    pub(super) fn populate_ttl_split(
        &self,
        root: RootDigest,
        operation: [u8; 16],
        high: bool,
    ) -> Result<crate::api::PopulateSplitChildResult> {
        let committed = self.ttl_split_plan(root, operation)?;
        let target = committed.plan().target(high);
        if committed.published() {
            return Err(Error::StaleEpoch {
                region: target.region,
            });
        }
        let deadline = Instant::now() + BUDGET;
        let source_driver = self.ttl_split_driver(committed.plan().parent.region)?;
        let source = source_driver.capture_ttl_split_source(committed.plan(), deadline)?;
        let child = self.ttl_split_driver(target.region)?;
        if child.ttl_import_state(target)?.is_none() {
            let at = child.propose(&Command::Noop)?;
            settle_proposal(at, child.wait_applied(at, remaining(deadline)?), true)?;
        }
        let probe = child.begin_ttl_split_import(&source, high, deadline)?;
        self.submit_ttl_split(&committed, &child, probe)?;
        while !child.ttl_import_state(target)?.is_some_and(|s| s.complete) {
            let probe = child.begin_ttl_split_copy(&source, high, deadline)?;
            self.submit_ttl_split(&committed, &child, probe)?;
        }
        child.verify_ttl_split_target(&source, high, deadline)?;
        Ok(crate::api::PopulateSplitChildResult {
            rows_copied: source.spec(high).rows,
            parent_half_digest: source.spec(high).digest,
            child_digest: source.spec(high).digest,
        })
    }
    pub(super) fn verify_ttl_split_publication(
        &self,
        root: RootDigest,
        operation: [u8; 16],
    ) -> Result<SplitPlan> {
        let committed = self.ttl_split_plan(root, operation)?;
        if committed.published() {
            return Err(Error::StaleEpoch {
                region: committed.plan().parent.region,
            });
        }
        let deadline = Instant::now() + BUDGET;
        let source = self
            .ttl_split_driver(committed.plan().parent.region)?
            .capture_ttl_split_source(committed.plan(), deadline)?;
        for high in [false, true] {
            self.ttl_split_driver(committed.plan().target(high).region)?
                .verify_ttl_split_target(&source, high, deadline)?;
        }
        Ok(committed.plan().clone())
    }
}
