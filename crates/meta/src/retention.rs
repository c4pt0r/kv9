//! Bounded retention-ledger planning over one established metadata view.
//!
//! Plans are not durable receipts. The caller must hold the metadata planner
//! lock, establish/drain the current Raft term before supplying this view, and
//! commit the entire returned batch in that same term before acknowledging it.
//! No caller may execute individual resource writes from a plan.
//!
//! The ledger tracks explicitly registered owners only. It does not certify a
//! complete backfill of existing references and provides no deletion operation.
//! Final release after actual use drains needs a separate settlement capability;
//! this first interface permits quiescence only under a published successor.
use std::collections::BTreeMap;
use std::num::NonZeroU64;

use kv9_common::retention::{
    OwnerId, OwnerSlot, OwnerToken, PinPhase, ResourceIdentity, ResourceKind,
};
use kv9_common::{Error, Result, RootDescriptor, RootDigest};
use kv9_engine::{ColumnFamily, ReadView, WriteBatch};

pub const MAX_OWNER_RESOURCES: usize = kv9_common::retention::MAX_OWNER_RESOURCES;
pub const MAX_LEDGER_BATCH_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_LEDGER_REQUEST_BYTES: usize = kv9_engine::checkpoint::MAX_MANIFEST_BYTES + 256;
const MAX_SMALL_REQUEST_BYTES: usize = 16 * 1024;
const MAX_OWNER_BYTES: usize = 8 * 1024;
const PREFIX: &[u8] = b"\0kv9\0retention_v1\0";
const HEADER_MAGIC: &[u8; 8] = b"KV9LED27";
const DATA_TIMELINE_HEADER_MAGIC: &[u8; 8] = b"KV9LED26";
const DATA_INITIAL_HEADER_MAGIC: &[u8; 8] = b"KV9LED25";
const DATA_ORIGINS_HEADER_MAGIC: &[u8; 8] = b"KV9LED24";
const DATA_MEMBERSHIP_HEADER_MAGIC: &[u8; 8] = b"KV9LED23";
const DATA_HISTORY_HEADER_MAGIC: &[u8; 8] = b"KV9LED22";
const DATA_FENCE_HEADER_MAGIC: &[u8; 8] = b"KV9LED21";
const IMAGE_PUBLISHER_HEADER_MAGIC: &[u8; 8] = b"KV9LED20";
const IMAGE_READER_HEADER_MAGIC: &[u8; 8] = b"KV9LED19";
const METADATA_IMAGE_GATE_HEADER_MAGIC: &[u8; 8] = b"KV9LED18";
const BOOTSTRAP_UNENTERED_HEADER_MAGIC: &[u8; 8] = b"KV9LED17";
const BOOTSTRAP_CLOSURE_HEADER_MAGIC: &[u8; 8] = b"KV9LED16";
const METADATA_READ_GATE_HEADER_MAGIC: &[u8; 8] = b"KV9LED15";
const METADATA_SELECTION_HEADER_MAGIC: &[u8; 8] = b"KV9LED14";
const BOOTSTRAP_INDEX_HEADER_MAGIC: &[u8; 8] = b"KV9LED13";
const BOOTSTRAP_ADMISSION_HEADER_MAGIC: &[u8; 8] = b"KV9LED12";
const READER_CLOSURE_HEADER_MAGIC: &[u8; 8] = b"KV9LED11";
const READER_GENERATION_HEADER_MAGIC: &[u8; 8] = b"KV9LED10";
const READER_INDEX_HEADER_MAGIC: &[u8; 8] = b"KV9LED09";
const MIGRATION_HEADER_MAGIC: &[u8; 8] = b"KV9LED08";
const CLOSURE_HEADER_MAGIC: &[u8; 8] = b"KV9LED07";
const ATOMIC_HEADER_MAGIC: &[u8; 8] = b"KV9LED06";
const ENROLLMENT_HEADER_MAGIC: &[u8; 8] = b"KV9LED05";
const BUILD_INDEX_HEADER_MAGIC: &[u8; 8] = b"KV9LED04";
const GATED_HEADER_MAGIC: &[u8; 8] = b"KV9LED03";
const ROWS_HEADER_MAGIC: &[u8; 8] = b"KV9LED02";
const LEGACY_HEADER_MAGIC: &[u8; 8] = b"KV9LED01";
const OWNER_MAGIC: &[u8; 8] = b"KV9OWN01";
const EXECUTOR_OWNER_MAGIC: &[u8; 8] = b"KV9OWN02";
const REQUEST_MAGIC: &[u8; 8] = b"KV9RTX01";
const TRACKING_ONLY: u8 = 1;

pub mod bootstrap_closure;
pub mod bootstrap_index;
pub mod build_index;
pub mod image_publisher_closure;
pub mod image_publisher_index;
pub mod image_reader_closure;
pub mod image_reader_index;
pub mod metadata_image_gate;
pub mod metadata_image_write_gate;
pub mod reader_closure;
pub mod reader_index;
pub mod reader_lifecycle;
mod resource_rows;
use resource_rows::ResourceRecord;
pub mod checkpoint_intent;
pub mod data_configuration_intervals;
pub mod data_configuration_origins;
pub mod data_configuration_timeline;
pub mod data_membership;
pub mod data_read_fence;
pub mod data_store_history;
use checkpoint_intent::CheckpointIntent;
pub mod bootstrap_reader;
pub mod checkpoint_admission;
pub mod checkpoint_begin;
pub mod checkpoint_closure;
pub mod checkpoint_reader;
pub mod checkpoint_refusal;
pub mod checkpoint_selection;
pub mod image_reader;
pub mod metadata_read_gate;
pub mod migration_publisher;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum OwnerKind {
    Build = 1,
    Pending = 2,
    Version = 3,
    Reader = 4,
    Snapshot = 5,
    Migration = 6,
    Backup = 7,
    Legacy = 8,
    Publisher = 9,
    /// Store-bound metadata bootstrap protection. It survives process restarts;
    /// no generic transfer, drain report or release can retire this owner.
    BootstrapReader = 10,
}

/// A random process incarnation, never a PID or an expiring clock lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalLifetime {
    pub node: u64,
    pub store: [u8; 16],
    pub process: [u8; 16],
}

/// Immutable binding for an OwnerId. The same ID can never name a different
/// operation, subject, scope, lifetime or resource closure, even after release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerDescriptor {
    pub root: RootDigest,
    pub id: OwnerId,
    pub kind: OwnerKind,
    pub region: u64,
    pub conf_ver: u64,
    pub version: u64,
    pub operation: [u8; 32],
    /// Exact immutable subject digest. A prepared artifact may precede an
    /// anchor; callers must distinguish it from a complete anchor identity.
    pub subject: [u8; 32],
    pub subject_is_anchor: bool,
    pub local: Option<LocalLifetime>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerBinding {
    pub descriptor: OwnerDescriptor,
    pub generation: NonZeroU64,
    /// Strictly ordered by resource kind and instance; duplicates refuse.
    pub resources: Vec<ResourceIdentity>,
}
impl OwnerBinding {
    pub fn token(&self) -> OwnerToken {
        OwnerToken {
            owner: self.descriptor.id,
            generation: self.generation,
        }
    }

