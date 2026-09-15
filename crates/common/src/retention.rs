//! Bounded per-resource retention transitions and recovery records.
//!
//! This is a deterministic state-machine component, not a durable pin service.
//! An enclosing replicated ledger must bind owner IDs to their full operation /
//! scope, commit these transitions, and persist the result before using it.
//! Decoding or retiring a record grants no physical deletion capability.
//! Published references, reader draining, settlement evidence and cross-resource
//! atomicity remain obligations of that ledger. No production GC uses this API.
use std::collections::BTreeMap;
use std::num::NonZeroU64;

use sha2::{Digest, Sha256};

use crate::RootDigest;

const MAGIC: &[u8; 8] = b"KV9PIN01";
const HEADER: usize = 8 + 32 + 1 + 16 + 32 + 8 + 1 + 4;
const OWNER_BYTES: usize = 16 + 8 + 1;
/// Includes released-owner tombstones. Capacity exhaustion never evicts them.
pub const MAX_PIN_OWNERS: usize = 4096;
pub const MAX_PIN_RECORD_BYTES: usize = HEADER + MAX_PIN_OWNERS * OWNER_BYTES + 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PinError {
    #[error("invalid retention identity or owner generation")]
    InvalidIdentity,
    #[error("retention record has an unknown format or invalid length")]
    Format,
    #[error("retention record checksum mismatch")]
    Checksum,
    #[error("retention record belongs to a different resource")]
    WrongResource,
    #[error("retention record is inconsistent")]
    Inconsistent,
    #[error("resource instance is permanently retired")]
    Retired,
    #[error("owner generation does not match")]
    Generation,
    #[error("owner is absent or ineligible for this transition")]
    OwnerState,
    #[error("resource still has retention owners")]
    Pinned,
    #[error("retention owner capacity exhausted")]
    Capacity,
    #[error("retention revision exhausted")]
    Exhausted,
    #[error("retention revision changed before retirement")]
    Revision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ResourceKind {
    Sst = 1,
    Manifest = 2,
    WalSegment = 3,
    Snapshot = 4,
    HistoryEvidence = 5,
    Backup = 6,
}

/// The root digest binds the cluster; instance identity is distinct from content
/// identity. Identical content after retirement requires a different instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceIdentity {
    root: RootDigest,
    kind: ResourceKind,
    instance: [u8; 16],
    content: [u8; 32],
}
impl ResourceIdentity {
    pub fn root(&self) -> RootDigest {
        self.root
    }
    pub fn kind(&self) -> ResourceKind {
        self.kind
    }
    pub fn instance(&self) -> &[u8; 16] {
        &self.instance
    }
    pub fn content(&self) -> &[u8; 32] {
        &self.content
    }
    pub fn new(
        root: RootDigest,
        kind: ResourceKind,
        instance: [u8; 16],
        content: [u8; 32],
    ) -> Result<Self, PinError> {
        if root.as_bytes() == &[0; 32] || instance == [0; 16] {
            return Err(PinError::InvalidIdentity);
        }
        Ok(Self {
            root,
            kind,
            instance,
            content,
        })
    }
}

