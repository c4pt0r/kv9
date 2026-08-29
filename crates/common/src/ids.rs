//! Cluster identifier newtypes (DESIGN §3).
//!
//! These are deliberately thin newtypes over integers so the type system prevents
//! mixing, e.g., a `RegionId` where a `KeyspaceId` is expected.

use serde::{Deserialize, Serialize};

/// Identifies one `kv9` process / store in the cluster (DESIGN §3.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct NodeId(pub u64);

/// Identifies a region (range shard = Raft group) (DESIGN §3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RegionId(pub u64);

/// The well-known, fixed region id of the L0 bootstrap meta group `META_REGION_0`
/// (DESIGN §5.1.1, §5.2). It covers the system key range and never grows.
pub const META_REGION_0: RegionId = RegionId(1);

/// Identifies a keyspace (DESIGN §3.2). Physically 3 bytes on the wire / in keys
/// (DESIGN §3.4), so the valid range is `0..=0x00FF_FFFF` (2^24 keyspaces).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct KeyspaceId(pub u32);

impl KeyspaceId {
    /// The reserved system keyspace (`keyspace_id = 0`, mode `'s'`) — DESIGN §5.
    pub const SYSTEM: KeyspaceId = KeyspaceId(0);

    /// Maximum encodable keyspace id given the 3-byte on-disk width (DESIGN §3.4).
    pub const MAX: u32 = 0x00FF_FFFF;
}

/// Identifies a tenant: the isolation and accounting boundary (DESIGN §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TenantId(pub u64);

impl TenantId {
    /// The default tenant created at bootstrap (DESIGN §5.2).
    pub const DEFAULT: TenantId = TenantId(0);
}

/// Identifies a transaction/consistency domain = timestamp shard (DESIGN §3.6, §8.1).
///
/// Every `txn` keyspace belongs to exactly one txn group; a transaction never crosses
/// a group boundary (the confinement invariant), which is what lets each group own an
/// independent, sharded TSO timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TxnGroupId(pub u64);

impl TxnGroupId {
    /// The `default` txn group — one timeline, behaves like a single classic TSO
    /// (DESIGN §3.6, §8.1).
    pub const DEFAULT: TxnGroupId = TxnGroupId(0);
}

/// Identifies one TSO timeline (1:1 with a txn group) — DESIGN §8.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TimelineId(pub u64);

/// Identifies a TSO provider (pool member) hosting one or more timelines — DESIGN §8.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TsoProviderId(pub u64);

/// The immutable identity of one bootstrapped cluster (task #24, three-gate
/// membership contract, gate 2).
///
/// Minted ONCE, from OS entropy, by the bootstrap winner — recorded in the
/// first committed catalog entries and the init marker. After initialization
/// it is the ONLY steady-state cluster identity: joins and restarts verify it,
/// and the bootstrap voter-set fingerprint retires (the fingerprint exists
/// solely to keep two *uninitialized* seed sets from cross-endorsing).
///
/// Wire/text form is exactly 32 hex characters (lowercase on output; either
/// case accepted on input). Anything else is a typed error — a cluster id is
/// never a free-form string, and a wrong one must fail loudly (a node joining
/// the wrong environment is pollution that looks healthy from both sides).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ClusterId([u8; 16]);

impl ClusterId {
    /// Mint a fresh id from OS entropy (`/dev/urandom`). Only the bootstrap
    /// initializer calls this, exactly once per cluster lifetime.
    pub fn mint() -> crate::Result<ClusterId> {
        use std::io::Read;
        let mut bytes = [0u8; 16];
        std::fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut bytes))
            .map_err(|e| crate::Error::Config(format!("cluster id entropy: {e}")))?;
        Ok(ClusterId(bytes))
    }

    pub fn from_bytes(bytes: [u8; 16]) -> ClusterId {
        ClusterId(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl std::fmt::Display for ClusterId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for b in &self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

impl std::fmt::Debug for ClusterId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ClusterId({self})")
    }
}

impl std::str::FromStr for ClusterId {
    type Err = crate::Error;

    fn from_str(s: &str) -> crate::Result<ClusterId> {
        // Never echo the rejected input: the most common way to reach this
        // error is a value pasted into the wrong slot — and the value sitting
        // next to `--cluster-id` in a join config is the one-time join
        // ticket, which must never appear in logs. Length + first bad offset
        // diagnose the mistake without reproducing the secret (Cindy's
        // review of d504d9e).
        if s.len() != 32 {
            return Err(crate::Error::Config(format!(
                "cluster id must be exactly 32 hex characters (got {} chars)",
                s.len()
            )));
        }
        if let Some(bad) = s.bytes().position(|b| !b.is_ascii_hexdigit()) {
            return Err(crate::Error::Config(format!(
                "cluster id must be hex; invalid character at offset {bad}"
            )));
        }
        let mut bytes = [0u8; 16];
        // `as_chunks::<2>()` rather than `chunks_exact(2)`: clippy 1.98 rejects the latter
        // for a constant chunk size (`chunks_exact_to_as_chunks`). The length is already
        // pinned at 32 above, so the remainder `.1` is provably empty and is dropped.
        for (i, chunk) in s.as_bytes().as_chunks::<2>().0.iter().enumerate() {
            let hex = std::str::from_utf8(chunk).expect("ascii checked above");
            bytes[i] = u8::from_str_radix(hex, 16).expect("hexdigit checked above");
        }
        Ok(ClusterId(bytes))
    }
}

#[cfg(test)]
mod cluster_id_tests {
    use super::ClusterId;
    use std::str::FromStr;

    #[test]
    fn display_parse_roundtrip_and_strictness() {
        let id = ClusterId::from_bytes([
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
            0xee, 0xff,
        ]);
        let text = id.to_string();
        assert_eq!(text, "00112233445566778899aabbccddeeff");
        assert_eq!(ClusterId::from_str(&text).unwrap(), id);
        // Uppercase input is accepted; output stays lowercase.
        assert_eq!(ClusterId::from_str(&text.to_uppercase()).unwrap(), id);
        // Anything that is not exactly 32 hex chars is a typed error.
        for bad in ["", "0011", &text[..31], &format!("{text}0"), "zz112233445566778899aabbccddeeff"] {
            assert!(ClusterId::from_str(bad).is_err(), "accepted {bad:?}");
        }
    }

    /// Two mints must differ — the control that entropy is actually read
    /// (an all-zeros stub would pass every other test).
    #[test]
    fn mint_draws_entropy() {
        let a = ClusterId::mint().unwrap();
        let b = ClusterId::mint().unwrap();
        assert_ne!(a, b);
        assert_ne!(a.as_bytes(), &[0u8; 16]);
    }
}
