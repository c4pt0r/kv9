//! A migration RPC owns one exact local Publisher generation, including all
//! object jobs and its durable completion report. No unreported scope resumes.
use super::*;
use crate::checkpoint_retention::{drain_outbox::ReportSlot, CheckpointLedger};
use kv9_common::retention::{OwnerId, PinPhase};
use kv9_engine::checkpoint::{DrainedExecution, ExecutionIdentity};
use kv9_meta::retention::{
    decode_owner_observation, encode_request, migration_publisher, LedgerRequest, OwnerBinding,
};
use kv9_raft::grpc::checkpoint_retention::{LedgerQuery, LedgerReply};
use std::num::NonZeroU64;

struct Ledger<'a>(&'a RuntimeBackend);
impl Ledger<'_> {
    fn call(&self, query: LedgerQuery, root: Option<RootDigest>) -> Result<LedgerReply> {
        checkpoint_retention::RoutedLedger::call_backend(
            self.0,
            query,
            root,
            Instant::now() + Duration::from_secs(30),
            &tokio::runtime::Handle::current(),
        )
    }
}
impl CheckpointLedger for Ledger<'_> {
    fn apply(&self, bytes: Vec<u8>) -> Result<()> {
        match self.call(LedgerQuery::Apply(bytes), None)? {
            LedgerReply::Applied { .. } => Ok(()),
            _ => Err(Error::MetaNotReady(
                "migration publisher receipt differs".into(),
            )),
        }
    }
    fn owner(&self, root: RootDigest, owner: OwnerId) -> Result<Option<Vec<u8>>> {
        match self.call(LedgerQuery::Owner(owner), Some(root))? {
            LedgerReply::Owner(bytes) => Ok(bytes),
            _ => Err(Error::MetaNotReady(
                "migration publisher observation differs".into(),
            )),
        }
    }
}
struct Publisher {
    operation: [u8; 16],
    binding: OwnerBinding,
    identity: ExecutionIdentity,
    slot: ReportSlot,
}
impl Publisher {
    fn prepare(
        backend: &RuntimeBackend,
        descriptor: &CaptureDescriptor,
        ledger: &Ledger<'_>,
    ) -> Result<Self> {
        let context = backend.ttl_context()?;
        let root = context.root.digest();
        let mut binding = migration_publisher::binding(descriptor, context.lifetime)?;
        let observed = ledger.owner(root, binding.descriptor.id)?;
        if let Some(bytes) = &observed {
            let old = decode_owner_observation(bytes, root, binding.descriptor.id)?;
            if old.binding.descriptor != binding.descriptor
                || old.binding.resources != binding.resources
                || old.phase != PinPhase::Released
            {
                return Err(Error::MetaNotReady(
                    "migration publisher has an unfinished generation".into(),
                ));
            }
            binding.generation = old
                .binding
                .generation
                .get()
                .checked_add(1)
                .and_then(NonZeroU64::new)
                .ok_or_else(|| Error::Config("migration publisher generation exhausted".into()))?;
        }
        // Required even in a harness: reservation serializes concurrent RPCs
        // with the same stable publisher ID before either can enter metadata.
        let outbox = context.drain_outbox.as_ref().ok_or_else(|| {
            Error::MetaNotReady("migration publisher requires a durable report outbox".into())
        })?;
        let slot = outbox.reserve(binding.token())?;
        // Another RPC may have completed between our first observation and
        // reservation. Recheck while holding the exclusive owner slot, before
        // binding an execution or constructing a terminal report for this token.
        match ledger.owner(root, binding.descriptor.id) {
            Ok(current) if current == observed => {}
            result => {
                slot.release_unused()?;
                return Err(result.err().unwrap_or_else(|| {
                    Error::MetaNotReady("migration publisher advanced before reservation".into())
                }));
            }
        }
        let identity = ExecutionIdentity {
            root,
            pending: migration_publisher::image_owners(descriptor)?
                .0
                .descriptor
                .id,
            executor: binding.token(),
        };
        Ok(Self {
            operation: descriptor.request().operation,
            binding,
            identity,
            slot,
        })
    }
    fn enter(&self, uploader: &RemoteUploader, ledger: &Ledger<'_>) -> Result<()> {
        uploader.bind_execution(self.identity)?;
        ledger.apply(encode_request(
            self.identity.root,
            &LedgerRequest::EnterMigrationPublisher {
                operation: self.operation,
                publisher: self.binding.clone(),
            },
        )?)
    }
    fn finish(self, report: DrainedExecution, ledger: &Ledger<'_>) -> Result<()> {
        if report.identity() != Some(self.identity) {
            return Err(Error::Engine(
                "migration publisher drain identity differs".into(),
            ));
        }
        let bytes = encode_request(
            self.identity.root,
            &LedgerRequest::FinishMigrationPublisher {
                operation: self.operation,
                publisher: self.binding.clone(),
                issued: report.issued_puts(),
                resolved: report.resolved_puts(),
            },
        )?;
        self.slot.persist(&bytes)?;
        ledger.apply(bytes.clone())?;
        self.slot.acknowledge(&bytes)
    }
}
impl RuntimeBackend {
    pub(in crate::runtime) fn upload_migration_published(
        &self,
        planned: CaptureWork,
        uploader: &RemoteUploader,
    ) -> Result<CapturedSourceImage> {
        let descriptor = planned.descriptor()?;
        let ledger = Ledger(self);
        let publisher = Publisher::prepare(self, &descriptor, &ledger)?;
        let mut payload_lock = None;
        let (result, report) = uploader.with_local_execution_report(|scoped| {
            publisher.enter(scoped, &ledger)?;
            planned.upload(self, scoped, &mut payload_lock)
        })?;
        // Keep the original file lock through the outer actual-job drain.
        drop(payload_lock);
        publisher.finish(report, &ledger)?;
        result
    }
}
