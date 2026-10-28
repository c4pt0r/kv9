//! Exact retained metadata source carried by a DATA membership proposal.
//! A decoded origin is descriptive: consumers must independently validate its
//! task against the certified catalog. It grants no membership or GC authority.
use kv9_common::{Error, NodeId, RegionId, Result, RootDigest, StoreIncarnation};
use raft::prelude::{ConfChangeTransition, ConfChangeType, ConfChangeV2};

pub const ORIGIN_BYTES: usize = 169;
const MAGIC: &[u8; 8] = b"KV9CFO01";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Attach,
    Promote,
    Remove,
    Detach,
}
impl Action {
    pub fn change_type(self) -> ConfChangeType {
        match self {
            Self::Attach => ConfChangeType::AddLearnerNode,
            Self::Promote => ConfChangeType::AddNode,
            Self::Remove | Self::Detach => ConfChangeType::RemoveNode,
        }
    }
    pub fn task_kind(self) -> u64 {
        match self {
            Self::Attach => 103,
            Self::Promote => 104,
            Self::Remove => 106,
            Self::Detach => 108,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigurationOrigin {
    pub root: RootDigest,
    pub creation: RootDigest,
    pub region: RegionId,
    pub operation: [u8; 16],
    pub task: u64,
    pub node: NodeId,
    pub store: StoreIncarnation,
    /// Digest of the canonical immutable metadata task row, including payload.
    pub source_digest: RootDigest,
    pub action: Action,
}
impl ConfigurationOrigin {
    pub fn encode(self) -> Result<Vec<u8>> {
        if self.root.as_bytes() == &[0; 32]
            || self.creation.as_bytes() == &[0; 32]
            || self.region.0 < 100
            || self.operation == [0; 16]
            || self.task < 100
            || self.node.0 == 0
            || self.store.as_bytes() == &[0; 16]
            || self.source_digest.as_bytes() == &[0; 32]
        {
            return Err(invalid());
        }
        let mut b = Vec::with_capacity(ORIGIN_BYTES);
        b.extend_from_slice(MAGIC);
        b.extend_from_slice(self.root.as_bytes());
        b.extend_from_slice(self.creation.as_bytes());
        b.extend_from_slice(&self.region.0.to_be_bytes());
        b.extend_from_slice(&self.operation);
        b.extend_from_slice(&self.task.to_be_bytes());
        b.extend_from_slice(&self.action.task_kind().to_be_bytes());
        b.extend_from_slice(&self.node.0.to_be_bytes());
        b.extend_from_slice(self.store.as_bytes());
        b.extend_from_slice(self.source_digest.as_bytes());
        b.push(match self.action {
            Action::Attach => 1,
            Action::Promote => 2,
            Action::Remove => 3,
            Action::Detach => 4,
        });
        Ok(b)
    }
    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() != ORIGIN_BYTES || &b[..8] != MAGIC {
            return Err(invalid());
        }
        let uint = |at| u64::from_be_bytes(b[at..at + 8].try_into().unwrap());
        let out = Self {
            root: RootDigest::from_bytes(b[8..40].try_into().unwrap()),
            creation: RootDigest::from_bytes(b[40..72].try_into().unwrap()),
            region: RegionId(uint(72)),
            operation: b[80..96].try_into().unwrap(),
            task: uint(96),
            node: NodeId(uint(112)),
            store: StoreIncarnation::from_bytes(b[120..136].try_into().unwrap()),
            source_digest: RootDigest::from_bytes(b[136..168].try_into().unwrap()),
            action: match b[168] {
                1 => Action::Attach,
                2 => Action::Promote,
                3 => Action::Remove,
                4 => Action::Detach,
                _ => return Err(invalid()),
            },
        };
        if out.encode()? != b {
            return Err(invalid());
        }
        Ok(out)
    }
    /// Unknown/legacy contexts stay unknown. A recognized malformed context
    /// must not be accepted as either a different node or a different action.
    pub fn from_change(cc: &ConfChangeV2) -> Result<Option<Self>> {
        use protobuf::Message as _;
        if !cc.context.starts_with(MAGIC) {
            return Ok(None);
        }
        let origin = Self::decode(&cc.context)?;
        if cc.get_unknown_fields().iter().next().is_some()
            || cc.transition != ConfChangeTransition::Auto
            || cc.changes.len() != 1
            || cc.changes[0].node_id != origin.node.0
            || cc.changes[0].change_type != origin.action.change_type()
            || cc.changes[0].get_unknown_fields().iter().next().is_some()
        {
            return Err(invalid());
        }
        Ok(Some(origin))
    }
}
fn invalid() -> Error {
    Error::Raft("invalid DATA configuration origin".into())
}

#[cfg(test)]
pub(crate) mod tests;
