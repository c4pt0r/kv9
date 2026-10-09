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
mod applied_through_tests {
    use super::{AppliedPosition, AppliedThrough};

    /// The acceptance I committed to in #dev before writing this: ordering must not be able
    /// to answer an identity question, and identity must not leak into an ordering answer.
    #[test]
    fn same_index_different_terms_project_to_one_ordering_answer() {
        // A reused index after failover: two DIFFERENT entries, one index. As identities they
        // differ; as prefix positions they are the same place. Both halves are asserted,
        // because the bug was using the identity answer where the ordering one belongs.
        let a = AppliedPosition { term: 7, index: 42 };
        let b = AppliedPosition { term: 9, index: 42 };
        assert_ne!(
            a, b,
            "distinct entries must remain distinguishable as identities"
        );
        assert_eq!(
            a.through(),
            b.through(),
            "as prefix positions they are the same place; term must not survive projection"
        );
    }

    #[test]
    fn a_lower_index_is_covered_regardless_of_term() {
        // THE DEFECT, stated as a test. The old predicate required `at.term <= cut.term`, so a
        // record at a lower index under a HIGHER term read as "not covered" and legitimate
        // reclaim was refused. Index alone decides.
        let cut = AppliedPosition {
            term: 3,
            index: 100,
        }
        .through();
        let lower_but_newer_term = AppliedPosition {
            term: 99,
            index: 50,
        }
        .through();
        assert!(
            lower_but_newer_term <= cut,
            "a lower index is behind the cut whatever its term"
        );
        assert!(cut.covers(lower_but_newer_term));
        assert!(!lower_but_newer_term.covers(cut));
    }

    #[test]
    fn covers_is_inclusive_at_the_cut() {
        let cut = AppliedThrough::at_index(10);
        assert!(cut.covers(cut), "`through` is inclusive of the cut itself");
    }

    #[test]
    fn the_projection_carries_nothing_but_the_index() {
        // Anti-vacuity for the two tests above: if `through()` kept the term in any form,
        // `same_index_different_terms` would still pass only if equality ignored it. This
        // pins that the value genuinely is the index.
        let p = AppliedPosition { term: 5, index: 77 };
        assert_eq!(p.through().index(), 77);
        assert_eq!(p.through(), AppliedThrough::at_index(77));
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
        for bad in [
            "",
            "0011",
            &text[..31],
            &format!("{text}0"),
            "zz112233445566778899aabbccddeeff",
        ] {
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

/// The exact `(term, index)` at which an entry was **applied** — an apply receipt.
///
/// Lives in `kv9-common` rather than any one crate because it is a cross-layer proof, not a
/// server DTO: the drain worker uses it to decide whether a WAL range may be truncated, GC
/// uses it to know a delete-intent is committed, and membership waits on an exact pair.
///
/// # Not interchangeable with `kv9_raft::ProposedAt`
///
/// The two carry identical fields and mean different things, so they must not be merged on
/// the strength of their shape. `ProposedAt` says *where an entry was placed*; this says
/// *where an entry was applied*. Between the two a leader can change and overwrite the
/// uncommitted slot, so a proposal at `(t, i)` may never apply, or a different entry may
/// apply at `(t, i)` instead. Code that treats a proposal receipt as an apply receipt is
/// claiming a write landed when it may have been discarded — which is precisely why
/// `commit_batch` compares the two rather than assuming they agree.
///
/// Index alone is never sufficient **to identify an entry**: after a failover the new leader
/// may reuse an index, so two different entries can share one. It *is* sufficient to order a
/// prefix — see [`AppliedThrough`], which is the projection reclaim and skip decisions take.
///
/// The unqualified form of this sentence used to read "index alone is never sufficient", which
/// is how a prefix-ordering question ended up being answered with an exact pair: every consumer
/// held a `term` and comparing it looked like extra safety. It is not — it wrongly refuses
/// legitimate reclaim (a record at a lower index under a *higher* term reads as "not covered").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppliedPosition {
    pub term: u64,
    pub index: u64,
}

impl AppliedPosition {
    /// Project to the prefix-ordering view, discarding the term.
    ///
    /// Explicit and one-way on purpose: this is where a caller states "I only need ordering
    /// here". There is no route back, because recovering a term from an index would be the
    /// same reuse-after-failover mistake in reverse.
    pub fn through(self) -> AppliedThrough {
        AppliedThrough(self.index)
    }
}

/// How far a prefix has been applied — an **ordering** answer, never an identity.
///
/// Opaque, and holding only the index. Reclaim, state-machine skip and checkpoint coverage ask
/// "have we reached this far"; none of them may ask "is this the same entry", because a reused
/// index makes that question unanswerable from ordering alone. Giving those consumers a `term`
/// is what let `covers()` compare whole pairs and gate on term, which is the defect this type
/// exists to make unrepresentable (task #16 blocker 1; interface shape ruled by Tess, rev 6).
///
/// Obtained by projection from [`AppliedPosition::through`], so the engine still answers with
/// **one atomic observation** — splitting it into two reads would let identity and ordering come
/// from two different instants, the tearing `write_applied` fuses on the write side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct AppliedThrough(u64);

impl AppliedThrough {
    /// Build from a bare index. For callers that legitimately have only an index (a log
    /// watermark, a reclaim cut) and never had a term to begin with.
    pub fn at_index(index: u64) -> Self {
        AppliedThrough(index)
    }

    /// The index. The only thing this type carries.
    pub fn index(self) -> u64 {
        self.0
    }

    /// Whether this prefix reaches `cut` — the whole question this type answers.
    ///
    /// Index-only by construction. A record at a lower index can never fail to be covered on
    /// account of its term, which is precisely the bug in the predicate this replaces.
    pub fn covers(self, cut: AppliedThrough) -> bool {
        self.0 >= cut.0
    }
}