    fn validate(&self, root: RootDigest) -> Result<()> {
        let d = &self.descriptor;
        if d.root != root
            || d.region == 0
            || d.conf_ver == 0
            || d.version == 0
            || d.operation == [0; 32]
            || d.subject == [0; 32]
            || self.resources.is_empty()
            || self.resources.len() > MAX_OWNER_RESOURCES
            || (matches!(d.kind, OwnerKind::Reader | OwnerKind::Publisher) && d.local.is_none())
            || (d.kind == OwnerKind::BootstrapReader
                && (d.region != kv9_common::META_REGION_0.0
                    || d.local.is_some()
                    || d.subject_is_anchor
                    || self.generation.get() != 1))
            || d.local
                .is_some_and(|l| l.node == 0 || l.store == [0; 16] || l.process == [0; 16])
        {
            return Err(invalid("invalid owner binding"));
        }
        validate_resources(&self.resources, root)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerOwner {
    pub binding: OwnerBinding,
    pub phase: PinPhase,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedgerRequest {
    /// Deny new uploads for an exact metadata image after its read fence.
    CloseMetadataImageWrites(OwnerToken),
    /// Bounded catalog coverage for non-anchor Build and migration Publishers.
    AdvanceImagePublisherIndex {
        expected: u64,
        stage: u8,
        after: Option<OwnerId>,
        next: Option<OwnerId>,
    },
    /// Dense generation observation only; never grants resource release.
    AdvanceImagePublisherClosure {
        pending: OwnerToken,
        expected: u64,
    },
    /// Bounded directory of non-anchor Readers across every image operation.
    AdvanceImageReaderIndex {
        expected: u64,
        stage: u8,
        after: Option<OwnerId>,
        next: Option<OwnerId>,
    },
    /// One image-wide generation census page after metadata admission closes.
    AdvanceImageReaderClosure {
        pending: OwnerToken,
        expected: u64,
    },
    /// Close all new catalog Reader/Bootstrap admissions for this exact
    /// metadata image, using an already closed all-store checkpoint read gate.
    CloseMetadataImageReads(OwnerToken),
    /// Census one bounded page of persistent grants for this exact metadata
    /// image after new read admission closes; never grants Version release.
    AdvanceBootstrapClosure {
        pending: OwnerToken,
        expected: u64,
    },
    /// Verify at most four provisioned metadata stores have selected a strictly
    /// later checkpoint; only the final page closes new reads of this attempt.
    AdvanceMetadataReadGate {
        pending: OwnerToken,
        expected: u64,
    },
    /// Derive a persistent bootstrap grant from the exact Published metadata
    /// Version and provisioned store, atomically retaining its admission origin.
    BeginBootstrapRead {
        pending: OwnerToken,
        source: kv9_common::NodeId,
        store: kv9_common::StoreIncarnation,
    },
    /// Requires an independent observation of this store's durable bootstrap
    /// read fence. A caller-supplied drain assertion cannot retire this grant.
    RetireBootstrapRead {
        pending: OwnerToken,
        source: kv9_common::NodeId,
        store: kv9_common::StoreIncarnation,
    },
    /// Actual GET-only installation/recovery reader drained under the exact
    /// still-Published Snapshot/Migration source; grants no source retirement.
    FinishImageRead {
        successor: OwnerToken,
        reader: OwnerBinding,
    },
    /// Atomically admit a process generation for the catalog's exact migration image.
    EnterMigrationPublisher {
        operation: [u8; 16],
        publisher: OwnerBinding,
    },
    /// Actual local jobs drained; preserve original PUT counts and complete coverage.
    FinishMigrationPublisher {
        operation: [u8; 16],
        publisher: OwnerBinding,
        issued: u64,
        resolved: u64,
    },
    /// Request independent observation of a replica's selected checkpoint.
    /// Caller assertions and committed manifest history alone are insufficient.
    RecordCheckpointSelection {
        pending: OwnerToken,
        source: kv9_common::NodeId,
        store: kv9_common::StoreIncarnation,
    },
    /// Independently inspect an exact store's durable prospective DATA floor.
    /// Caller-supplied selection or drain assertions cannot mint this fact.
    RecordDataMembership(data_membership::Request),
    /// Independent exact-store inspection; caller cannot supply origin pages.
    RecordConfigurationOrigins(data_configuration_origins::Request),
    /// Independently query actual ordered configuration states; no caller page.
    RecordConfigurationTimeline(data_configuration_timeline::Request),
    /// Derive one bounded page from already durable actual configurations.
    AdvanceConfigurationIntervals(data_configuration_intervals::Request),
    RecordDataReadFence {
        pending: OwnerToken,
        source: kv9_common::NodeId,
        store: kv9_common::StoreIncarnation,
        creation: RootDigest,
    },
    /// One bounded page from the retained placement catalog at its exact
    /// atomic task revision. Completion never grants all-store read closure.
    AdvanceDataStoreHistory {
        expected: u64,
        revision: u64,
        after: Option<u64>,
    },
    /// Actual local read scope drained. Finish/fence this Reader generation
    /// atomically while its complete Published successor remains protected.
    FinishCheckpointRead {
        successor: OwnerToken,
        reader: OwnerBinding,
    },
    /// The serving metadata leader must independently verify this exact window.
    /// The ordinary metadata planner cannot persist a caller's assertion.
    RecordCheckpointRefusal {
        pending: OwnerToken,
        source: kv9_common::NodeId,
        winner: [u8; 32],
    },
    /// Recompute one bounded closure page; callers cannot supply its findings.
    AdvanceCheckpointClosure {
        pending: OwnerToken,
        expected: u64,
    },
    /// Bounded dense Reader-generation census after an exact refusal.
    AdvanceReaderClosure {
        pending: OwnerToken,
        expected: u64,
    },
    /// Explicit bootstrap of an empty ledger namespace, never automatic repair.
    Initialize,
    /// Registration alone is not a pin or permission to upload/use a resource.
    Register(Vec<ResourceIdentity>),
    Acquire(OwnerBinding),
    /// Register publication eligibility before exposing the actual reference.
    /// This does not install a manifest or prove that a remote object exists.
    Publish(OwnerToken),
    /// Add full destination protection while keeping the source unchanged.
    Share {
        from: OwnerToken,
        to: OwnerBinding,
    },
    /// Stop source publication only while a published successor retains its
    /// exact resource closure and immutable subject. No timeout/death inference.
    QuiesceAfterTransfer {
        from: OwnerToken,
        to: OwnerToken,
    },
    Release(OwnerToken),
    /// Immutable recovery information only: no preparation, admission closure,
    /// manifest settlement or release authority is created by this record.
    RecordCheckpoint(CheckpointIntent),
    /// Atomically create Published Pending, resources, intent and admission.
    /// Existing Pending resumes without manufacturing an atomic-birth record.
    BeginCheckpoint(CheckpointIntent),
    /// Monotone admission fence, not an executor drain or negative certificate.
    CloseCheckpointAdmission(OwnerToken),
    /// Terminal generation for an executor that never reached Held/Published.
    /// Prevents delayed entry after an uncertain pre-append failure.
    CancelUnenteredCheckpoint(OwnerBinding),
    /// Closed gate plus exact tracked Held generation proves Publish cannot
    /// authorize this generation. Global Pending/Version coverage remains.
    RetireUnpublishedCheckpoint {
        pending: OwnerToken,
        executor: OwnerToken,
    },
    /// CAS progress of the bounded Held-only sweep. Completion cannot certify
    /// that Published executors, legacy writers or remote PUTs have drained.
    AdvanceCheckpointSweep {
        pending: OwnerToken,
        expected: u64,
        after: Option<OwnerId>,
        next: Option<OwnerId>,
    },
    /// Trusted executor observation, never reconstructed from Released. Every
    /// generation keeps its own immutable row; unresolved PUTs remain debt.
    RecordCheckpointDrain {
        pending: OwnerToken,
        executor: OwnerToken,
        issued: u64,
        resolved: u64,
    },
    /// Exact locally drained scope: terminal executor pins and its immutable
    /// observation commit together under complete Published Pending/Version
    /// coverage. Unknown remote PUTs remain recorded, not cancelled.
    FinishCheckpointExecution {
        pending: OwnerToken,
        executor: OwnerBinding,
        issued: u64,
        resolved: u64,
    },
    /// Bounded global owner/registration census. This establishes catalog
    /// reverse-index coverage, never executor drain or remote I/O termination.
    AdvanceBuildIndex {
        expected: u64,
        stage: u8,
        after: Option<OwnerId>,
        next: Option<OwnerId>,
    },
    /// Bounded census of persistent bootstrap owners, registrations and origins.
    AdvanceBootstrapIndex {
        expected: u64,
        stage: u8,
        after: Option<OwnerId>,
        next: Option<OwnerId>,
    },
    /// Bounded census of per-process Reader registrations, not a drain proof.
    AdvanceReaderIndex {
        expected: u64,
        stage: u8,
        after: Option<OwnerId>,
        next: Option<OwnerId>,
    },
}

/// Encode untrusted request data, not a prepared/committed operation capability.
pub fn encode_request(root: RootDigest, request: &LedgerRequest) -> Result<Vec<u8>> {
    let mut b = REQUEST_MAGIC.to_vec();
    b.extend_from_slice(root.as_bytes());
    let token = |b: &mut Vec<u8>, t: OwnerToken| {
        b.extend_from_slice(t.owner.as_bytes());
        uint(b, t.generation.get());
    };
    let binding = |b: &mut Vec<u8>, owner: &OwnerBinding| -> Result<()> {
        owner.validate(root)?;
        b.extend_from_slice(&encode_owner(&LedgerOwner {
            binding: owner.clone(),
            phase: PinPhase::Held,
        }));
        Ok(())
    };
    match request {
        LedgerRequest::AdvanceConfigurationIntervals(request) => {
            b.push(41);
            b.extend_from_slice(&request.encode()?);
        }
        LedgerRequest::RecordConfigurationTimeline(request) => {
            b.push(40);
            b.extend_from_slice(&request.encode()?);
        }
        LedgerRequest::RecordConfigurationOrigins(request) => {
            b.push(39);
            b.extend_from_slice(&request.encode()?);
        }
        LedgerRequest::RecordDataMembership(request) => {
            b.push(38);
            b.extend_from_slice(&request.encode()?);
        }
        LedgerRequest::AdvanceDataStoreHistory {
            expected,
            revision,
            after,
        } => {
            if after.is_some_and(|id| id < crate::store::FIRST_DYNAMIC_ID) {
                return Err(invalid("DATA store census task cursor"));
            }
            b.push(37);
            uint(&mut b, *expected);
            uint(&mut b, *revision);
            b.push(u8::from(after.is_some()));
            if let Some(id) = after {
                uint(&mut b, *id);
            }
        }
        LedgerRequest::CloseMetadataImageWrites(pending) => {
            if pending.generation.get() != 1 {
                return Err(invalid("image write gate Pending generation"));
            }
            b.push(33);
            token(&mut b, *pending);
        }
        LedgerRequest::AdvanceImagePublisherClosure { pending, expected } => {
            if pending.generation.get() != 1 {
                return Err(invalid("image Publisher closure Pending generation"));
            }
            b.push(35);
            token(&mut b, *pending);
            uint(&mut b, *expected);
        }
        LedgerRequest::AdvanceImageReaderClosure { pending, expected } => {
            if pending.generation.get() != 1 {
                return Err(invalid("image Reader closure Pending generation"));
            }
            b.push(32);
            token(&mut b, *pending);
            uint(&mut b, *expected);
        }
        LedgerRequest::CloseMetadataImageReads(pending) => {
            if pending.generation.get() != 1 {
                return Err(invalid("metadata image gate Pending generation"));
            }
            b.push(30);
            token(&mut b, *pending);
        }
        LedgerRequest::AdvanceBootstrapClosure { pending, expected } => {
            if pending.generation.get() != 1 {
                return Err(invalid("bootstrap closure Pending generation"));
            }
            b.push(29);
            token(&mut b, *pending);
            uint(&mut b, *expected);
        }
        LedgerRequest::AdvanceMetadataReadGate { pending, expected } => {
            if pending.generation.get() != 1 {
                return Err(invalid("metadata read gate Pending generation"));
            }
            b.push(28);
            token(&mut b, *pending);
            uint(&mut b, *expected);
        }
        LedgerRequest::BeginBootstrapRead {
            pending,
            source,
            store,
        }
        | LedgerRequest::RetireBootstrapRead {
            pending,
            source,
            store,
        } => {
            if pending.generation.get() != 1 || source.0 == 0 || store.as_bytes() == &[0; 16] {
                return Err(invalid("bootstrap request identity"));
            }
            b.push(
                if matches!(request, LedgerRequest::BeginBootstrapRead { .. }) {
                    26
                } else {
                    23
                },
            );
            token(&mut b, *pending);
            uint(&mut b, source.0);
            b.extend_from_slice(store.as_bytes());
        }
        LedgerRequest::FinishImageRead { successor, reader } => {
            b.push(22);
            token(&mut b, *successor);
            binding(&mut b, reader)?;
        }
        LedgerRequest::EnterMigrationPublisher {
            operation,
            publisher,
        } => {
            if operation == &[0; 16] {
                return Err(invalid("migration operation zero"));
            }
            b.push(20);
            b.extend_from_slice(operation);
            binding(&mut b, publisher)?;
        }
        LedgerRequest::FinishMigrationPublisher {
            operation,
            publisher,
            issued,
            resolved,
        } => {
            if operation == &[0; 16] || resolved > issued {
                return Err(invalid("migration drain bounds"));
            }
            b.push(21);
            b.extend_from_slice(operation);
            uint(&mut b, *issued);
            uint(&mut b, *resolved);
            binding(&mut b, publisher)?;
        }
        LedgerRequest::RecordDataReadFence {
            pending,
            source,
            store,
            creation,
        } => {
            if source.0 == 0 || store.as_bytes() == &[0; 16] || creation.as_bytes() == &[0; 32] {
                return Err(invalid("DATA fence source, store or creation"));
            }
            b.push(36);
            token(&mut b, *pending);
            uint(&mut b, source.0);
            b.extend_from_slice(store.as_bytes());
            b.extend_from_slice(creation.as_bytes());
        }
        LedgerRequest::RecordCheckpointSelection {
            pending,
            source,
            store,
        } => {
            if source.0 == 0 || store.as_bytes() == &[0; 16] {
                return Err(invalid("checkpoint selection source or store"));
            }
            b.push(19);
            token(&mut b, *pending);
            uint(&mut b, source.0);
            b.extend_from_slice(store.as_bytes());
        }
        LedgerRequest::FinishCheckpointRead { successor, reader } => {
            b.push(18);
            token(&mut b, *successor);
            binding(&mut b, reader)?;
        }
        LedgerRequest::RecordCheckpointRefusal {
            pending,
            source,
            winner,
        } => {
            if source.0 == 0 || winner == &[0; 32] {
                return Err(invalid("checkpoint refusal source or winner"));
            }
            b.push(17);
            token(&mut b, *pending);
            uint(&mut b, source.0);
            b.extend_from_slice(winner);
        }
        LedgerRequest::AdvanceCheckpointClosure { pending, expected }
        | LedgerRequest::AdvanceReaderClosure { pending, expected } => {
            b.push(
                if matches!(request, LedgerRequest::AdvanceCheckpointClosure { .. }) {
                    16
                } else {
                    25
                },
            );
            token(&mut b, *pending);
            uint(&mut b, *expected);
        }
        LedgerRequest::Initialize => b.push(0),
        LedgerRequest::Register(resources) => {
            validate_resources(resources, root)?;
            b.push(1);
            b.extend_from_slice(&(resources.len() as u32).to_be_bytes());
            for r in resources {
                b.extend_from_slice(&resource_id(r));
                b.extend_from_slice(r.content());
            }
        }
        LedgerRequest::Acquire(owner) => {
            b.push(2);
            binding(&mut b, owner)?;
        }
        LedgerRequest::Publish(t) => {
            b.push(3);
            token(&mut b, *t);
        }
        LedgerRequest::Share { from, to } => {
            b.push(4);
            token(&mut b, *from);
            binding(&mut b, to)?;
        }
        LedgerRequest::QuiesceAfterTransfer { from, to } => {
            b.push(5);
            token(&mut b, *from);
            token(&mut b, *to);
        }
        LedgerRequest::RecordCheckpointDrain {
            pending,
            executor,
            issued,
            resolved,
        } => {
            b.push(12);
            token(&mut b, *pending);
            token(&mut b, *executor);
            uint(&mut b, *issued);
            uint(&mut b, *resolved);
        }
        LedgerRequest::Release(t) => {
            b.push(6);
            token(&mut b, *t);
        }
        LedgerRequest::FinishCheckpointExecution {
            pending,
            executor,
            issued,
            resolved,
        } => {
            b.push(13);
            token(&mut b, *pending);
            uint(&mut b, *issued);
            uint(&mut b, *resolved);
            binding(&mut b, executor)?;
        }
        LedgerRequest::RecordCheckpoint(intent) | LedgerRequest::BeginCheckpoint(intent) => {
            if intent.root() != root {
                return Err(invalid("checkpoint intent root differs"));
            }
            b.push(if matches!(request, LedgerRequest::BeginCheckpoint(_)) {
                15
            } else {
                7
            });
            b.extend_from_slice(&intent.encode());
        }
        LedgerRequest::CloseCheckpointAdmission(t) => {
            b.push(8);
            token(&mut b, *t);
        }
        LedgerRequest::CancelUnenteredCheckpoint(owner) => {
            b.push(9);
            binding(&mut b, owner)?;
        }
        LedgerRequest::RetireUnpublishedCheckpoint { pending, executor } => {
            b.push(10);
            token(&mut b, *pending);
            token(&mut b, *executor);
        }
        LedgerRequest::AdvanceCheckpointSweep {
            pending,
            expected,
            after,
            next,
        } => {
            b.push(11);
            token(&mut b, *pending);
            uint(&mut b, *expected);
            for cursor in [after, next] {
                b.push(u8::from(cursor.is_some()));
                if let Some(id) = cursor {
                    b.extend_from_slice(id.as_bytes());
                }
            }
        }
        LedgerRequest::AdvanceBuildIndex {
            expected,
            stage,
            after,
            next,
        }
        | LedgerRequest::AdvanceReaderIndex {
            expected,
            stage,
            after,
            next,
        }
        | LedgerRequest::AdvanceImagePublisherIndex {
            expected,
            stage,
            after,
            next,
        }
        | LedgerRequest::AdvanceImageReaderIndex {
            expected,
            stage,
            after,
            next,
        }
        | LedgerRequest::AdvanceBootstrapIndex {
            expected,
            stage,
            after,
            next,
        } => {
            let bootstrap = matches!(request, LedgerRequest::AdvanceBootstrapIndex { .. });
            if *stage > if bootstrap { 3 } else { 1 } {
                return Err(invalid("owner index stage"));
            }
            b.push(
                if matches!(request, LedgerRequest::AdvanceBuildIndex { .. }) {
                    14
                } else if matches!(request, LedgerRequest::AdvanceImageReaderIndex { .. }) {
                    31
                } else if matches!(request, LedgerRequest::AdvanceImagePublisherIndex { .. }) {
                    34
                } else if bootstrap {
                    27
                } else {
                    24
                },
            );
            uint(&mut b, *expected);
            b.push(*stage);
            for cursor in [after, next] {
                b.push(u8::from(cursor.is_some()));
                if let Some(id) = cursor {
                    b.extend_from_slice(id.as_bytes());
                }
            }
        }
    }
    let limit = if matches!(
        request,
        LedgerRequest::RecordCheckpoint(_) | LedgerRequest::BeginCheckpoint(_)
    ) {
        MAX_LEDGER_REQUEST_BYTES
    } else {
        MAX_SMALL_REQUEST_BYTES
    };
    if root.as_bytes() == &[0; 32] || b.len() + 32 > limit {
        return Err(invalid("request root or byte bound"));
    }
    Ok(seal(b))
}

pub fn decode_request(bytes: &[u8], root: RootDigest) -> Result<LedgerRequest> {
    let mut r = Reader(checked(bytes, REQUEST_MAGIC, MAX_LEDGER_REQUEST_BYTES)?);
    if &r.array::<32>()? != root.as_bytes() || root.as_bytes() == &[0; 32] {
        return Err(invalid("request root differs"));
    }
    let token = |r: &mut Reader<'_>| -> Result<OwnerToken> {
        Ok(OwnerToken {
            owner: OwnerId::new(r.array()?).map_err(pin_error)?,
            generation: NonZeroU64::new(r.uint()?)
                .ok_or_else(|| invalid("zero request generation"))?,
        })
    };
    let binding = |r: &mut Reader<'_>| -> Result<OwnerBinding> {
        let bytes = r.take(r.0.len())?;
        if bytes.len() < 56 {
            return Err(invalid("truncated request owner"));
        }
        let id = OwnerId::new(bytes[40..56].try_into().unwrap()).map_err(pin_error)?;
        let owner = decode_owner(bytes, root, id)?;
        if owner.phase != PinPhase::Held {
            return Err(invalid("request cannot supply its own owner phase"));
        }
        Ok(owner.binding)
    };
    let request = match r.byte()? {
        41 => LedgerRequest::AdvanceConfigurationIntervals(
            data_configuration_intervals::Request::decode(&mut r)?,
        ),
        40 => LedgerRequest::RecordConfigurationTimeline(
            data_configuration_timeline::Request::decode(&mut r)?,
        ),
        39 => LedgerRequest::RecordConfigurationOrigins(
            data_configuration_origins::Request::decode(&mut r)?,
        ),
        38 => LedgerRequest::RecordDataMembership(data_membership::Request::decode(&mut r)?),
        37 => LedgerRequest::AdvanceDataStoreHistory {
            expected: r.uint()?,
            revision: r.uint()?,
            after: if r.boolean()? { Some(r.uint()?) } else { None },
        },
        33 => LedgerRequest::CloseMetadataImageWrites(token(&mut r)?),
        35 => LedgerRequest::AdvanceImagePublisherClosure {
            pending: token(&mut r)?,
            expected: r.uint()?,
        },
        32 => LedgerRequest::AdvanceImageReaderClosure {
            pending: token(&mut r)?,
            expected: r.uint()?,
        },
        34 => LedgerRequest::AdvanceImagePublisherIndex {
            expected: r.uint()?,
            stage: r.byte()?,
            after: if r.boolean()? {
                Some(OwnerId::new(r.array()?).map_err(pin_error)?)
            } else {
                None
            },
            next: if r.boolean()? {
                Some(OwnerId::new(r.array()?).map_err(pin_error)?)
            } else {
                None
            },
        },
        31 => LedgerRequest::AdvanceImageReaderIndex {
            expected: r.uint()?,
            stage: r.byte()?,
            after: if r.boolean()? {
                Some(OwnerId::new(r.array()?).map_err(pin_error)?)
            } else {
                None
            },
            next: if r.boolean()? {
                Some(OwnerId::new(r.array()?).map_err(pin_error)?)
            } else {
                None
            },
        },
        30 => LedgerRequest::CloseMetadataImageReads(token(&mut r)?),
        29 => LedgerRequest::AdvanceBootstrapClosure {
            pending: token(&mut r)?,
            expected: r.uint()?,
        },
        28 => LedgerRequest::AdvanceMetadataReadGate {
            pending: token(&mut r)?,
            expected: r.uint()?,
        },
        26 => LedgerRequest::BeginBootstrapRead {
            pending: token(&mut r)?,
            source: kv9_common::NodeId(r.uint()?),
            store: kv9_common::StoreIncarnation::from_bytes(r.array()?),
        },
        23 => LedgerRequest::RetireBootstrapRead {
            pending: token(&mut r)?,
            source: kv9_common::NodeId(r.uint()?),
            store: kv9_common::StoreIncarnation::from_bytes(r.array()?),
        },
        22 => LedgerRequest::FinishImageRead {
            successor: token(&mut r)?,
            reader: binding(&mut r)?,
        },
        20 => LedgerRequest::EnterMigrationPublisher {
            operation: r.array()?,
            publisher: binding(&mut r)?,
        },
        21 => LedgerRequest::FinishMigrationPublisher {
            operation: r.array()?,
            issued: r.uint()?,
            resolved: r.uint()?,
            publisher: binding(&mut r)?,
        },
        36 => LedgerRequest::RecordDataReadFence {
            pending: token(&mut r)?,
            source: kv9_common::NodeId(r.uint()?),
            store: kv9_common::StoreIncarnation::from_bytes(r.array()?),
            creation: RootDigest::from_bytes(r.array()?),
        },
        19 => LedgerRequest::RecordCheckpointSelection {
            pending: token(&mut r)?,
            source: kv9_common::NodeId(r.uint()?),
            store: kv9_common::StoreIncarnation::from_bytes(r.array()?),
        },
        18 => LedgerRequest::FinishCheckpointRead {
            successor: token(&mut r)?,
            reader: binding(&mut r)?,
        },
        0 => LedgerRequest::Initialize,
        1 => {
            let count = u32::from_be_bytes(r.array()?) as usize;
            if count == 0 || count > MAX_OWNER_RESOURCES || r.0.len() != count * 49 {
                return Err(invalid("request resource count"));
            }
            let mut resources = Vec::with_capacity(count);
            for _ in 0..count {
                resources.push(
                    ResourceIdentity::new(root, resource_kind(r.byte()?)?, r.array()?, r.array()?)
                        .map_err(pin_error)?,
                );
            }
            validate_resources(&resources, root)?;
            LedgerRequest::Register(resources)
        }
        2 => LedgerRequest::Acquire(binding(&mut r)?),
        3 => LedgerRequest::Publish(token(&mut r)?),
        4 => LedgerRequest::Share {
            from: token(&mut r)?,
            to: binding(&mut r)?,
        },
        5 => LedgerRequest::QuiesceAfterTransfer {
            from: token(&mut r)?,
            to: token(&mut r)?,
        },
        6 => LedgerRequest::Release(token(&mut r)?),
        7 => {
            let bytes = r.take(r.0.len())?;
            LedgerRequest::RecordCheckpoint(CheckpointIntent::decode_request(bytes, root)?)
        }
        15 => {
            let bytes = r.take(r.0.len())?;
            LedgerRequest::BeginCheckpoint(CheckpointIntent::decode_request(bytes, root)?)
        }
        16 => LedgerRequest::AdvanceCheckpointClosure {
            pending: token(&mut r)?,
            expected: r.uint()?,
        },
        25 => LedgerRequest::AdvanceReaderClosure {
            pending: token(&mut r)?,
            expected: r.uint()?,
        },
        17 => LedgerRequest::RecordCheckpointRefusal {
            pending: token(&mut r)?,
            source: kv9_common::NodeId(r.uint()?),
            winner: r.array()?,
        },
        8 => LedgerRequest::CloseCheckpointAdmission(token(&mut r)?),
        9 => LedgerRequest::CancelUnenteredCheckpoint(binding(&mut r)?),
        10 => LedgerRequest::RetireUnpublishedCheckpoint {
            pending: token(&mut r)?,
            executor: token(&mut r)?,
        },
        11 => LedgerRequest::AdvanceCheckpointSweep {
            pending: token(&mut r)?,
            expected: r.uint()?,
            after: if r.boolean()? {
                Some(OwnerId::new(r.array()?).map_err(pin_error)?)
            } else {
                None
            },
            next: if r.boolean()? {
                Some(OwnerId::new(r.array()?).map_err(pin_error)?)
            } else {
                None
            },
        },
        12 => LedgerRequest::RecordCheckpointDrain {
            pending: token(&mut r)?,
            executor: token(&mut r)?,
            issued: r.uint()?,
            resolved: r.uint()?,
        },
        13 => LedgerRequest::FinishCheckpointExecution {
            pending: token(&mut r)?,
            issued: r.uint()?,
            resolved: r.uint()?,
            executor: binding(&mut r)?,
        },
        14 => LedgerRequest::AdvanceBuildIndex {
            expected: r.uint()?,
            stage: r.byte()?,
            after: if r.boolean()? {
                Some(OwnerId::new(r.array()?).map_err(pin_error)?)
            } else {
                None
            },
            next: if r.boolean()? {
                Some(OwnerId::new(r.array()?).map_err(pin_error)?)
            } else {
                None
            },
        },
        24 => LedgerRequest::AdvanceReaderIndex {
            expected: r.uint()?,
            stage: r.byte()?,
            after: if r.boolean()? {
                Some(OwnerId::new(r.array()?).map_err(pin_error)?)
            } else {
                None
            },
            next: if r.boolean()? {
                Some(OwnerId::new(r.array()?).map_err(pin_error)?)
            } else {
                None
            },
        },
        27 => LedgerRequest::AdvanceBootstrapIndex {
            expected: r.uint()?,
            stage: r.byte()?,
            after: if r.boolean()? {
                Some(OwnerId::new(r.array()?).map_err(pin_error)?)
            } else {
                None
            },
            next: if r.boolean()? {
                Some(OwnerId::new(r.array()?).map_err(pin_error)?)
            } else {
                None
            },
        },
        _ => return Err(invalid("unknown retention request operation")),
    };
    if !r.0.is_empty() || encode_request(root, &request)? != bytes {
        return Err(invalid("request has noncanonical or trailing data"));
    }
    Ok(request)
}

/// Serialize a plain observation for admin readback; no authority is minted.
pub fn encode_owner_observation(owner: &LedgerOwner) -> Result<Vec<u8>> {
    owner.binding.validate(owner.binding.descriptor.root)?;
    Ok(encode_owner(owner))
}

/// Validate returned data against the requested root and owner identity. A
/// successful decode does not turn an observation into a live-use capability.
pub fn decode_owner_observation(
    bytes: &[u8],
    root: RootDigest,
    id: OwnerId,
) -> Result<LedgerOwner> {
    decode_owner(bytes, root, id)
}

/// A proposed atomic effect. This is deliberately not a committed capability.
pub struct LedgerPlan {
    batch: WriteBatch,
    revision: u64,
    changed: bool,
}
impl LedgerPlan {
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn changed(&self) -> bool {
        self.changed
    }
    pub fn into_batch(self) -> WriteBatch {
        self.batch
    }
}

fn invalid(reason: &str) -> Error {
    Error::Engine(format!("retention ledger: {reason}"))
}
fn conflict(reason: &str) -> Error {
    Error::WriteConflict(format!("retention ledger: {reason}"))
}
fn pin_error(error: impl std::fmt::Display) -> Error {
    invalid(&error.to_string())
}
fn key(tag: u8, id: &[u8]) -> Vec<u8> {
    let mut k = PREFIX.to_vec();
    k.push(tag);
    k.extend_from_slice(id);
    k
}
fn resource_id(r: &ResourceIdentity) -> [u8; 17] {
    let mut id = [0; 17];
    id[0] = r.kind() as u8;
    id[1..].copy_from_slice(r.instance());
    id
}
fn resource_key(r: &ResourceIdentity) -> Vec<u8> {
    key(2, &resource_id(r))
}
fn resource_registration_key(r: &ResourceIdentity) -> Vec<u8> {
    key(4, &resource_id(r))
}
fn resource_registration(r: &ResourceIdentity) -> Vec<u8> {
    let mut b = b"KV9REG01".to_vec();
    b.extend_from_slice(r.root().as_bytes());
    b.extend_from_slice(&resource_id(r));
    b.extend_from_slice(r.content());
    seal(b)
}
fn owner_key(id: OwnerId) -> Vec<u8> {
    key(3, id.as_bytes())
}
fn owner_registration_key(id: OwnerId) -> Vec<u8> {
    key(5, id.as_bytes())
}
fn owner_registration(binding: &OwnerBinding) -> Vec<u8> {
    let mut binding = binding.clone();
    binding.generation = NonZeroU64::new(1).unwrap();
    encode_owner(&LedgerOwner {
        binding,
        phase: PinPhase::Held,
    })
}
fn validate_resources(resources: &[ResourceIdentity], root: RootDigest) -> Result<()> {
    if resources.is_empty()
        || resources.len() > MAX_OWNER_RESOURCES
        || resources.iter().any(|r| r.root() != root)
        || resources
            .windows(2)
            .any(|p| resource_id(&p[0]) >= resource_id(&p[1]))
    {
        return Err(invalid("resource root, count or canonical order"));
    }
    Ok(())
}
fn seal(mut b: Vec<u8>) -> Vec<u8> {
    b.extend_from_slice(RootDigest::sha256(&b).as_bytes());
    b
}
fn checked<'a>(b: &'a [u8], magic: &[u8], max: usize) -> Result<&'a [u8]> {
    if b.len() < magic.len() + 32 || b.len() > max || !b.starts_with(magic) {
        return Err(invalid("record format or length"));
    }
    let end = b.len() - 32;
    if RootDigest::sha256(&b[..end]).as_bytes() != &b[end..] {
        return Err(invalid("record checksum"));
    }
    Ok(&b[magic.len()..end])
}
fn header(root: RootDigest, revision: u64) -> Vec<u8> {
    let mut b = HEADER_MAGIC.to_vec();
    b.extend_from_slice(root.as_bytes());
    b.push(TRACKING_ONLY);
    b.extend_from_slice(&revision.to_be_bytes());
    seal(b)
}
// V8 introduced migration publisher generations. Newer formats retain them.
fn bootstrap_index_format(bytes: &[u8]) -> bool {
    metadata_selection_format(bytes) || bytes.starts_with(BOOTSTRAP_INDEX_HEADER_MAGIC)
}
fn metadata_selection_format(bytes: &[u8]) -> bool {
    metadata_read_gate_format(bytes) || bytes.starts_with(METADATA_SELECTION_HEADER_MAGIC)
}
fn metadata_read_gate_format(bytes: &[u8]) -> bool {
    bootstrap_closure_format(bytes) || bytes.starts_with(METADATA_READ_GATE_HEADER_MAGIC)
}
fn bootstrap_closure_format(bytes: &[u8]) -> bool {
    bootstrap_unentered_format(bytes) || bytes.starts_with(BOOTSTRAP_CLOSURE_HEADER_MAGIC)
}
fn image_reader_format(bytes: &[u8]) -> bool {
    image_publisher_format(bytes) || bytes.starts_with(IMAGE_READER_HEADER_MAGIC)
}
fn image_publisher_format(bytes: &[u8]) -> bool {
    data_fence_format(bytes) || bytes.starts_with(IMAGE_PUBLISHER_HEADER_MAGIC)
}
fn data_fence_format(bytes: &[u8]) -> bool {
    bytes.starts_with(HEADER_MAGIC)
        || bytes.starts_with(DATA_TIMELINE_HEADER_MAGIC)
        || bytes.starts_with(DATA_INITIAL_HEADER_MAGIC)
        || bytes.starts_with(DATA_ORIGINS_HEADER_MAGIC)
        || bytes.starts_with(DATA_MEMBERSHIP_HEADER_MAGIC)
        || bytes.starts_with(DATA_HISTORY_HEADER_MAGIC)
        || bytes.starts_with(DATA_FENCE_HEADER_MAGIC)
}
fn metadata_image_gate_format(bytes: &[u8]) -> bool {
    image_reader_format(bytes) || bytes.starts_with(METADATA_IMAGE_GATE_HEADER_MAGIC)
}
fn bootstrap_unentered_format(bytes: &[u8]) -> bool {
    metadata_image_gate_format(bytes) || bytes.starts_with(BOOTSTRAP_UNENTERED_HEADER_MAGIC)
}
fn bootstrap_admission_format(bytes: &[u8]) -> bool {
    bootstrap_index_format(bytes) || bytes.starts_with(BOOTSTRAP_ADMISSION_HEADER_MAGIC)
}
fn reader_closure_format(bytes: &[u8]) -> bool {
    bootstrap_admission_format(bytes) || bytes.starts_with(READER_CLOSURE_HEADER_MAGIC)
}
fn reader_generation_format(bytes: &[u8]) -> bool {
    reader_closure_format(bytes) || bytes.starts_with(READER_GENERATION_HEADER_MAGIC)
}
fn migration_format(bytes: &[u8]) -> bool {
    reader_generation_format(bytes)
        || bytes.starts_with(READER_INDEX_HEADER_MAGIC)
        || bytes.starts_with(MIGRATION_HEADER_MAGIC)
}
fn check_upgrade(view: &dyn ReadView, stored: &[u8]) -> Result<()> {
    data_configuration_intervals::check_old_format(view)?;
    if stored.starts_with(DATA_TIMELINE_HEADER_MAGIC) {
        return Ok(());
    }
    data_configuration_timeline::check_old_format(view)?;
    if stored.starts_with(DATA_INITIAL_HEADER_MAGIC) {
        return Ok(());
    }
    data_configuration_origins::initial::check_old_format(view)?;
    if stored.starts_with(DATA_ORIGINS_HEADER_MAGIC) {
        return Ok(());
    }
    data_configuration_origins::check_old_format(view)?;
    if stored.starts_with(DATA_MEMBERSHIP_HEADER_MAGIC) {
        return Ok(());
    }
    data_membership::check_old_format(view)?;
    if stored.starts_with(DATA_HISTORY_HEADER_MAGIC) {
        return Ok(());
    }
    data_store_history::check_old_format(view)?;
    if stored.starts_with(DATA_FENCE_HEADER_MAGIC) {
        return Ok(());
    }
    data_read_fence::check_old_format(view)?;
    if stored.starts_with(IMAGE_PUBLISHER_HEADER_MAGIC) {
        return Ok(());
    }
    metadata_image_write_gate::check_old_format(view)?;
    image_publisher_index::check_old_format(view)?;
    image_publisher_closure::check_old_format(view)?;
    if stored.starts_with(IMAGE_READER_HEADER_MAGIC) {
        return Ok(());
    }
    image_reader_index::check_old_format(view)?;
    image_reader_closure::check_old_format(view)?;
    if stored.starts_with(METADATA_IMAGE_GATE_HEADER_MAGIC) {
        return Ok(());
    }
    metadata_image_gate::check_old_format(view)?;
    if stored.starts_with(BOOTSTRAP_UNENTERED_HEADER_MAGIC) {
        return Ok(());
    }
    bootstrap_reader::unentered::check_old_format(view)?;
    bootstrap_index::check_old_unentered_format(view)?;
    if stored.starts_with(BOOTSTRAP_CLOSURE_HEADER_MAGIC) {
        return Ok(());
    }
    bootstrap_closure::check_old_format(view)?;
    if stored.starts_with(METADATA_READ_GATE_HEADER_MAGIC) {
        return Ok(());
    }
    metadata_read_gate::check_old_format(view)?;
    if stored.starts_with(METADATA_SELECTION_HEADER_MAGIC) {
        return Ok(());
    }
    checkpoint_selection::check_old_metadata_format(view)?;
    if stored.starts_with(BOOTSTRAP_INDEX_HEADER_MAGIC) {
        return Ok(());
    }
    bootstrap_index::check_old_format(view)?;
    if stored.starts_with(BOOTSTRAP_ADMISSION_HEADER_MAGIC) {
        return Ok(());
    }
    bootstrap_reader::admission::check_old_format(view)?;
    if stored.starts_with(READER_CLOSURE_HEADER_MAGIC) {
        return Ok(());
    }
    reader_closure::check_old_format(view)?;
    if stored.starts_with(READER_GENERATION_HEADER_MAGIC) {
        return Ok(());
    }
    reader_lifecycle::check_old_format(view)?;
    if stored.starts_with(READER_INDEX_HEADER_MAGIC) {
        return Ok(());
    }
    reader_index::check_old_format(view)?;
    if !stored.starts_with(MIGRATION_HEADER_MAGIC) {
        migration_publisher::check_old_format(view)?;
        if !stored.starts_with(CLOSURE_HEADER_MAGIC) {
            checkpoint_closure::check_old_format(view, stored)?;
        }
    }
    Ok(())
}
fn read_header(b: &[u8], root: RootDigest) -> Result<(u64, bool)> {
    let rows_enabled = migration_format(b)
        || b.starts_with(CLOSURE_HEADER_MAGIC)
        || b.starts_with(ATOMIC_HEADER_MAGIC)
        || b.starts_with(ENROLLMENT_HEADER_MAGIC)
        || b.starts_with(BUILD_INDEX_HEADER_MAGIC)
        || b.starts_with(GATED_HEADER_MAGIC)
        || b.starts_with(ROWS_HEADER_MAGIC);
    let magic = if b.starts_with(HEADER_MAGIC) {
        HEADER_MAGIC
    } else if b.starts_with(DATA_TIMELINE_HEADER_MAGIC) {
        DATA_TIMELINE_HEADER_MAGIC
    } else if b.starts_with(DATA_INITIAL_HEADER_MAGIC) {
        DATA_INITIAL_HEADER_MAGIC
    } else if b.starts_with(DATA_ORIGINS_HEADER_MAGIC) {
        DATA_ORIGINS_HEADER_MAGIC
    } else if b.starts_with(DATA_MEMBERSHIP_HEADER_MAGIC) {
        DATA_MEMBERSHIP_HEADER_MAGIC
    } else if b.starts_with(DATA_HISTORY_HEADER_MAGIC) {
        DATA_HISTORY_HEADER_MAGIC
    } else if b.starts_with(DATA_FENCE_HEADER_MAGIC) {
        DATA_FENCE_HEADER_MAGIC
    } else if b.starts_with(IMAGE_PUBLISHER_HEADER_MAGIC) {
        IMAGE_PUBLISHER_HEADER_MAGIC
    } else if b.starts_with(IMAGE_READER_HEADER_MAGIC) {
        IMAGE_READER_HEADER_MAGIC
    } else if b.starts_with(METADATA_IMAGE_GATE_HEADER_MAGIC) {
        METADATA_IMAGE_GATE_HEADER_MAGIC
    } else if b.starts_with(BOOTSTRAP_UNENTERED_HEADER_MAGIC) {
        BOOTSTRAP_UNENTERED_HEADER_MAGIC
    } else if b.starts_with(BOOTSTRAP_CLOSURE_HEADER_MAGIC) {
        BOOTSTRAP_CLOSURE_HEADER_MAGIC
    } else if b.starts_with(METADATA_READ_GATE_HEADER_MAGIC) {
        METADATA_READ_GATE_HEADER_MAGIC
    } else if b.starts_with(METADATA_SELECTION_HEADER_MAGIC) {
        METADATA_SELECTION_HEADER_MAGIC
    } else if b.starts_with(BOOTSTRAP_INDEX_HEADER_MAGIC) {
        BOOTSTRAP_INDEX_HEADER_MAGIC
    } else if b.starts_with(BOOTSTRAP_ADMISSION_HEADER_MAGIC) {
        BOOTSTRAP_ADMISSION_HEADER_MAGIC
    } else if b.starts_with(READER_CLOSURE_HEADER_MAGIC) {
        READER_CLOSURE_HEADER_MAGIC
    } else if b.starts_with(READER_GENERATION_HEADER_MAGIC) {
        READER_GENERATION_HEADER_MAGIC
    } else if b.starts_with(READER_INDEX_HEADER_MAGIC) {
        READER_INDEX_HEADER_MAGIC
    } else if b.starts_with(MIGRATION_HEADER_MAGIC) {
        MIGRATION_HEADER_MAGIC
    } else if b.starts_with(CLOSURE_HEADER_MAGIC) {
        CLOSURE_HEADER_MAGIC
    } else if b.starts_with(ATOMIC_HEADER_MAGIC) {
        ATOMIC_HEADER_MAGIC
    } else if b.starts_with(ENROLLMENT_HEADER_MAGIC) {
        ENROLLMENT_HEADER_MAGIC
    } else if b.starts_with(BUILD_INDEX_HEADER_MAGIC) {
        BUILD_INDEX_HEADER_MAGIC
    } else if b.starts_with(GATED_HEADER_MAGIC) {
        GATED_HEADER_MAGIC
    } else if rows_enabled {
        ROWS_HEADER_MAGIC
    } else {
        LEGACY_HEADER_MAGIC
    };
    let b = checked(b, magic, 81)?;
    if b.len() != 41 || &b[..32] != root.as_bytes() || b[32] != TRACKING_ONLY {
        return Err(invalid("unsupported ledger root or coverage mode"));
    }
    Ok((
        u64::from_be_bytes(b[33..41].try_into().unwrap()),
        rows_enabled,
    ))
}
fn uint(b: &mut Vec<u8>, n: u64) {
    b.extend_from_slice(&n.to_be_bytes());
}
fn resource_kind(kind: u8) -> Result<ResourceKind> {
    match kind {
        1 => Ok(ResourceKind::Sst),
        2 => Ok(ResourceKind::Manifest),
        3 => Ok(ResourceKind::WalSegment),
        4 => Ok(ResourceKind::Snapshot),
        5 => Ok(ResourceKind::HistoryEvidence),
        6 => Ok(ResourceKind::Backup),
        _ => Err(invalid("unknown resource kind")),
    }
}
fn encode_owner(owner: &LedgerOwner) -> Vec<u8> {
    let d = &owner.binding.descriptor;
    let mut b = OWNER_MAGIC.to_vec();
    b.extend_from_slice(d.root.as_bytes());
    b.extend_from_slice(d.id.as_bytes());
    b.push(d.kind as u8);
    for n in [d.region, d.conf_ver, d.version] {
        uint(&mut b, n);
    }
    b.extend_from_slice(&d.operation);
    b.extend_from_slice(&d.subject);
    b.push(u8::from(d.subject_is_anchor));
    b.push(u8::from(d.local.is_some()));
    if let Some(l) = d.local {
        uint(&mut b, l.node);
        b.extend_from_slice(&l.store);
        b.extend_from_slice(&l.process);
    }
    uint(&mut b, owner.binding.generation.get());
    b.push(owner.phase as u8);
    b.extend_from_slice(&(owner.binding.resources.len() as u32).to_be_bytes());
    for r in &owner.binding.resources {
        b.push(r.kind() as u8);
        b.extend_from_slice(r.instance());
        b.extend_from_slice(r.content());
    }
    seal(b)
}
struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if n > self.0.len() {
            return Err(invalid("truncated owner record"));
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        Ok(self.take(N)?.try_into().unwrap())
    }
    fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn uint(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.array()?))
    }
    fn boolean(&mut self) -> Result<bool> {
        match self.byte()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(invalid("noncanonical boolean")),
        }
    }
}
fn decode_owner(bytes: &[u8], root: RootDigest, id: OwnerId) -> Result<LedgerOwner> {
    let executor = bytes.starts_with(EXECUTOR_OWNER_MAGIC);
    let managed_reader = bytes.starts_with(reader_lifecycle::OWNER_MAGIC);
    let mut r = Reader(checked(
        bytes,
        if executor {
            EXECUTOR_OWNER_MAGIC
        } else if bytes.starts_with(bootstrap_reader::admission::OWNER_MAGIC) {
            bootstrap_reader::admission::OWNER_MAGIC
        } else if bytes.starts_with(bootstrap_reader::unentered::OWNER_MAGIC) {
            bootstrap_reader::unentered::OWNER_MAGIC
        } else if managed_reader {
            reader_lifecycle::OWNER_MAGIC
        } else {
            OWNER_MAGIC
        },
        MAX_OWNER_BYTES,
    )?);
    let stored_root = RootDigest::from_bytes(r.array()?);
    let stored_id = OwnerId::new(r.array()?).map_err(pin_error)?;
    if stored_root != root || stored_id != id {
        return Err(invalid("owner key/identity mismatch"));
    }
    let kind = match r.byte()? {
        1 => OwnerKind::Build,
        2 => OwnerKind::Pending,
        3 => OwnerKind::Version,
        4 => OwnerKind::Reader,
        5 => OwnerKind::Snapshot,
        6 => OwnerKind::Migration,
        7 => OwnerKind::Backup,
        8 => OwnerKind::Legacy,
        9 => OwnerKind::Publisher,
        10 => OwnerKind::BootstrapReader,
        _ => return Err(invalid("unknown owner kind")),
    };
    let region = r.uint()?;
    let conf_ver = r.uint()?;
    let version = r.uint()?;
    let operation = r.array()?;
    let subject = r.array()?;
    let subject_is_anchor = r.boolean()?;
    let local = if r.boolean()? {
        Some(LocalLifetime {
            node: r.uint()?,
            store: r.array()?,
            process: r.array()?,
        })
    } else {
        None
    };
    let generation = NonZeroU64::new(r.uint()?).ok_or_else(|| invalid("zero owner generation"))?;
    let phase = match r.byte()? {
        1 => PinPhase::Held,
        2 => PinPhase::Published,
        3 => PinPhase::Quiesced,
        4 => PinPhase::Released,
        _ => return Err(invalid("unknown owner phase")),
    };
    let count = u32::from_be_bytes(r.array()?) as usize;
    if count == 0 || count > MAX_OWNER_RESOURCES || r.0.len() != count * 49 {
        return Err(invalid("owner resource count"));
    }
    let mut resources = Vec::with_capacity(count);
    for _ in 0..count {
        let kind = match r.byte()? {
            1 => ResourceKind::Sst,
            2 => ResourceKind::Manifest,
            3 => ResourceKind::WalSegment,
            4 => ResourceKind::Snapshot,
            5 => ResourceKind::HistoryEvidence,
            6 => ResourceKind::Backup,
            _ => return Err(invalid("unknown resource kind")),
        };
        resources
            .push(ResourceIdentity::new(root, kind, r.array()?, r.array()?).map_err(pin_error)?);
    }
    let owner = LedgerOwner {
        binding: OwnerBinding {
            descriptor: OwnerDescriptor {
                root,
                id,
                kind,
                region,
                conf_ver,
                version,
                operation,
                subject,
                subject_is_anchor,
                local,
            },
            generation,
            resources,
        },
        phase,
    };
    owner.binding.validate(root)?;
    let canonical = if executor {
        checkpoint_admission::encode_executor(&owner)
    } else if bytes.starts_with(bootstrap_reader::admission::OWNER_MAGIC)
        && owner.binding.descriptor.kind == OwnerKind::BootstrapReader
    {
        bootstrap_reader::admission::encode(&owner)
    } else if bytes.starts_with(bootstrap_reader::unentered::OWNER_MAGIC)
        && owner.binding.descriptor.kind == OwnerKind::BootstrapReader
    {
        bootstrap_reader::unentered::encode(&owner)
    } else if managed_reader && owner.binding.descriptor.kind == OwnerKind::Reader {
        reader_lifecycle::encode(&owner)
    } else {
        encode_owner(&owner)
    };
    if canonical != bytes {
        return Err(invalid("noncanonical owner record"));
    }
    Ok(owner)
}