/// Opaque identity of an owner descriptor in the enclosing ledger. It must not
/// be reused for a different operation or scope. This module does not mint IDs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct OwnerId([u8; 16]);
impl OwnerId {
    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
    pub fn new(bytes: [u8; 16]) -> Result<Self, PinError> {
        if bytes == [0; 16] {
            return Err(PinError::InvalidIdentity);
        }
        Ok(Self(bytes))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnerToken {
    pub owner: OwnerId,
    pub generation: NonZeroU64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PinPhase {
    Held = 1,
    Published = 2,
    Quiesced = 3,
    Released = 4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnerSlot {
    pub generation: NonZeroU64,
    pub phase: PinPhase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinCommand {
    Acquire(OwnerToken),
    Publish(OwnerToken),
    /// The enclosing ledger must first stop publication/admission and establish
    /// that all uses ended (or transfer their protection to another owner).
    Quiesce(OwnerToken),
    Release(OwnerToken),
    /// Acquire destination protection while retaining the source. Publishing
    /// and quiescing/releasing the old owner are subsequent ledger transitions.
    Share {
        from: OwnerToken,
        to: OwnerToken,
    },
    Retire {
        expected_revision: u64,
    },
}

/// Observation of an in-memory transition. This is not a durable receipt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinTransition {
    Changed { revision: u64 },
    Unchanged { revision: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetentionRecord {
    identity: ResourceIdentity,
    revision: u64,
    retired: bool,
    owners: BTreeMap<OwnerId, OwnerSlot>,
}
impl RetentionRecord {
    /// Only a proved new resource may start here. Missing/corrupt persistent
    /// state must be refused by the ledger, never replaced with an empty record.
    pub fn new(identity: ResourceIdentity) -> Self {
        Self {
            identity,
            revision: 0,
            retired: false,
            owners: BTreeMap::new(),
        }
    }
    pub fn identity(&self) -> ResourceIdentity {
        self.identity
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn is_retired(&self) -> bool {
        self.retired
    }
    pub fn owner(&self, owner: OwnerId) -> Option<OwnerSlot> {
        self.owners.get(&owner).copied()
    }

    fn exact(&self, token: OwnerToken) -> Result<PinPhase, PinError> {
        let slot = self.owners.get(&token.owner).ok_or(PinError::OwnerState)?;
        if slot.generation != token.generation {
            return Err(PinError::Generation);
        }
        Ok(slot.phase)
    }
    fn acquire(&self, token: OwnerToken) -> Result<Option<PinPhase>, PinError> {
        if self.retired {
            return Err(PinError::Retired);
        }
        match self.owners.get(&token.owner) {
            None => {
                if token.generation.get() != 1 {
                    return Err(PinError::Generation);
                }
                if self.owners.len() == MAX_PIN_OWNERS {
                    return Err(PinError::Capacity);
                }
            }
            Some(slot) if slot.generation == token.generation => {
                return match slot.phase {
                    PinPhase::Held | PinPhase::Published => Ok(None),
                    PinPhase::Quiesced | PinPhase::Released => Err(PinError::OwnerState),
                };
            }
            Some(slot) => {
                if slot.phase != PinPhase::Released
                    || slot.generation.get().checked_add(1) != Some(token.generation.get())
                {
                    return Err(PinError::Generation);
                }
            }
        }
        Ok(Some(PinPhase::Held))
    }

    /// All fallible checks precede mutation. Refusal leaves byte-identical state.
    /// Exact retries do not advance revision or revive a quiesced/released owner.
    pub fn apply(&mut self, command: PinCommand) -> Result<PinTransition, PinError> {
        let edit = match command {
            PinCommand::Acquire(token) => self.acquire(token)?.map(|phase| (token, phase)),
            PinCommand::Share { from, to } => {
                if from.owner == to.owner {
                    return Err(PinError::InvalidIdentity);
                }
                if !matches!(self.exact(from)?, PinPhase::Held | PinPhase::Published) {
                    return Err(PinError::OwnerState);
                }
                self.acquire(to)?.map(|phase| (to, phase))
            }
            PinCommand::Publish(token) => match self.exact(token)? {
                PinPhase::Held => Some((token, PinPhase::Published)),
                PinPhase::Published => None,
                _ => return Err(PinError::OwnerState),
            },
            PinCommand::Quiesce(token) => match self.exact(token)? {
                PinPhase::Held | PinPhase::Published => Some((token, PinPhase::Quiesced)),
                PinPhase::Quiesced => None,
                PinPhase::Released => return Err(PinError::OwnerState),
            },
            PinCommand::Release(token) => match self.exact(token)? {
                PinPhase::Quiesced => Some((token, PinPhase::Released)),
                PinPhase::Released => None,
                _ => return Err(PinError::Pinned),
            },
            PinCommand::Retire { expected_revision } => {
                if self.retired {
                    return Ok(PinTransition::Unchanged {
                        revision: self.revision,
                    });
                }
                if self.revision != expected_revision {
                    return Err(PinError::Revision);
                }
                if self
                    .owners
                    .values()
                    .any(|slot| slot.phase != PinPhase::Released)
                {
                    return Err(PinError::Pinned);
                }
                let revision = self.revision.checked_add(1).ok_or(PinError::Exhausted)?;
                self.retired = true;
                self.revision = revision;
                return Ok(PinTransition::Changed { revision });
            }
        };
        let Some((token, phase)) = edit else {
            return Ok(PinTransition::Unchanged {
                revision: self.revision,
            });
        };
        let revision = self.revision.checked_add(1).ok_or(PinError::Exhausted)?;
        self.owners.insert(
            token.owner,
            OwnerSlot {
                generation: token.generation,
                phase,
            },
        );
        self.revision = revision;
        Ok(PinTransition::Changed { revision })
    }

    /// Canonical owner ordering; checksum binds format, full identity and state.
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(HEADER + self.owners.len() * OWNER_BYTES + 32);
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(self.identity.root.as_bytes());
        bytes.push(self.identity.kind as u8);
        bytes.extend_from_slice(&self.identity.instance);
        bytes.extend_from_slice(&self.identity.content);
        bytes.extend_from_slice(&self.revision.to_be_bytes());
        bytes.push(u8::from(self.retired));
        bytes.extend_from_slice(&(self.owners.len() as u32).to_be_bytes());
        for (owner, slot) in &self.owners {
            bytes.extend_from_slice(&owner.0);
            bytes.extend_from_slice(&slot.generation.get().to_be_bytes());
            bytes.push(slot.phase as u8);
        }
        bytes.extend_from_slice(&Sha256::digest(&bytes));
        bytes
    }

    /// Validate the expected resource before returning recovered state. No I/O,
    /// namespace repair, or durable authority is performed by this decoder.
    pub fn decode(bytes: &[u8], expected: ResourceIdentity) -> Result<Self, PinError> {
        if bytes.len() < HEADER + 32
            || bytes.len() > MAX_PIN_RECORD_BYTES
            || !bytes.starts_with(MAGIC)
        {
            return Err(PinError::Format);
        }
        let end = bytes.len() - 32;
        if Sha256::digest(&bytes[..end]).as_slice() != &bytes[end..] {
            return Err(PinError::Checksum);
        }
        let kind = match bytes[40] {
            1 => ResourceKind::Sst,
            2 => ResourceKind::Manifest,
            3 => ResourceKind::WalSegment,
            4 => ResourceKind::Snapshot,
            5 => ResourceKind::HistoryEvidence,
            6 => ResourceKind::Backup,
            _ => return Err(PinError::Format),
        };
        let identity = ResourceIdentity::new(
            RootDigest::from_bytes(bytes[8..40].try_into().unwrap()),
            kind,
            bytes[41..57].try_into().unwrap(),
            bytes[57..89].try_into().unwrap(),
        )?;
        if identity != expected {
            return Err(PinError::WrongResource);
        }
        let revision = u64::from_be_bytes(bytes[89..97].try_into().unwrap());
        let retired = match bytes[97] {
            0 => false,
            1 => true,
            _ => return Err(PinError::Format),
        };
        let count = u32::from_be_bytes(bytes[98..102].try_into().unwrap()) as usize;
        if count > MAX_PIN_OWNERS || HEADER + count * OWNER_BYTES != end {
            return Err(PinError::Format);
        }
        if retired && revision == 0 {
            return Err(PinError::Inconsistent);
        }
        let mut owners = BTreeMap::new();
        let mut previous = None;
        for chunk in bytes[HEADER..end].chunks_exact(OWNER_BYTES) {
            let owner = OwnerId::new(chunk[..16].try_into().unwrap())?;
            let generation = NonZeroU64::new(u64::from_be_bytes(chunk[16..24].try_into().unwrap()))
                .ok_or(PinError::InvalidIdentity)?;
            let phase = match chunk[24] {
                1 => PinPhase::Held,
                2 => PinPhase::Published,
                3 => PinPhase::Quiesced,
                4 => PinPhase::Released,
                _ => return Err(PinError::Format),
            };
            if previous.is_some_and(|id| id >= owner)
                || generation.get() > revision
                || (retired && phase != PinPhase::Released)
            {
                return Err(PinError::Inconsistent);
            }
            owners.insert(owner, OwnerSlot { generation, phase });
            previous = Some(owner);
        }
        Ok(Self {
            identity,
            revision,
            retired,
            owners,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn identity(instance: u8) -> ResourceIdentity {
        ResourceIdentity::new(
            RootDigest::from_bytes([9; 32]),
            ResourceKind::Sst,
            [instance; 16],
            [8; 32],
        )
        .unwrap()
    }
    fn token(owner: u64, generation: u64) -> OwnerToken {
        let mut bytes = [0; 16];
        bytes[..8].copy_from_slice(&owner.to_be_bytes());
        OwnerToken {
            owner: OwnerId::new(bytes).unwrap(),
            generation: NonZeroU64::new(generation).unwrap(),
        }
    }
    fn refuse(record: &mut RetentionRecord, command: PinCommand, expected: PinError) {
        let before = record.encode();
        assert_eq!(record.apply(command), Err(expected));
        assert_eq!(
            record.encode(),
            before,
            "refusal must not partially mutate state"
        );
    }
    fn release(record: &mut RetentionRecord, owner: OwnerToken) {
        record.apply(PinCommand::Quiesce(owner)).unwrap();
        record.apply(PinCommand::Release(owner)).unwrap();
    }
    fn rehash(bytes: &mut [u8]) {
        let end = bytes.len() - 32;
        let hash = Sha256::digest(&bytes[..end]);
        bytes[end..].copy_from_slice(&hash);
    }
    #[test]
    fn publication_retirement_and_retries_preserve_protection() {
        let mut record = RetentionRecord::new(identity(1));
        let owner = token(1, 1);
        refuse(
            &mut record,
            PinCommand::Publish(owner),
            PinError::OwnerState,
        );
        record.apply(PinCommand::Acquire(owner)).unwrap();
        record.apply(PinCommand::Publish(owner)).unwrap();
        let before = record.encode();
        record.apply(PinCommand::Acquire(owner)).unwrap();
        record.apply(PinCommand::Publish(owner)).unwrap();
        assert_eq!(record.encode(), before);
        refuse(&mut record, PinCommand::Release(owner), PinError::Pinned);
        let revision = record.revision();
        refuse(
            &mut record,
            PinCommand::Retire {
                expected_revision: revision,
            },
            PinError::Pinned,
        );
        record.apply(PinCommand::Quiesce(owner)).unwrap();
        refuse(
            &mut record,
            PinCommand::Publish(owner),
            PinError::OwnerState,
        );
        let revision = record.revision();
        refuse(
            &mut record,
            PinCommand::Retire {
                expected_revision: revision,
            },
            PinError::Pinned,
        );
        record.apply(PinCommand::Release(owner)).unwrap();
        let revision = record.revision();
        record
            .apply(PinCommand::Retire {
                expected_revision: revision,
            })
            .unwrap();
        record = RetentionRecord::decode(&record.encode(), identity(1)).unwrap();
        refuse(
            &mut record,
            PinCommand::Acquire(token(1, 2)),
            PinError::Retired,
        );
        refuse(
            &mut record,
            PinCommand::Acquire(token(2, 1)),
            PinError::Retired,
        );
        assert!(record.is_retired());
        let mut replacement = RetentionRecord::new(identity(2));
        replacement.apply(PinCommand::Acquire(token(1, 1))).unwrap();
    }
    #[test]
    fn stale_owner_messages_cannot_release_or_revive_another_generation() {
        let mut record = RetentionRecord::new(identity(1));
        let old = token(1, 1);
        record.apply(PinCommand::Acquire(old)).unwrap();
        release(&mut record, old);
        refuse(&mut record, PinCommand::Acquire(old), PinError::OwnerState);
        refuse(
            &mut record,
            PinCommand::Acquire(token(1, 3)),
            PinError::Generation,
        );
        record.apply(PinCommand::Acquire(token(1, 2))).unwrap();
        record.apply(PinCommand::Publish(token(1, 2))).unwrap();
        for command in [
            PinCommand::Release(old),
            PinCommand::Quiesce(old),
            PinCommand::Publish(old),
        ] {
            refuse(&mut record, command, PinError::Generation);
        }
        assert_eq!(record.owner(old.owner).unwrap().phase, PinPhase::Published);
    }
    #[test]
    fn sharing_keeps_both_owners_and_failed_handoff_keeps_the_source() {
        let mut record = RetentionRecord::new(identity(1));
        let from = token(1, 1);
        let to = token(2, 1);
        record.apply(PinCommand::Acquire(from)).unwrap();
        record.apply(PinCommand::Publish(from)).unwrap();
        refuse(
            &mut record,
            PinCommand::Share {
                from,
                to: token(2, 2),
            },
            PinError::Generation,
        );
        record.apply(PinCommand::Share { from, to }).unwrap();
        assert_eq!(record.owner(from.owner).unwrap().phase, PinPhase::Published);
        assert_eq!(record.owner(to.owner).unwrap().phase, PinPhase::Held);
        record.apply(PinCommand::Publish(to)).unwrap();
        release(&mut record, from);
        let revision = record.revision();
        refuse(
            &mut record,
            PinCommand::Retire {
                expected_revision: revision,
            },
            PinError::Pinned,
        );
    }
    #[test]
    fn capacity_and_revision_exhaustion_do_not_drop_tombstones_or_pins() {
        let mut record = RetentionRecord::new(identity(1));
        for id in 1..=MAX_PIN_OWNERS as u64 {
            let owner = token(id, 1);
            record.apply(PinCommand::Acquire(owner)).unwrap();
            release(&mut record, owner);
        }
        refuse(
            &mut record,
            PinCommand::Acquire(token(MAX_PIN_OWNERS as u64 + 1, 1)),
            PinError::Capacity,
        );
        record.apply(PinCommand::Acquire(token(1, 2))).unwrap();
        refuse(
            &mut record,
            PinCommand::Share {
                from: token(1, 2),
                to: token(9000, 1),
            },
            PinError::Capacity,
        );
        assert_eq!(
            RetentionRecord::decode(&record.encode(), identity(1)).unwrap(),
            record
        );
        record.revision = u64::MAX;
        refuse(
            &mut record,
            PinCommand::Publish(token(1, 2)),
            PinError::Exhausted,
        );
        let mut empty = RetentionRecord::new(identity(1));
        empty.revision = u64::MAX;
        refuse(
            &mut empty,
            PinCommand::Retire {
                expected_revision: u64::MAX,
            },
            PinError::Exhausted,
        );
    }
    #[test]
    fn retirement_checks_the_sampled_revision() {
        let mut record = RetentionRecord::new(identity(1));
        record.apply(PinCommand::Acquire(token(1, 1))).unwrap();
        release(&mut record, token(1, 1));
        refuse(
            &mut record,
            PinCommand::Retire {
                expected_revision: 0,
            },
            PinError::Revision,
        );
    }
    #[test]
    fn recovery_checks_every_torn_cut_and_each_corrupt_byte() {
        let mut record = RetentionRecord::new(identity(1));
        record.apply(PinCommand::Acquire(token(1, 1))).unwrap();
        let bytes = record.encode();
        for cut in 0..bytes.len() {
            assert!(
                RetentionRecord::decode(&bytes[..cut], identity(1)).is_err(),
                "cut {cut}"
            );
        }
        for at in 0..bytes.len() {
            let mut damaged = bytes.clone();
            damaged[at] ^= 1;
            assert!(
                RetentionRecord::decode(&damaged, identity(1)).is_err(),
                "byte {at}"
            );
        }
        assert_eq!(
            RetentionRecord::decode(&bytes, identity(2)),
            Err(PinError::WrongResource)
        );
        let other_root = ResourceIdentity::new(
            RootDigest::from_bytes([7; 32]),
            ResourceKind::Sst,
            [1; 16],
            [8; 32],
        )
        .unwrap();
        assert_eq!(
            RetentionRecord::decode(&bytes, other_root),
            Err(PinError::WrongResource)
        );
    }
    #[test]
    fn valid_checksum_does_not_bypass_semantic_or_format_refusals() {
        let mut record = RetentionRecord::new(identity(1));
        record.apply(PinCommand::Acquire(token(1, 1))).unwrap();
        record.apply(PinCommand::Acquire(token(2, 1))).unwrap();
        let bytes = record.encode();
        for (at, value) in [
            (7, b'2'),
            (40, 255),
            (97, 1),
            (97, 2),
            (HEADER + 24, 0),
            (101, 255),
        ] {
            let mut bad = bytes.clone();
            bad[at] = value;
            rehash(&mut bad);
            assert!(
                RetentionRecord::decode(&bad, identity(1)).is_err(),
                "offset {at}"
            );
        }
        for range in [HEADER..HEADER + 16, HEADER + 16..HEADER + 24, 89..97] {
            let mut bad = bytes.clone();
            bad[range].fill(0);
            rehash(&mut bad);
            assert!(RetentionRecord::decode(&bad, identity(1)).is_err());
        }
        let mut duplicate = bytes.clone();
        duplicate[HEADER + OWNER_BYTES..HEADER + OWNER_BYTES + 16]
            .copy_from_slice(&bytes[HEADER..HEADER + 16]);
        rehash(&mut duplicate);
        assert_eq!(
            RetentionRecord::decode(&duplicate, identity(1)),
            Err(PinError::Inconsistent)
        );
        let mut trailing = bytes.clone();
        trailing.insert(HEADER, 0);
        rehash(&mut trailing);
        assert!(RetentionRecord::decode(&trailing, identity(1)).is_err());
        assert_eq!(
            RetentionRecord::decode(&vec![0; MAX_PIN_RECORD_BYTES + 1], identity(1)),
            Err(PinError::Format)
        );
    }
}
