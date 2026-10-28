//! Frozen pre-TTL V3 record codec from 4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8.
//! Original selected source SHA-256: 147ea562997111ce50c809f791a9de3bac1fe82b421e8bee7fcf5f9d57277070
//! Test fixture only; this is not a full prior-binary downgrade test.
#![allow(dead_code)]
use crate::{Error, NodeId, Result, RootDigest, StoreIncarnation};

const LEGACY_MAGIC: &[u8; 8] = b"KV9LIFE1";
const PRE_RANGE_MAGIC: &[u8; 8] = b"KV9LIFE2";
const MAGIC: &[u8; 8] = b"KV9LIFE3";
const PAYLOAD_LEN: usize = 8 + 8 + 16 + 1 + 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorePhase {
    Prepared,
    Bound(RootDigest),
    Active(RootDigest),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StoreRecord {
    pub node_id: NodeId,
    pub incarnation: StoreIncarnation,
    pub phase: StorePhase,
}

impl StoreRecord {
    fn encode(self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(PAYLOAD_LEN + 32);
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&self.node_id.0.to_be_bytes());
        bytes.extend_from_slice(self.incarnation.as_bytes());
        let (phase, root) = match self.phase {
            StorePhase::Prepared => (0, RootDigest::from_bytes([0; 32])),
            StorePhase::Bound(root) => (1, root),
            StorePhase::Active(root) => (2, root),
        };
        bytes.push(phase);
        bytes.extend_from_slice(root.as_bytes());
        bytes.extend_from_slice(RootDigest::sha256(&bytes).as_bytes());
        bytes
    }

    fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != PAYLOAD_LEN + 32
            || (&bytes[..8] != MAGIC
                && &bytes[..8] != LEGACY_MAGIC
                && &bytes[..8] != PRE_RANGE_MAGIC)
        {
            return Err(Error::Config(
                "invalid store lifecycle record format".into(),
            ));
        }
        if RootDigest::sha256(&bytes[..PAYLOAD_LEN]).as_bytes() != &bytes[PAYLOAD_LEN..] {
            return Err(Error::Config("store lifecycle checksum mismatch".into()));
        }
        let node_id = NodeId(u64::from_be_bytes(bytes[8..16].try_into().unwrap()));
        let incarnation = StoreIncarnation::from_bytes(bytes[16..32].try_into().unwrap());
        let root = RootDigest::from_bytes(bytes[33..65].try_into().unwrap());
        let phase = match bytes[32] {
            0 if root.as_bytes() == &[0; 32] => StorePhase::Prepared,
            1 => StorePhase::Bound(root),
            2 => StorePhase::Active(root),
            _ => return Err(Error::Config("invalid store lifecycle phase".into())),
        };
        if node_id.0 == 0 || incarnation.as_bytes() == &[0; 16] {
            return Err(Error::Config(
                "store lifecycle identity must be non-zero".into(),
            ));
        }
        Ok(Self {
            node_id,
            incarnation,
            phase,
        })
    }
}

pub(super) fn accepts(bytes: &[u8]) -> bool {
    StoreRecord::decode(bytes).is_ok()
}