struct Planner<'a> {
    view: &'a dyn ReadView,
    root: RootDigest,
    revision: u64,
    rows_enabled: bool,
    writes: BTreeMap<Vec<u8>, Vec<u8>>,
}
impl Planner<'_> {
    fn register_resource(&mut self, identity: ResourceIdentity) -> Result<ResourceRecord> {
        let record = self
            .view
            .get(ColumnFamily::Default, &resource_key(&identity))?;
        let registered = self
            .view
            .get(ColumnFamily::Default, &resource_registration_key(&identity))?;
        match (record, registered) {
            (Some(_), Some(_)) => {
                let record = self.resource(identity)?;
                if record.is_retired() {
                    return Err(conflict("resource instance permanently retired"));
                }
                record.normalize(&mut self.writes);
                Ok(record)
            }
            (None, None) => {
                ResourceRecord::check_empty(self.view, identity)?;
                let record = ResourceRecord::new(identity);
                self.writes.insert(resource_key(&identity), record.encode());
                self.writes.insert(
                    resource_registration_key(&identity),
                    resource_registration(&identity),
                );
                Ok(record)
            }
            _ => Err(invalid("resource registration/state pair is incomplete")),
        }
    }
    fn resource(&self, identity: ResourceIdentity) -> Result<ResourceRecord> {
        let registered = self
            .view
            .get(ColumnFamily::Default, &resource_registration_key(&identity))?
            .ok_or_else(|| invalid("resource registration missing"))?;
        if registered != resource_registration(&identity) {
            return Err(invalid("resource registration identity disagrees"));
        }
        let value = self
            .view
            .get(ColumnFamily::Default, &resource_key(&identity))?
            .ok_or_else(|| invalid("registered resource missing; refusing empty reconstruction"))?;
        ResourceRecord::load(self.view, identity, &value, self.rows_enabled)
    }
    fn owner(&self, id: OwnerId) -> Result<Option<LedgerOwner>> {
        let value = self.view.get(ColumnFamily::Default, &owner_key(id))?;
        let registered = self
            .view
            .get(ColumnFamily::Default, &owner_registration_key(id))?;
        let (value, registered) = match (value, registered) {
            (None, None) => return Ok(None),
            (Some(value), Some(registered)) => (value, registered),
            _ => return Err(invalid("owner registration/state pair is incomplete")),
        };
        let owner = decode_owner(&value, self.root, id)?;
        let expected_registration = if value.starts_with(EXECUTOR_OWNER_MAGIC) {
            checkpoint_admission::executor_registration(&owner.binding)
        } else if value.starts_with(bootstrap_reader::admission::OWNER_MAGIC) {
            bootstrap_reader::admission::registration(&owner.binding)
        } else if value.starts_with(bootstrap_reader::unentered::OWNER_MAGIC) {
            bootstrap_reader::unentered::registration(&owner.binding)
        } else if value.starts_with(reader_lifecycle::OWNER_MAGIC) {
            reader_lifecycle::registration(&owner.binding)
        } else {
            owner_registration(&owner.binding)
        };
        if registered != expected_registration {
            return Err(invalid("owner immutable registration disagrees"));
        }
        // The owner row and every forward pin are one durable transaction.
        // Missing or conflicting cross-links cannot be interpreted as no uses.
        for resource in &owner.binding.resources {
            let record = self.resource(*resource)?;
            let slot = record
                .owner(self.view, id)?
                .ok_or_else(|| invalid("owner resource link missing"))?;
            if slot.generation != owner.binding.generation || slot.phase != owner.phase {
                return Err(invalid("owner resource link disagrees"));
            }
        }
        if value.starts_with(EXECUTOR_OWNER_MAGIC) {
            checkpoint_admission::check_index(self, &owner)?;
        }
        image_publisher_index::check(self, &owner.binding)?;
        bootstrap_index::check(self, &owner.binding)?;
        build_index::check(self, &owner.binding)?;
        reader_index::check(self, &owner.binding)?;
        reader_lifecycle::check(
            self,
            &owner,
            value.starts_with(reader_lifecycle::OWNER_MAGIC),
        )?;
        checkpoint_admission::enrollment::check(self, &owner)?;
        migration_publisher::check(self, &owner)?;
        bootstrap_reader::admission::check(
            self,
            &owner,
            value.starts_with(bootstrap_reader::admission::OWNER_MAGIC),
        )?;
        bootstrap_reader::unentered::check(
            self,
            &owner,
            value.starts_with(bootstrap_reader::unentered::OWNER_MAGIC),
        )?;
        Ok(Some(owner))
    }
    fn exact_owner(&self, token: OwnerToken) -> Result<LedgerOwner> {
        let owner = self
            .owner(token.owner)?
            .ok_or_else(|| conflict("owner absent"))?;
        if owner.binding.generation != token.generation {
            return Err(conflict("stale owner generation"));
        }
        Ok(owner)
    }
    fn update_resources(
        &mut self,
        owner: &LedgerOwner,
        previous: Option<OwnerSlot>,
        phase: PinPhase,
    ) -> Result<()> {
        for identity in &owner.binding.resources {
            let mut record = self.resource(*identity)?;
            record.update(
                self.view,
                owner.binding.descriptor.id,
                previous,
                OwnerSlot {
                    generation: owner.binding.generation,
                    phase,
                },
                &mut self.writes,
            )?;
        }
        Ok(())
    }
    fn acquire(&mut self, binding: &OwnerBinding, from: Option<OwnerToken>) -> Result<()> {
        binding.validate(self.root)?;
        if binding.descriptor.kind == OwnerKind::Reader {
            return reader_lifecycle::share(
                self,
                from.ok_or_else(|| conflict("Reader requires an exact Share source"))?,
                binding,
            );
        }
        if binding.descriptor.kind == OwnerKind::BootstrapReader {
            return Err(conflict("bootstrap reader requires exact typed admission"));
        }
        if binding.descriptor.kind == OwnerKind::Publisher {
            return Err(conflict("Publisher requires atomic migration entry"));
        }
        build_index::record(self, binding)?;
        reader_index::record(self, binding)?;
        let executor = checkpoint_admission::entry(self, binding)?;
        if let Some(source) = from {
            let source = self.exact_owner(source)?;
            if source.binding.descriptor.id == binding.descriptor.id
                || !matches!(source.phase, PinPhase::Held | PinPhase::Published)
                || source.binding.resources != binding.resources
                || source.binding.descriptor.subject != binding.descriptor.subject
                || source.binding.descriptor.subject_is_anchor
                    != binding.descriptor.subject_is_anchor
            {
                return Err(conflict("transfer source or subject/closure differs"));
            }
        }
        let previous = match self.owner(binding.descriptor.id)? {
            None if binding.generation.get() != 1 => {
                return Err(conflict("initial generation must be one"))
            }
            None => {
                for resource in &binding.resources {
                    if self
                        .resource(*resource)?
                        .owner(self.view, binding.descriptor.id)?
                        .is_some()
                    {
                        return Err(invalid(
                            "resource pin exists without its owner registration",
                        ));
                    }
                }
                self.writes.insert(
                    owner_registration_key(binding.descriptor.id),
                    owner_registration(binding),
                );
                None
            }
            Some(old) => {
                if old.binding.descriptor != binding.descriptor
                    || old.binding.resources != binding.resources
                {
                    return Err(conflict("owner identity or closure cannot be reused"));
                }
                if old.binding.generation == binding.generation {
                    if matches!(old.phase, PinPhase::Held | PinPhase::Published) {
                        if let Some(pending) = executor {
                            checkpoint_admission::enrollment::require_entered(self, &old)?;
                            checkpoint_admission::write_index(self, pending, &old)?;
                        }
                        return Ok(());
                    }
                    return Err(conflict("quiesced/released generation cannot resume"));
                }
                if old.phase != PinPhase::Released
                    || old.binding.generation.get().checked_add(1) != Some(binding.generation.get())
                {
                    return Err(conflict("owner generation does not follow release"));
                }
                Some(OwnerSlot {
                    generation: old.binding.generation,
                    phase: old.phase,
                })
            }
        };
        let owner = LedgerOwner {
            binding: binding.clone(),
            phase: PinPhase::Held,
        };
        if executor.is_some() {
            checkpoint_admission::enrollment::record_new(
                self,
                binding,
                checkpoint_admission::enrollment::Origin::Entered,
            )?;
        }
        self.update_resources(&owner, previous, PinPhase::Held)?;
        if let Some(pending) = executor {
            checkpoint_admission::write_index(self, pending, &owner)?;
        } else {
            self.writes
                .insert(owner_key(binding.descriptor.id), encode_owner(&owner));
        }
        Ok(())
    }
    fn transition(&mut self, token: OwnerToken, phase: PinPhase) -> Result<()> {
        let mut owner = self.exact_owner(token)?;
        if owner.binding.descriptor.kind == OwnerKind::Reader {
            if phase == PinPhase::Published {
                return reader_lifecycle::publish(self, &owner);
            }
            if self
                .view
                .get(ColumnFamily::Default, &owner_key(token.owner))?
                .is_some_and(|b| b.starts_with(reader_lifecycle::OWNER_MAGIC))
            {
                return Err(conflict("Reader requires its exact drained finish"));
            }
        }
        if owner.binding.descriptor.kind == OwnerKind::BootstrapReader {
            return if phase == PinPhase::Published {
                bootstrap_reader::admission::publish(self, &owner)
            } else {
                Err(conflict(
                    "bootstrap reader requires durable store read fencing",
                ))
            };
        }
        if owner.binding.descriptor.kind == OwnerKind::Publisher {
            return Err(conflict("Publisher requires exact drained finish"));
        }
        build_index::record(self, &owner.binding)?;
        reader_index::record(self, &owner.binding)?;
        let executor = if phase == PinPhase::Published {
            checkpoint_admission::entry(self, &owner.binding)?
        } else {
            checkpoint_admission::managed(self, &owner.binding)?.map(|g| g.pending)
        };
        if executor.is_some() && phase == PinPhase::Published {
            checkpoint_admission::enrollment::require_entered(self, &owner)?;
        }
        // Generic legacy generations can transfer their pins, but doing so
        // must never retroactively enroll them in the managed I/O protocol.
        let indexed = self
            .view
            .get(ColumnFamily::Default, &owner_key(token.owner))?
            .is_some_and(|b| b.starts_with(EXECUTOR_OWNER_MAGIC));
        let executor = executor.filter(|_| indexed);
        if owner.phase == phase {
            if let Some(pending) = executor {
                checkpoint_admission::write_index(self, pending, &owner)?;
            }
            return Ok(());
        }
        let allowed = match phase {
            PinPhase::Published => owner.phase == PinPhase::Held,
            PinPhase::Quiesced => matches!(owner.phase, PinPhase::Held | PinPhase::Published),
            PinPhase::Released => owner.phase == PinPhase::Quiesced,
            PinPhase::Held => false,
        };
        if !allowed {
            return Err(conflict("owner phase does not permit transition"));
        }
        self.update_resources(
            &owner,
            Some(OwnerSlot {
                generation: token.generation,
                phase: owner.phase,
            }),
            phase,
        )?;
        owner.phase = phase;
        if let Some(pending) = executor {
            checkpoint_admission::write_index(self, pending, &owner)?;
        } else {
            self.writes
                .insert(owner_key(token.owner), encode_owner(&owner));
        }
        Ok(())
    }
    fn finish(mut self) -> Result<LedgerPlan> {
        image_publisher_index::record_changes(&mut self)?;
        image_publisher_closure::mark_changes(&mut self)?;
        bootstrap_index::record_changes(&mut self)?;
        checkpoint_closure::mark_changes(&mut self)?;
        reader_closure::mark_changes(&mut self)?;
        image_reader_closure::mark_changes(&mut self)?;
        bootstrap_closure::mark_changes(&mut self)?;
        let changed = !self.writes.is_empty();
        if changed {
            // Trusted specialized planners also write the current header.
            // They must not bypass the same downgrade/orphan checks.
            if let Some(stored) = self.view.get(ColumnFamily::Default, &key(1, &[]))? {
                if !stored.starts_with(HEADER_MAGIC) {
                    check_upgrade(self.view, &stored)?;
                }
            }
            self.revision = self
                .revision
                .checked_add(1)
                .ok_or_else(|| invalid("ledger revision exhausted"))?;
            self.writes
                .insert(key(1, &[]), header(self.root, self.revision));
        }
        let bytes: usize = self
            .writes
            .iter()
            .map(|(k, v)| k.len() + v.len() + 16)
            .sum();
        if bytes > MAX_LEDGER_BATCH_BYTES {
            return Err(invalid("atomic ledger batch exceeds bound"));
        }
        let mut batch = WriteBatch::new();
        for (k, v) in self.writes {
            batch.put(ColumnFamily::Default, k, v);
        }
        Ok(LedgerPlan {
            batch,
            revision: self.revision,
            changed,
        })
    }
}

