//! Per-process, per-source read generations. A returned GET timeout cannot
//! release a pin: only the GET-only uploader's distinct drained report can.
use super::*;
use kv9_common::retention::OwnerToken;
use kv9_engine::checkpoint::{CheckpointReader, DrainedRead, ExecutionIdentity};

pub(crate) struct CheckpointRead {
    root: RootDigest,
    pending: OwnerId,
    source: OwnerToken,
    binding: OwnerBinding,
    attached: bool,
    drained: bool,
    slot: Option<super::drain_outbox::ReportSlot>,
    image: bool,
    read_image: Option<kv9_common::data_read_fence::ReadImage>,
}
impl CheckpointRead {
    pub(crate) fn new(
        root: &RootDescriptor,
        manifest: &CheckpointManifest,
        predecessor: u64,
        source_kind: OwnerKind,
        ledger: &(impl CheckpointLedger + ?Sized),
    ) -> Result<Self> {
        let intent = CheckpointIntent::new(root, manifest, predecessor)?;
        let source = intent.binding(source_kind)?;
        Self::prepare(
            root.digest(),
            intent.pending_id(),
            source,
            false,
            data_read_image(manifest)?,
            ledger,
        )
    }
    pub(crate) fn for_image(
        source: OwnerBinding,
        manifest: &CheckpointManifest,
        ledger: &(impl CheckpointLedger + ?Sized),
    ) -> Result<Self> {
        Self::prepare(
            source.descriptor.root,
            source.descriptor.id,
            source,
            true,
            data_read_image(manifest)?,
            ledger,
        )
    }
    fn prepare(
        root: RootDigest,
        pending: OwnerId,
        source: OwnerBinding,
        image: bool,
        read_image: Option<kv9_common::data_read_fence::ReadImage>,
        ledger: &(impl CheckpointLedger + ?Sized),
    ) -> Result<Self> {
        let mut binding = if image {
            kv9_meta::retention::image_reader::binding(&source, ledger.local_lifetime()?)?
        } else {
            kv9_meta::retention::checkpoint_reader::binding(&source, ledger.local_lifetime()?)?
        };
        let observed = ledger.owner(root, binding.descriptor.id)?;
        if let Some(bytes) = &observed {
            let old = decode_owner_observation(bytes, root, binding.descriptor.id)?;
            if old.binding.descriptor != binding.descriptor
                || old.binding.resources != binding.resources
                || old.phase != PinPhase::Released
            {
                return Err(Error::Engine(
                    "checkpoint reader has an unfinished generation".into(),
                ));
            }
            binding.generation = old
                .binding
                .generation
                .get()
                .checked_add(1)
                .and_then(NonZeroU64::new)
                .ok_or_else(|| Error::Engine("checkpoint reader generation exhausted".into()))?;
        }
        let slot = ledger
            .drain_outbox()?
            .map(|outbox| outbox.reserve(binding.token()))
            .transpose()?;
        if image && slot.is_none() {
            return Err(Error::Engine(
                "image reader requires a durable report outbox".into(),
            ));
        }
        if let Some(slot) = &slot {
            match ledger.owner(root, binding.descriptor.id) {
                Ok(current) if current == observed => {}
                result => {
                    slot.release_unused()?;
                    return Err(result.err().unwrap_or_else(|| {
                        Error::MetaNotReady("reader advanced before reservation".into())
                    }));
                }
            }
        }
        Ok(Self {
            root,
            pending,
            source: source.token(),
            binding,
            attached: false,
            drained: false,
            slot,
            image,
            read_image,
        })
    }
    fn identity(&self) -> ExecutionIdentity {
        ExecutionIdentity {
            root: self.root,
            pending: self.pending,
            executor: self.binding.token(),
        }
    }
    pub(crate) fn before_io(
        &mut self,
        ledger: &(impl CheckpointLedger + ?Sized),
        scoped: &CheckpointReader<'_>,
    ) -> Result<()> {
        if self.attached {
            return Err(Error::Engine(
                "checkpoint reader scope already attached".into(),
            ));
        }
        scoped.bind_execution(self.identity())?;
        self.attached = true;
        ledger.check_data_read(self.root, self.read_image.as_ref())?;
        ledger.apply(encode_request(
            self.root,
            &LedgerRequest::Share {
                from: self.source,
                to: self.binding.clone(),
            },
        )?)?;
        ledger.apply(encode_request(
            self.root,
            &LedgerRequest::Publish(self.binding.token()),
        )?)
    }
    pub(crate) fn observe_drain(&mut self, report: DrainedRead) -> Result<()> {
        if !self.attached || self.drained || report.identity() != Some(self.identity()) {
            return Err(Error::Engine(
                "checkpoint read drain identity differs".into(),
            ));
        }
        self.drained = true;
        Ok(())
    }
    pub(crate) fn finish(&self, ledger: &(impl CheckpointLedger + ?Sized)) -> Result<()> {
        if !self.drained {
            return Err(Error::Engine("checkpoint reader has not drained".into()));
        }
        let request = if self.image {
            LedgerRequest::FinishImageRead {
                successor: self.source,
                reader: self.binding.clone(),
            }
        } else {
            LedgerRequest::FinishCheckpointRead {
                successor: self.source,
                reader: self.binding.clone(),
            }
        };
        let bytes = encode_request(self.root, &request)?;
        if let Some(slot) = &self.slot {
            slot.persist(&bytes)?;
        }
        ledger.apply(bytes.clone())?;
        if let Some(slot) = &self.slot {
            slot.acknowledge(&bytes)?;
        }
        Ok(())
    }
}
