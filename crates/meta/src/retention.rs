//! Bounded retention-ledger planning over one established metadata view.
//!
//! Plans are not durable receipts. The caller must hold the metadata planner
//! lock, establish/drain the current Raft term before supplying this view, and
//! commit the entire returned batch in that same term before acknowledging it.
//! No caller may execute individual resource writes from a plan.
//!
//! Version 1 tracks explicitly registered owners only. It does not certify a
//! complete backfill of existing references and provides no deletion operation.
//! Final release after actual use drains needs a separate settlement capability;
//! this first interface permits quiescence only under a published successor.
use std::collections::BTreeMap;
use std::num::NonZeroU64;

use kv9_common::retention::{
    OwnerId, OwnerToken, PinCommand, PinPhase, ResourceIdentity, ResourceKind, RetentionRecord,
};
use kv9_common::{Error, Result, RootDescriptor, RootDigest};
use kv9_engine::{ColumnFamily, ReadView, WriteBatch};

pub const MAX_OWNER_RESOURCES: usize = 64;
pub const MAX_LEDGER_BATCH_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_LEDGER_REQUEST_BYTES: usize = 16 * 1024;
const MAX_OWNER_BYTES: usize = 8 * 1024;
const PREFIX: &[u8] = b"\0kv9\0retention_v1\0";
const HEADER_MAGIC: &[u8; 8] = b"KV9LED01";
const OWNER_MAGIC: &[u8; 8] = b"KV9OWN01";
const REQUEST_MAGIC: &[u8; 8] = b"KV9RTX01";
const TRACKING_ONLY: u8 = 1;

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
            || (d.kind == OwnerKind::Reader && d.local.is_none())
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
        LedgerRequest::Release(t) => {
            b.push(6);
            token(&mut b, *t);
        }
    }
    if root.as_bytes() == &[0; 32] || b.len() + 32 > MAX_LEDGER_REQUEST_BYTES {
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
fn read_header(b: &[u8], root: RootDigest) -> Result<u64> {
    let b = checked(b, HEADER_MAGIC, 81)?;
    if b.len() != 41 || &b[..32] != root.as_bytes() || b[32] != TRACKING_ONLY {
        return Err(invalid("unsupported ledger root or coverage mode"));
    }
    Ok(u64::from_be_bytes(b[33..41].try_into().unwrap()))
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
    let mut r = Reader(checked(bytes, OWNER_MAGIC, MAX_OWNER_BYTES)?);
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
    if encode_owner(&owner) != bytes {
        return Err(invalid("noncanonical owner record"));
    }
    Ok(owner)
}

struct Planner<'a> {
    view: &'a dyn ReadView,
    root: RootDigest,
    revision: u64,
    writes: BTreeMap<Vec<u8>, Vec<u8>>,
}
impl Planner<'_> {
    fn resource(&self, identity: ResourceIdentity) -> Result<RetentionRecord> {
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
        RetentionRecord::decode(&value, identity).map_err(pin_error)
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
        if registered != owner_registration(&owner.binding) {
            return Err(invalid("owner immutable registration disagrees"));
        }
        // The owner row and every forward pin are one durable transaction.
        // Missing or conflicting cross-links cannot be interpreted as no uses.
        for resource in &owner.binding.resources {
            let record = self.resource(*resource)?;
            let slot = record
                .owner(id)
                .ok_or_else(|| invalid("owner resource link missing"))?;
            if slot.generation != owner.binding.generation || slot.phase != owner.phase {
                return Err(invalid("owner resource link disagrees"));
            }
        }
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
    fn update_resources(&mut self, owner: &LedgerOwner, command: PinCommand) -> Result<()> {
        for identity in &owner.binding.resources {
            let mut record = self.resource(*identity)?;
            record.apply(command).map_err(pin_error)?;
            self.writes.insert(resource_key(identity), record.encode());
        }
        Ok(())
    }
    fn acquire(&mut self, binding: &OwnerBinding, from: Option<OwnerToken>) -> Result<()> {
        binding.validate(self.root)?;
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
        match self.owner(binding.descriptor.id)? {
            None if binding.generation.get() != 1 => {
                return Err(conflict("initial generation must be one"))
            }
            None => {
                for resource in &binding.resources {
                    if self
                        .resource(*resource)?
                        .owner(binding.descriptor.id)
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
            }
            Some(old) => {
                if old.binding.descriptor != binding.descriptor
                    || old.binding.resources != binding.resources
                {
                    return Err(conflict("owner identity or closure cannot be reused"));
                }
                if old.binding.generation == binding.generation {
                    if matches!(old.phase, PinPhase::Held | PinPhase::Published) {
                        return Ok(());
                    }
                    return Err(conflict("quiesced/released generation cannot resume"));
                }
                if old.phase != PinPhase::Released
                    || old.binding.generation.get().checked_add(1) != Some(binding.generation.get())
                {
                    return Err(conflict("owner generation does not follow release"));
                }
            }
        }
        let owner = LedgerOwner {
            binding: binding.clone(),
            phase: PinPhase::Held,
        };
        let command = match from {
            Some(from) => PinCommand::Share {
                from,
                to: binding.token(),
            },
            None => PinCommand::Acquire(binding.token()),
        };
        self.update_resources(&owner, command)?;
        self.writes
            .insert(owner_key(binding.descriptor.id), encode_owner(&owner));
        Ok(())
    }
    fn transition(&mut self, token: OwnerToken, phase: PinPhase) -> Result<()> {
        let mut owner = self.exact_owner(token)?;
        if owner.phase == phase {
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
        let command = match phase {
            PinPhase::Published => PinCommand::Publish(token),
            PinPhase::Quiesced => PinCommand::Quiesce(token),
            PinPhase::Released => PinCommand::Release(token),
            PinPhase::Held => unreachable!(),
        };
        self.update_resources(&owner, command)?;
        owner.phase = phase;
        self.writes
            .insert(owner_key(token.owner), encode_owner(&owner));
        Ok(())
    }
    fn finish(mut self) -> Result<LedgerPlan> {
        let changed = !self.writes.is_empty();
        if changed {
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
        if *request != LedgerRequest::Initialize {
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
        let mut batch = WriteBatch::new();
        batch.put(ColumnFamily::Default, key(1, &[]), header(digest, 0));
        return Ok(LedgerPlan {
            batch,
            revision: 0,
            changed: true,
        });
    };
    let revision = read_header(&stored, digest)?;
    let mut p = Planner {
        view,
        root: digest,
        revision,
        writes: BTreeMap::new(),
    };
    match request {
        LedgerRequest::Initialize => {}
        LedgerRequest::Register(resources) => {
            validate_resources(resources, digest)?;
            for identity in resources {
                let record = view.get(ColumnFamily::Default, &resource_key(identity))?;
                let registered =
                    view.get(ColumnFamily::Default, &resource_registration_key(identity))?;
                match (record, registered) {
                    (Some(_), Some(_)) => {
                        let record = p.resource(*identity)?;
                        if record.is_retired() {
                            return Err(conflict("resource instance permanently retired"));
                        }
                    }
                    (None, None) => {
                        p.writes.insert(
                            resource_key(identity),
                            RetentionRecord::new(*identity).encode(),
                        );
                        p.writes.insert(
                            resource_registration_key(identity),
                            resource_registration(identity),
                        );
                    }
                    _ => return Err(invalid("resource registration/state pair is incomplete")),
                }
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
            // Migration pins have no transfer-settlement seam yet: a published
            // destination owner is a description of intent, not durable
            // destination-install evidence, and cannot quiesce its source.
            if matches!(
                source.binding.descriptor.kind,
                OwnerKind::Snapshot | OwnerKind::Migration
            ) || target.binding.descriptor.kind == OwnerKind::Migration
            {
                return Err(conflict(
                    "migration pins cannot quiesce without committed install evidence",
                ));
            }
            p.transition(*from, PinPhase::Quiesced)?;
        }
        LedgerRequest::Release(token) => p.transition(*token, PinPhase::Released)?,
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
    let revision = read_header(&header, digest)?;
    Planner {
        view,
        root: digest,
        revision,
        writes: BTreeMap::new(),
    }
    .owner(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{encode_row_key, memcmp_uint, ColumnValue, RowValue};
    use crate::schema::{ColumnId, REGIONS_DESC, SCHEMA_VERSION, SCHEMA_VERSION_DESC};
    use crate::store::MetaStore;
    use kv9_common::{
        AppliedPosition, BootstrapGeneration, ClusterId, NodeId, RootVoter, StoreIncarnation,
    };
    use kv9_engine::{Engine, MemEngine, ReplicatedEngine, WalEngine};
    use std::sync::Arc;

    fn seed() -> (RootDescriptor, WriteBatch) {
        let root = RootDescriptor::new(
            ClusterId::from_bytes([1; 16]),
            BootstrapGeneration::from_bytes([2; 16]),
            vec![RootVoter {
                node_id: NodeId(1),
                addr: "127.0.0.1:20160".parse().unwrap(),
                store_incarnation: StoreIncarnation::from_bytes([3; 16]),
            }],
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
    fn fixture() -> (MemEngine, RootDescriptor) {
        let (root, batch) = seed();
        let engine = MemEngine::new();
        engine.write(batch).unwrap();
        (engine, root)
    }
    fn resources(root: &RootDescriptor) -> Vec<ResourceIdentity> {
        (1..=2)
            .map(|n| {
                ResourceIdentity::new(root.digest(), ResourceKind::Sst, [n; 16], [n + 8; 32])
                    .unwrap()
            })
            .collect()
    }
    fn binding(root: &RootDescriptor, n: u8) -> OwnerBinding {
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
    fn plan(
        engine: &impl Engine,
        root: &RootDescriptor,
        request: &LedgerRequest,
    ) -> Result<LedgerPlan> {
        plan_retention(engine.snapshot()?.as_ref(), root, request)
    }
    fn apply(engine: &MemEngine, root: &RootDescriptor, request: LedgerRequest) -> u64 {
        let plan = plan(engine, root, &request).unwrap();
        let revision = plan.revision();
        engine.write(plan.into_batch()).unwrap();
        revision
    }
    fn owner(engine: &impl Engine, root: &RootDescriptor, id: OwnerId) -> LedgerOwner {
        retention_owner(engine.snapshot().unwrap().as_ref(), root, id)
            .unwrap()
            .unwrap()
    }
    fn all(engine: &impl Engine) -> Vec<(Vec<u8>, Vec<u8>)> {
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
    fn initialized(engine: &MemEngine, root: &RootDescriptor) {
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