/// Plan only from one established catalog snapshot. A local snapshot by itself
/// is insufficient: the runtime must enforce the module's term/drain contract.
pub fn plan_retention(
    view: &dyn ReadView,
    root: &RootDescriptor,
    request: &LedgerRequest,
) -> Result<LedgerPlan> {
    crate::checkpoint::inspect_initial_checkpoint_base(view, root)?;
    let digest = root.digest();
    let stored = view.get(ColumnFamily::Default, &key(1, &[]))?;
    let Some(stored) = stored else {
        if *request != LedgerRequest::Initialize
            && !matches!(request, LedgerRequest::BeginCheckpoint(_))
        {
            return Err(invalid("ledger has not been initialized"));
        }
        let mut end = PREFIX.to_vec();
        *end.last_mut().unwrap() += 1;
        if view
            .iter(ColumnFamily::Default, PREFIX, &end)?
            .next()
            .transpose()?
            .is_some()
        {
            return Err(invalid("ledger header missing over nonempty state"));
        }
        if let LedgerRequest::BeginCheckpoint(intent) = request {
            let mut p = Planner {
                view,
                root: digest,
                revision: 0,
                rows_enabled: true,
                writes: BTreeMap::new(),
            };
            checkpoint_begin::begin(&mut p, root, intent)?;
            return p.finish();
        }
        let mut batch = WriteBatch::new();
        batch.put(ColumnFamily::Default, key(1, &[]), header(digest, 0));
        return Ok(LedgerPlan {
            batch,
            revision: 0,
            changed: true,
        });
    };
    let (revision, rows_enabled) = read_header(&stored, digest)?;
    let mut p = Planner {
        view,
        root: digest,
        revision,
        rows_enabled,
        writes: BTreeMap::new(),
    };
    // Older planners must not grant Publish/retirement without generation
    // provenance. Upgrade and all other writes share one transaction.
    if !stored.starts_with(HEADER_MAGIC) {
        check_upgrade(view, &stored)?;
        p.writes.insert(key(1, &[]), header(digest, revision));
    }
    match request {
        LedgerRequest::BeginBootstrapRead {
            pending,
            source,
            store,
        } => {
            bootstrap_reader::admission::begin(
                &mut p,
                root,
                *pending,
                kv9_common::StoreIdentity {
                    cluster_id: root.cluster_id,
                    root_digest: digest,
                    node_id: *source,
                    store_incarnation: *store,
                },
            )?;
        }
        LedgerRequest::RetireBootstrapRead { .. } => {
            return Err(conflict(
                "bootstrap retirement requires independently verified durable read fence",
            ));
        }
        LedgerRequest::FinishImageRead { successor, reader } => {
            image_reader::finish(&mut p, *successor, reader)?;
        }
        LedgerRequest::EnterMigrationPublisher {
            operation,
            publisher,
        } => {
            migration_publisher::enter(&mut p, root, *operation, publisher)?;
        }
        LedgerRequest::FinishMigrationPublisher {
            operation,
            publisher,
            issued,
            resolved,
        } => {
            migration_publisher::finish(&mut p, root, *operation, publisher, *issued, *resolved)?;
        }
        LedgerRequest::Initialize => {}
        LedgerRequest::FinishCheckpointRead { successor, reader } => {
            checkpoint_reader::finish(&mut p, *successor, reader)?
        }
        LedgerRequest::Register(resources) => {
            validate_resources(resources, digest)?;
            for identity in resources {
                p.register_resource(*identity)?;
            }
        }
        LedgerRequest::Acquire(binding) => p.acquire(binding, None)?,
        LedgerRequest::Publish(token) => p.transition(*token, PinPhase::Published)?,
        LedgerRequest::Share { from, to } => p.acquire(to, Some(*from))?,
        LedgerRequest::QuiesceAfterTransfer { from, to } => {
            let source = p.exact_owner(*from)?;
            let target = p.exact_owner(*to)?;
            if from.owner == to.owner
                || target.phase != PinPhase::Published
                || source.binding.resources != target.binding.resources
                || source.binding.descriptor.subject != target.binding.descriptor.subject
                || source.binding.descriptor.subject_is_anchor
                    != target.binding.descriptor.subject_is_anchor
            {
                return Err(conflict(
                    "quiescence lacks a published successor for the complete subject",
                ));
            }
            // Migration pins quiesce ONLY against a committed settlement of
            // the operation, read from the SAME applied view this plan runs
            // against — commitment is exactly local visibility and absence
            // refuses in the safe direction. Two settlements exist and they
            // are mutually exclusive per operation: destination-install
            // EVIDENCE (the transfer completed) or a committed ABORT (the
            // stranded destination was abandoned). Either way the settlement
            // subject and derived operation digest must match BOTH owners —
            // the pair that one committed migration operation produced. A
            // published destination owner alone is a description of intent.
            if matches!(
                source.binding.descriptor.kind,
                OwnerKind::Snapshot | OwnerKind::Migration
            ) || target.binding.descriptor.kind == OwnerKind::Migration
            {
                let settled = source.binding.descriptor.operation
                    == target.binding.descriptor.operation
                    && (crate::data_groups::evidence::evidence_matches_owner_pair(
                        p.view,
                        p.root,
                        source.binding.descriptor.operation,
                        source.binding.descriptor.subject,
                    )? || crate::data_groups::abort::abort_matches_owner_pair(
                        p.view,
                        p.root,
                        source.binding.descriptor.operation,
                        source.binding.descriptor.subject,
                    )?);
                if !settled {
                    return Err(conflict(
                        "migration pins cannot quiesce without committed install evidence",
                    ));
                }
            }
            p.transition(*from, PinPhase::Quiesced)?;
        }
        LedgerRequest::RecordCheckpointDrain {
            pending,
            executor,
            issued,
            resolved,
        } => checkpoint_admission::drain::record(&mut p, *pending, *executor, *issued, *resolved)?,
        LedgerRequest::FinishCheckpointExecution {
            pending,
            executor,
            issued,
            resolved,
        } => checkpoint_admission::drain::finish(&mut p, *pending, executor, *issued, *resolved)?,
        LedgerRequest::Release(token) => p.transition(*token, PinPhase::Released)?,
        LedgerRequest::RecordCheckpoint(intent) => {
            checkpoint_begin::reject_legacy_record(&p, intent)?;
            checkpoint_intent::record(&mut p, root, intent)?;
        }
        LedgerRequest::BeginCheckpoint(intent) => checkpoint_begin::begin(&mut p, root, intent)?,
        LedgerRequest::RecordCheckpointRefusal { .. } => {
            return Err(conflict(
                "checkpoint refusal requires independently verified history",
            ))
        }
        LedgerRequest::AdvanceDataStoreHistory {
            expected,
            revision,
            after,
        } => {
            data_store_history::advance(&mut p, root, *expected, *revision, *after)?;
        }
        LedgerRequest::AdvanceConfigurationIntervals(request) => {
            data_configuration_intervals::advance(&mut p, root, *request)?;
        }
        LedgerRequest::RecordConfigurationTimeline(_) => {
            return Err(conflict(
                "configuration timeline requires independently verified source",
            ));
        }
        LedgerRequest::RecordConfigurationOrigins(_) => {
            return Err(conflict(
                "configuration origins require independently verified source",
            ));
        }
        LedgerRequest::RecordDataMembership(_) => {
            return Err(conflict(
                "DATA membership requires independently verified source",
            ));
        }
        LedgerRequest::RecordDataReadFence { .. } => {
            return Err(conflict(
                "DATA read fence requires independently verified store",
            ))
        }
        LedgerRequest::RecordCheckpointSelection { .. } => {
            return Err(conflict(
                "checkpoint selection requires independently verified replica",
            ))
        }
        LedgerRequest::AdvanceCheckpointClosure { pending, expected } => {
            checkpoint_closure::advance(&mut p, root, *pending, *expected)?;
        }
        LedgerRequest::AdvanceImageReaderClosure { pending, expected } => {
            image_reader_closure::advance(&mut p, root, *pending, *expected)?;
        }
        LedgerRequest::AdvanceReaderClosure { pending, expected } => {
            reader_closure::advance(&mut p, root, *pending, *expected)?;
        }
        LedgerRequest::AdvanceMetadataReadGate { pending, expected } => {
            metadata_read_gate::advance(&mut p, root, *pending, *expected)?;
        }
        LedgerRequest::AdvanceBootstrapClosure { pending, expected } => {
            bootstrap_closure::advance(&mut p, root, *pending, *expected)?;
        }
        LedgerRequest::CloseMetadataImageReads(pending) => {
            metadata_image_gate::close(&mut p, root, *pending)?;
        }
        LedgerRequest::CloseMetadataImageWrites(pending) => {
            metadata_image_write_gate::close(&mut p, root, *pending)?
        }
        LedgerRequest::AdvanceImagePublisherClosure { pending, expected } => {
            image_publisher_closure::advance(&mut p, root, *pending, *expected)?
        }
        LedgerRequest::AdvanceImagePublisherIndex {
            expected,
            stage,
            after,
            next,
        } => image_publisher_index::advance(&mut p, *expected, *stage, *after, *next)?,
        LedgerRequest::CloseCheckpointAdmission(token) => {
            checkpoint_admission::close(&mut p, *token)?
        }
        LedgerRequest::CancelUnenteredCheckpoint(binding) => {
            checkpoint_admission::cancel(&mut p, binding)?
        }
        LedgerRequest::RetireUnpublishedCheckpoint { pending, executor } => {
            checkpoint_admission::retirement::retire(&mut p, *pending, *executor)?
        }
        LedgerRequest::AdvanceCheckpointSweep {
            pending,
            expected,
            after,
            next,
        } => {
            checkpoint_admission::sweep::advance(&mut p, root, *pending, *expected, *after, *next)?
        }
        LedgerRequest::AdvanceBuildIndex {
            expected,
            stage,
            after,
            next,
        } => build_index::advance(&mut p, *expected, *stage, *after, *next)?,
        LedgerRequest::AdvanceReaderIndex {
            expected,
            stage,
            after,
            next,
        } => reader_index::advance(&mut p, *expected, *stage, *after, *next)?,
        LedgerRequest::AdvanceImageReaderIndex {
            expected,
            stage,
            after,
            next,
        } => image_reader_index::advance(&mut p, *expected, *stage, *after, *next)?,
        LedgerRequest::AdvanceBootstrapIndex {
            expected,
            stage,
            after,
            next,
        } => bootstrap_index::advance(&mut p, *expected, *stage, *after, *next)?,
    }
    p.finish()
}

/// Decode and cross-check one owner for an already initialized ledger. This is
/// an observation of the supplied view, not proof of freshness or live-use drain.
pub fn retention_owner(
    view: &dyn ReadView,
    root: &RootDescriptor,
    id: OwnerId,
) -> Result<Option<LedgerOwner>> {
    crate::checkpoint::inspect_initial_checkpoint_base(view, root)?;
    let digest = root.digest();
    let header = view
        .get(ColumnFamily::Default, &key(1, &[]))?
        .ok_or_else(|| invalid("ledger header missing"))?;
    let (revision, rows_enabled) = read_header(&header, digest)?;
    Planner {
        view,
        root: digest,
        revision,
        rows_enabled,
        writes: BTreeMap::new(),
    }
    .owner(id)
}

/// Pre-registration compatibility check. A missing ledger can describe only
/// an absent owner; an orphan owner row still refuses instead of being hidden.
pub fn optional_retention_owner(
    view: &dyn ReadView,
    root: &RootDescriptor,
    id: OwnerId,
) -> Result<Option<LedgerOwner>> {
    crate::checkpoint::inspect_initial_checkpoint_base(view, root)?;
    if view.get(ColumnFamily::Default, &key(1, &[]))?.is_none()
        && view.get(ColumnFamily::Default, &owner_key(id))?.is_none()
        && view
            .get(ColumnFamily::Default, &owner_registration_key(id))?
            .is_none()
    {
        return Ok(None);
    }
    retention_owner(view, root, id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{encode_row_key, memcmp_uint, ColumnValue, RowValue};
    use crate::schema::{ColumnId, REGIONS_DESC, SCHEMA_VERSION, SCHEMA_VERSION_DESC};
    use crate::store::MetaStore;
    use kv9_common::retention::{PinCommand, RetentionRecord};
    use kv9_common::{
        AppliedPosition, BootstrapGeneration, ClusterId, NodeId, RootVoter, StoreIncarnation,
    };
    use kv9_engine::{Engine, MemEngine, ReplicatedEngine, WalEngine};
    use std::sync::Arc;

    pub(super) fn seed() -> (RootDescriptor, WriteBatch) {
        seed_with_voters(vec![RootVoter {
            node_id: NodeId(1),
            addr: "127.0.0.1:20160".parse().unwrap(),
            store_incarnation: StoreIncarnation::from_bytes([3; 16]),
        }])
    }
    pub(super) fn seed_with_voters(voters: Vec<RootVoter>) -> (RootDescriptor, WriteBatch) {
        let root = RootDescriptor::new(
            ClusterId::from_bytes([1; 16]),
            BootstrapGeneration::from_bytes([2; 16]),
            voters,
            b"retention-ledger-test-credential",
        )
        .unwrap();
        let store = MetaStore::new(Arc::new(MemEngine::new()));
        let mut txn = store.begin().unwrap();
        crate::root::initialize_root(&mut txn, &root).unwrap();
        crate::admission::initialize_cluster(&mut txn, root.cluster_id, 1).unwrap();
        let mut schema = RowValue::new();
        schema.set(ColumnId(1), ColumnValue::Uint(0));
        schema.set(ColumnId(2), ColumnValue::Uint(SCHEMA_VERSION as u64));
        txn.insert(&SCHEMA_VERSION_DESC, &[memcmp_uint(0)], schema)
            .unwrap();
        let mut batch = txn.into_batch();
        let mut region = RowValue::new();
        for (column, value) in [(1, 1), (2, 0), (5, 1), (6, 1), (7, 1)] {
            region.set(ColumnId(column), ColumnValue::Uint(value));
        }
        region.set(ColumnId(3), ColumnValue::Bytes(vec![]));
        region.set(ColumnId(4), ColumnValue::Bytes(vec![]));
        batch.put(
            ColumnFamily::Default,
            encode_row_key(REGIONS_DESC.id, &[memcmp_uint(1)]).unwrap(),
            region.encode(),
        );
        (root, batch)
    }
    pub(super) fn fixture() -> (MemEngine, RootDescriptor) {
        let (root, batch) = seed();
        let engine = MemEngine::new();
        engine.write(batch).unwrap();
        (engine, root)
    }
    #[test]
    fn optional_owner_refuses_orphan_state_or_registration_without_header() {
        for registration in [false, true] {
            let (engine, root) = fixture();
            let id = OwnerId::new([21; 16]).unwrap();
            assert!(
                optional_retention_owner(engine.snapshot().unwrap().as_ref(), &root, id)
                    .unwrap()
                    .is_none()
            );
            let mut batch = WriteBatch::new();
            batch.put(
                ColumnFamily::Default,
                if registration {
                    owner_registration_key(id)
                } else {
                    owner_key(id)
                },
                vec![1],
            );
            engine.write(batch).unwrap();
            assert!(
                optional_retention_owner(engine.snapshot().unwrap().as_ref(), &root, id).is_err()
            );
        }
    }
    pub(super) fn resources(root: &RootDescriptor) -> Vec<ResourceIdentity> {
        (1..=2)
            .map(|n| {
                ResourceIdentity::new(root.digest(), ResourceKind::Sst, [n; 16], [n + 8; 32])
                    .unwrap()
            })
            .collect()
    }
    pub(super) fn binding(root: &RootDescriptor, n: u8) -> OwnerBinding {
        OwnerBinding {
            descriptor: OwnerDescriptor {
                root: root.digest(),
                id: OwnerId::new([n; 16]).unwrap(),
                // Pending has a real settlement seam (the checkpoint worker);
                // Snapshot/Migration transfers are separately fenced below.
                kind: OwnerKind::Pending,
                region: 1,
                conf_ver: 1,
                version: 1,
                operation: [n; 32],
                subject: [99; 32],
                subject_is_anchor: true,
                local: None,
            },
            generation: NonZeroU64::new(1).unwrap(),
            resources: resources(root),
        }
    }
    pub(super) fn plan(
        engine: &impl Engine,
        root: &RootDescriptor,
        request: &LedgerRequest,
    ) -> Result<LedgerPlan> {
        plan_retention(engine.snapshot()?.as_ref(), root, request)
    }
    pub(super) fn apply(engine: &MemEngine, root: &RootDescriptor, request: LedgerRequest) -> u64 {
        let plan = plan(engine, root, &request).unwrap();
        let revision = plan.revision();
        engine.write(plan.into_batch()).unwrap();
        revision
    }
    pub(super) fn owner(engine: &impl Engine, root: &RootDescriptor, id: OwnerId) -> LedgerOwner {
        retention_owner(engine.snapshot().unwrap().as_ref(), root, id)
            .unwrap()
            .unwrap()
    }
    pub(super) fn all(engine: &impl Engine) -> Vec<(Vec<u8>, Vec<u8>)> {
        let mut end = PREFIX.to_vec();
        *end.last_mut().unwrap() += 1;
        engine
            .snapshot()
            .unwrap()
            .iter(ColumnFamily::Default, PREFIX, &end)
            .unwrap()
            .collect::<Result<_>>()
            .unwrap()
    }
    pub(super) fn initialized(engine: &MemEngine, root: &RootDescriptor) {
        apply(engine, root, LedgerRequest::Initialize);
        apply(engine, root, LedgerRequest::Register(resources(root)));
    }

    #[test]
    fn request_codec_binds_all_operations_to_root_and_canonical_payload() {
        let (root, _) = seed();
        let a = binding(&root, 1);
        let b = binding(&root, 2);
        let requests = [
            LedgerRequest::Initialize,
            LedgerRequest::Register(a.resources.clone()),
            LedgerRequest::Acquire(a.clone()),
            LedgerRequest::Publish(a.token()),
            LedgerRequest::Share {
                from: a.token(),
                to: b.clone(),
            },
            LedgerRequest::QuiesceAfterTransfer {
                from: a.token(),
                to: b.token(),
            },
            LedgerRequest::Release(a.token()),
        ];
        for request in requests {
            let bytes = encode_request(root.digest(), &request).unwrap();
            assert_eq!(decode_request(&bytes, root.digest()).unwrap(), request);
            assert!(decode_request(&bytes, RootDigest::from_bytes([9; 32])).is_err());
            for cut in 0..bytes.len() {
                assert!(decode_request(&bytes[..cut], root.digest()).is_err());
            }
            let mut unknown = bytes[..bytes.len() - 32].to_vec();
            unknown[40] = 255;
            assert!(decode_request(&seal(unknown), root.digest()).is_err());
            let mut corrupt = bytes.clone();
            corrupt[41] ^= 1;
            assert!(decode_request(&corrupt, root.digest()).is_err());
            let mut trailing = bytes[..bytes.len() - 32].to_vec();
            trailing.push(0);
            assert!(decode_request(&seal(trailing), root.digest()).is_err());
        }
        // Even a correctly rehashed nested owner cannot supply a later phase.
        let mut phase = REQUEST_MAGIC.to_vec();
        phase.extend_from_slice(root.digest().as_bytes());
        phase.push(2);
        phase.extend_from_slice(&encode_owner(&LedgerOwner {
            binding: a,
            phase: PinPhase::Published,
        }));
        assert!(decode_request(&seal(phase), root.digest()).is_err());
        let mut count =
            encode_request(root.digest(), &LedgerRequest::Register(b.resources)).unwrap();
        count.truncate(count.len() - 32);
        count[41..45].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(decode_request(&seal(count), root.digest()).is_err());
        assert!(decode_request(&vec![0; MAX_LEDGER_REQUEST_BYTES + 1], root.digest()).is_err());
    }

    #[test]
    fn complete_transfer_keeps_source_until_published_destination_and_fences_generations() {
        let (engine, root) = fixture();
        initialized(&engine, &root);
        let a = binding(&root, 1);
        let b = binding(&root, 2);
        apply(&engine, &root, LedgerRequest::Acquire(a.clone()));
        apply(&engine, &root, LedgerRequest::Publish(a.token()));
        assert!(plan(&engine, &root, &LedgerRequest::Release(a.token())).is_err());
        apply(
            &engine,
            &root,
            LedgerRequest::Share {
                from: a.token(),
                to: b.clone(),
            },
        );
        assert_eq!(
            owner(&engine, &root, a.descriptor.id).phase,
            PinPhase::Published
        );
        assert_eq!(owner(&engine, &root, b.descriptor.id).phase, PinPhase::Held);
        assert!(plan(
            &engine,
            &root,
            &LedgerRequest::QuiesceAfterTransfer {
                from: a.token(),
                to: b.token()
            }
        )
        .is_err());
        apply(&engine, &root, LedgerRequest::Publish(b.token()));
        apply(
            &engine,
            &root,
            LedgerRequest::QuiesceAfterTransfer {
                from: a.token(),
                to: b.token(),
            },
        );
        apply(&engine, &root, LedgerRequest::Release(a.token()));
        assert_eq!(
            owner(&engine, &root, b.descriptor.id).phase,
            PinPhase::Published
        );
        assert!(!plan(&engine, &root, &LedgerRequest::Release(a.token()))
            .unwrap()
            .changed());
        assert!(plan(&engine, &root, &LedgerRequest::Acquire(a.clone())).is_err());
        let mut next = a.clone();
        next.generation = NonZeroU64::new(2).unwrap();
        apply(&engine, &root, LedgerRequest::Acquire(next.clone()));
        let before = all(&engine);
        assert!(plan(&engine, &root, &LedgerRequest::Release(a.token())).is_err());
        assert_eq!(all(&engine), before);
        assert_eq!(
            owner(&engine, &root, a.descriptor.id).binding.generation,
            next.generation
        );
    }

    #[test]
    fn exact_retry_does_not_rebind_identity_or_advance_ledger_revision() {
        let (engine, root) = fixture();
        initialized(&engine, &root);
        let a = binding(&root, 1);
        let revision = apply(&engine, &root, LedgerRequest::Acquire(a.clone()));
        let again = plan(&engine, &root, &LedgerRequest::Acquire(a.clone())).unwrap();
        assert!(!again.changed());
        assert_eq!(again.revision(), revision);
        for field in 0..5 {
            let mut wrong = a.clone();
            match field {
                0 => wrong.descriptor.operation[0] ^= 1,
                1 => wrong.descriptor.subject[0] ^= 1,
                2 => wrong.descriptor.version += 1,
                3 => {
                    wrong.descriptor.local = Some(LocalLifetime {
                        node: 1,
                        store: [4; 16],
                        process: [5; 16],
                    })
                }
                _ => {
                    wrong.resources.pop();
                }
            }
            assert!(plan(&engine, &root, &LedgerRequest::Acquire(wrong)).is_err());
        }
        apply(&engine, &root, LedgerRequest::Publish(a.token()));
        assert!(!plan(&engine, &root, &LedgerRequest::Acquire(a))
            .unwrap()
            .changed());
    }

    #[test]
    fn failed_late_resource_validation_exposes_no_partial_batch() {
        let (engine, root) = fixture();
        apply(&engine, &root, LedgerRequest::Initialize);
        let rs = resources(&root);
        apply(&engine, &root, LedgerRequest::Register(vec![rs[1]]));
        let wrong = ResourceIdentity::new(
            root.digest(),
            ResourceKind::Sst,
            *rs[1].instance(),
            [17; 32],
        )
        .unwrap();
        let before = all(&engine);
        assert!(plan(&engine, &root, &LedgerRequest::Register(vec![rs[0], wrong])).is_err());
        assert_eq!(all(&engine), before);
        assert!(engine
            .get(ColumnFamily::Default, &resource_key(&rs[0]))
            .unwrap()
            .is_none());
        apply(&engine, &root, LedgerRequest::Register(vec![rs[0]]));
        let mut retired = RetentionRecord::new(rs[1]);
        retired
            .apply(PinCommand::Retire {
                expected_revision: 0,
            })
            .unwrap();
        let mut corrupt = WriteBatch::new();
        corrupt.put(
            ColumnFamily::Default,
            resource_key(&rs[1]),
            retired.encode(),
        );
        engine.write(corrupt).unwrap();
        let before = all(&engine);
        assert!(plan(&engine, &root, &LedgerRequest::Acquire(binding(&root, 1))).is_err());
        assert_eq!(all(&engine), before);
        assert!(retention_owner(
            engine.snapshot().unwrap().as_ref(),
            &root,
            binding(&root, 1).descriptor.id
        )
        .unwrap()
        .is_none());
    }

    #[test]
    fn missing_registered_state_is_unavailable_and_never_reconstructed_as_empty() {
        for missing in 0..5 {
            let (engine, root) = fixture();
            initialized(&engine, &root);
            let a = binding(&root, 1);
            apply(&engine, &root, LedgerRequest::Acquire(a.clone()));
            let k = match missing {
                0 => key(1, &[]),
                1 => owner_key(a.descriptor.id),
                2 => owner_registration_key(a.descriptor.id),
                3 => resource_key(&a.resources[0]),
                _ => resource_registration_key(&a.resources[0]),
            };
            let mut damage = WriteBatch::new();
            damage.delete(ColumnFamily::Default, k);
            engine.write(damage).unwrap();
            let before = all(&engine);
            assert!(
                retention_owner(engine.snapshot().unwrap().as_ref(), &root, a.descriptor.id)
                    .is_err()
            );
            assert!(plan(&engine, &root, &LedgerRequest::Acquire(a.clone())).is_err());
            if missing == 0 {
                assert!(plan(&engine, &root, &LedgerRequest::Initialize).is_err());
            }
            if missing >= 3 {
                assert!(plan(&engine, &root, &LedgerRequest::Register(a.resources)).is_err());
            }
            assert_eq!(all(&engine), before);
        }
    }

    #[test]
    fn transfer_requires_the_whole_subject_and_cannot_cycle_quiesced_owners() {
        let (engine, root) = fixture();
        initialized(&engine, &root);
        let a = binding(&root, 1);
        let mut b = binding(&root, 2);
        apply(&engine, &root, LedgerRequest::Acquire(a.clone()));
        apply(&engine, &root, LedgerRequest::Publish(a.token()));
        b.resources.pop();
        apply(&engine, &root, LedgerRequest::Acquire(b.clone()));
        apply(&engine, &root, LedgerRequest::Publish(b.token()));
        assert!(plan(
            &engine,
            &root,
            &LedgerRequest::QuiesceAfterTransfer {
                from: a.token(),
                to: b.token()
            }
        )
        .is_err());
        let mut c = binding(&root, 3);
        c.descriptor.subject[0] ^= 1;
        assert!(plan(
            &engine,
            &root,
            &LedgerRequest::Share {
                from: a.token(),
                to: c
            }
        )
        .is_err());
        let c = binding(&root, 3);
        apply(
            &engine,
            &root,
            LedgerRequest::Share {
                from: a.token(),
                to: c.clone(),
            },
        );
        apply(&engine, &root, LedgerRequest::Publish(c.token()));
        apply(
            &engine,
            &root,
            LedgerRequest::QuiesceAfterTransfer {
                from: a.token(),
                to: c.token(),
            },
        );
        assert!(plan(
            &engine,
            &root,
            &LedgerRequest::QuiesceAfterTransfer {
                from: c.token(),
                to: a.token()
            }
        )
        .is_err());
        assert!(plan(
            &engine,
            &root,
            &LedgerRequest::Share {
                from: a.token(),
                to: binding(&root, 4)
            }
        )
        .is_err());
    }

    #[test]
    fn migration_pins_cannot_quiesce_or_release_without_install_evidence() {
        let (engine, root) = fixture();
        initialized(&engine, &root);
        for (n, source_kind, target_kind) in [
            (1, OwnerKind::Snapshot, OwnerKind::Snapshot),
            (5, OwnerKind::Migration, OwnerKind::Migration),
            (9, OwnerKind::Pending, OwnerKind::Migration),
        ] {
            let mut a = binding(&root, n);
            a.descriptor.kind = source_kind;
            let mut b = binding(&root, n + 1);
            b.descriptor.kind = target_kind;
            apply(&engine, &root, LedgerRequest::Acquire(a.clone()));
            apply(&engine, &root, LedgerRequest::Publish(a.token()));
            apply(
                &engine,
                &root,
                LedgerRequest::Share {
                    from: a.token(),
                    to: b.clone(),
                },
            );
            apply(&engine, &root, LedgerRequest::Publish(b.token()));
            // A published migration successor is a description of intent, not
            // durable install evidence: quiesce and release must both refuse.
            assert!(
                plan(
                    &engine,
                    &root,
                    &LedgerRequest::QuiesceAfterTransfer {
                        from: a.token(),
                        to: b.token()
                    }
                )
                .is_err(),
                "{source_kind:?}->{target_kind:?} quiesced without settlement"
            );
            assert!(plan(&engine, &root, &LedgerRequest::Release(a.token())).is_err());
            assert_eq!(
                owner(&engine, &root, a.descriptor.id).phase,
                PinPhase::Published
            );
            assert_eq!(
                owner(&engine, &root, b.descriptor.id).phase,
                PinPhase::Published
            );
        }
    }

    /// One committed evidence row — read from the very view the plan runs
    /// against — is what turns the migration quiesce refusal into an accepted
    /// settlement. Mismatched subject or operation keeps refusing, the
    /// released source follows only from the quiesced phase, and the
    /// destination pin (the live replica's protection) never drops.
    #[test]
    fn migration_pins_quiesce_and_release_only_with_matching_committed_evidence() {
        use crate::schema::TASKS_DESC;
        let evidence_payload =
            |root: &RootDescriptor, operation: [u8; 16], task: u64, subject: [u8; 32]| {
                let mut b = b"KV9EVD01".to_vec();
                b.extend_from_slice(root.digest().as_bytes());
                b.extend_from_slice(&operation);
                b.extend_from_slice(&task.to_be_bytes());
                b.extend_from_slice(&700u64.to_be_bytes()); // migration task
                b.extend_from_slice(&800u64.to_be_bytes()); // region
                b.extend_from_slice(&4u64.to_be_bytes()); // destination node
                b.extend_from_slice(&[40; 16]); // destination incarnation
                b.extend_from_slice(&[41; 16]); // adopted generation
                b.extend_from_slice(&[42; 32]); // image digest
                b.extend_from_slice(&subject);
                b.extend_from_slice(&3u64.to_be_bytes()); // cut term
                b.extend_from_slice(&12u64.to_be_bytes()); // cut index
                b
            };
        let write_evidence = |engine: &MemEngine, payload: Vec<u8>, task: u64| {
            let mut row = RowValue::new();
            row.set(ColumnId(1), ColumnValue::Uint(task));
            row.set(ColumnId(2), ColumnValue::Uint(104));
            row.set(ColumnId(3), ColumnValue::Bytes(payload));
            row.set(ColumnId(4), ColumnValue::Uint(0));
            row.set(ColumnId(5), ColumnValue::Uint(0));
            let mut batch = WriteBatch::new();
            batch.put(
                ColumnFamily::Default,
                encode_row_key(TASKS_DESC.id, &[memcmp_uint(task)]).unwrap(),
                row.encode(),
            );
            engine.write(batch).unwrap();
        };
        let (engine, root) = fixture();
        initialized(&engine, &root);
        let operation = [7u8; 16];
        let digest =
            crate::data_groups::evidence::migration_operation_digest(root.digest(), operation);
        let subject = [42u8; 32];
        let prepared = |kind: OwnerKind, n: u8| {
            let mut owner = binding(&root, n);
            owner.descriptor.kind = kind;
            owner.descriptor.operation = digest;
            owner.descriptor.subject = subject;
            owner.descriptor.subject_is_anchor = false;
            owner
        };
        let a = prepared(OwnerKind::Snapshot, 1);
        let b = prepared(OwnerKind::Migration, 2);
        apply(&engine, &root, LedgerRequest::Acquire(a.clone()));
        apply(&engine, &root, LedgerRequest::Publish(a.token()));
        apply(
            &engine,
            &root,
            LedgerRequest::Share {
                from: a.token(),
                to: b.clone(),
            },
        );
        apply(&engine, &root, LedgerRequest::Publish(b.token()));
        let quiesce = LedgerRequest::QuiesceAfterTransfer {
            from: a.token(),
            to: b.token(),
        };
        assert!(
            plan(&engine, &root, &quiesce).is_err(),
            "quiesced without any committed evidence"
        );
        // Evidence naming a different image subject is not settlement.
        write_evidence(
            &engine,
            evidence_payload(&root, operation, 300, [43; 32]),
            300,
        );
        assert!(
            plan(&engine, &root, &quiesce).is_err(),
            "quiesced on a different image's evidence"
        );
        // Evidence for a different operation is not settlement either.
        write_evidence(&engine, evidence_payload(&root, [8; 16], 300, subject), 300);
        assert!(
            plan(&engine, &root, &quiesce).is_err(),
            "quiesced on a different operation's evidence"
        );
        // The matching committed row settles the transfer: source quiesces,
        // then releases; the destination pin stays published throughout.
        write_evidence(
            &engine,
            evidence_payload(&root, operation, 300, subject),
            300,
        );
        apply(&engine, &root, quiesce);
        assert_eq!(
            owner(&engine, &root, a.descriptor.id).phase,
            PinPhase::Quiesced
        );
        assert_eq!(
            owner(&engine, &root, b.descriptor.id).phase,
            PinPhase::Published
        );
        apply(&engine, &root, LedgerRequest::Release(a.token()));
        assert_eq!(
            owner(&engine, &root, a.descriptor.id).phase,
            PinPhase::Released
        );
        assert_eq!(
            owner(&engine, &root, b.descriptor.id).phase,
            PinPhase::Published
        );
        assert!(
            plan(&engine, &root, &LedgerRequest::Release(b.token())).is_err(),
            "the destination pin must never drop while the replica lives"
        );
        assert!(
            plan(
                &engine,
                &root,
                &LedgerRequest::QuiesceAfterTransfer {
                    from: b.token(),
                    to: a.token()
                }
            )
            .is_err(),
            "the destination cannot quiesce toward a released owner"
        );
    }

    /// A committed ABORT row settles the transfer exactly like evidence —
    /// the stranded-destination recovery. Same view-level discipline: the
    /// subject and derived operation digest must match both owners, and
    /// mismatches keep refusing. The destination pin still never drops.
    #[test]
    fn migration_pins_quiesce_and_release_with_a_matching_committed_abort() {
        use crate::schema::TASKS_DESC;
        let abort_payload =
            |root: &RootDescriptor, operation: [u8; 16], task: u64, subject: [u8; 32]| {
                let mut b = b"KV9ABT01".to_vec();
                b.extend_from_slice(root.digest().as_bytes());
                b.extend_from_slice(&operation);
                b.extend_from_slice(&task.to_be_bytes());
                b.extend_from_slice(&700u64.to_be_bytes()); // migration task
                b.extend_from_slice(&800u64.to_be_bytes()); // region
                b.extend_from_slice(&4u64.to_be_bytes()); // destination node
                b.extend_from_slice(&[40; 16]); // destination incarnation
                b.extend_from_slice(&subject);
                b
            };
        let write_abort = |engine: &MemEngine, payload: Vec<u8>, task: u64| {
            let mut row = RowValue::new();
            row.set(ColumnId(1), ColumnValue::Uint(task));
            row.set(ColumnId(2), ColumnValue::Uint(108));
            row.set(ColumnId(3), ColumnValue::Bytes(payload));
            row.set(ColumnId(4), ColumnValue::Uint(0));
            row.set(ColumnId(5), ColumnValue::Uint(0));
            let mut batch = WriteBatch::new();
            batch.put(
                ColumnFamily::Default,
                encode_row_key(TASKS_DESC.id, &[memcmp_uint(task)]).unwrap(),
                row.encode(),
            );
            engine.write(batch).unwrap();
        };
        let (engine, root) = fixture();
        initialized(&engine, &root);
        let operation = [7u8; 16];
        let digest =
            crate::data_groups::evidence::migration_operation_digest(root.digest(), operation);
        let subject = [42u8; 32];
        let prepared = |kind: OwnerKind, n: u8| {
            let mut owner = binding(&root, n);
            owner.descriptor.kind = kind;
            owner.descriptor.operation = digest;
            owner.descriptor.subject = subject;
            owner.descriptor.subject_is_anchor = false;
            owner
        };
        let a = prepared(OwnerKind::Snapshot, 1);
        let b = prepared(OwnerKind::Migration, 2);
        apply(&engine, &root, LedgerRequest::Acquire(a.clone()));
        apply(&engine, &root, LedgerRequest::Publish(a.token()));
        apply(
            &engine,
            &root,
            LedgerRequest::Share {
                from: a.token(),
                to: b.clone(),
            },
        );
        apply(&engine, &root, LedgerRequest::Publish(b.token()));
        let quiesce = LedgerRequest::QuiesceAfterTransfer {
            from: a.token(),
            to: b.token(),
        };
        assert!(
            plan(&engine, &root, &quiesce).is_err(),
            "quiesced without any committed settlement"
        );
        write_abort(&engine, abort_payload(&root, operation, 300, [43; 32]), 300);
        assert!(
            plan(&engine, &root, &quiesce).is_err(),
            "quiesced on a different image's abort"
        );
        write_abort(&engine, abort_payload(&root, [8; 16], 300, subject), 300);
        assert!(
            plan(&engine, &root, &quiesce).is_err(),
            "quiesced on a different operation's abort"
        );
        write_abort(&engine, abort_payload(&root, operation, 300, subject), 300);
        apply(&engine, &root, quiesce);
        assert_eq!(
            owner(&engine, &root, a.descriptor.id).phase,
            PinPhase::Quiesced
        );
        apply(&engine, &root, LedgerRequest::Release(a.token()));
        assert_eq!(
            owner(&engine, &root, a.descriptor.id).phase,
            PinPhase::Released
        );
        assert_eq!(
            owner(&engine, &root, b.descriptor.id).phase,
            PinPhase::Published
        );
        assert!(
            plan(&engine, &root, &LedgerRequest::Release(b.token())).is_err(),
            "the destination pin must never drop"
        );
    }

    #[test]
    fn owner_codec_rejects_truncation_corruption_and_checksum_valid_bad_counts() {
        let (_, root) = fixture();
        let mut a = binding(&root, 1);
        a.descriptor.kind = OwnerKind::Reader;
        a.descriptor.local = Some(LocalLifetime {
            node: 1,
            store: [4; 16],
            process: [5; 16],
        });
        let record = LedgerOwner {
            binding: a.clone(),
            phase: PinPhase::Held,
        };
        let b = encode_owner(&record);
        assert_eq!(
            decode_owner(&b, root.digest(), a.descriptor.id).unwrap(),
            record
        );
        for end in 0..b.len() {
            assert!(decode_owner(&b[..end], root.digest(), a.descriptor.id).is_err());
        }
        for pos in 0..b.len() {
            let mut damaged = b.clone();
            damaged[pos] ^= 1;
            assert!(decode_owner(&damaged, root.digest(), a.descriptor.id).is_err());
        }
        let mut bad = b[..b.len() - 32].to_vec();
        let count = bad.len() - a.resources.len() * 49 - 4;
        bad[count..count + 4].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(decode_owner(&seal(bad), root.digest(), a.descriptor.id).is_err());
        a.resources = (1..=MAX_OWNER_RESOURCES)
            .map(|n| {
                ResourceIdentity::new(root.digest(), ResourceKind::Sst, [n as u8; 16], [8; 32])
                    .unwrap()
            })
            .collect();
        let max = LedgerOwner {
            binding: a.clone(),
            phase: PinPhase::Held,
        };
        let b = encode_owner(&max);
        assert!(b.len() <= MAX_OWNER_BYTES);
        assert_eq!(
            decode_owner(&b, root.digest(), a.descriptor.id).unwrap(),
            max
        );
        let mut unknown = header(root.digest(), 0);
        unknown.truncate(unknown.len() - 32);
        unknown[40] = 2;
        assert!(read_header(&seal(unknown), root.digest()).is_err());
    }

    #[test]
    fn planned_effect_has_no_visibility_until_one_positioned_atomic_write() {
        let (engine, root) = fixture();
        initialized(&engine, &root);
        let a = binding(&root, 1);
        let pending = plan(&engine, &root, &LedgerRequest::Acquire(a.clone())).unwrap();
        assert!(
            retention_owner(engine.snapshot().unwrap().as_ref(), &root, a.descriptor.id)
                .unwrap()
                .is_none()
        );
        engine
            .write_applied(pending.into_batch(), AppliedPosition { term: 1, index: 5 })
            .unwrap();
        assert_eq!(owner(&engine, &root, a.descriptor.id).phase, PinPhase::Held);
    }

    #[test]
    fn actual_wal_tail_cuts_restore_the_whole_acquisition_or_no_acquisition() {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kv9-ledger-wal-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("source.wal");
        let (root, seed) = seed();
        let (engine, _) = WalEngine::open(&path).unwrap();
        engine
            .write_applied(seed, AppliedPosition { term: 1, index: 1 })
            .unwrap();
        for (index, request) in [
            (2, LedgerRequest::Initialize),
            (3, LedgerRequest::Register(resources(&root))),
        ] {
            let p = plan(&engine, &root, &request).unwrap();
            engine
                .write_applied(p.into_batch(), AppliedPosition { term: 1, index })
                .unwrap();
        }
        let prefix = std::fs::read(&path).unwrap();
        let a = binding(&root, 1);
        let p = plan(&engine, &root, &LedgerRequest::Acquire(a.clone())).unwrap();
        engine
            .write_applied(p.into_batch(), AppliedPosition { term: 1, index: 4 })
            .unwrap();
        drop(engine);
        let complete = std::fs::read(&path).unwrap();
        assert!(complete.len() > prefix.len());
        let appended = complete.len() - prefix.len();
        let mut cuts = vec![
            0,
            1,
            3,
            4,
            8,
            16,
            appended / 2,
            appended - 4,
            appended - 1,
            appended,
        ];
        cuts.sort_unstable();
        cuts.dedup();
        for cut in cuts {
            let path = dir.join(format!("cut-{cut}.wal"));
            std::fs::write(&path, &complete[..prefix.len() + cut]).unwrap();
            let (recovered, _) = WalEngine::open(&path).unwrap();
            let observed = retention_owner(
                recovered.snapshot().unwrap().as_ref(),
                &root,
                a.descriptor.id,
            )
            .unwrap();
            assert_eq!(observed.is_some(), cut == appended, "cut {cut}");
            if let Some(observed) = observed {
                assert_eq!(observed.binding, a);
                assert_eq!(observed.phase, PinPhase::Held);
            }
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
