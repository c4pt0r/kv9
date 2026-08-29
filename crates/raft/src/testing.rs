//! Deterministic network-partition injection for consensus tests (task #28).
//!
//! **This module only exists under `cfg(any(test, feature = "testing"))`.** A
//! production build has zero injection surface — a reverse-import probe in a
//! non-test build must fail to compile (the same guard shape as
//! `kv9_engine::testing::FaultyEngine`). The partition mask can silently drop
//! real raft traffic, so it must never ship.
//!
//! # What it does
//!
//! [`PartitionMask`] names a set of node ids that the local node is isolated
//! from. The transport consults it on both send and receive: a message to or
//! from a masked node is dropped, exactly as if the wire were cut. Because both
//! directions consult the same mask, a single process achieves *symmetric*
//! isolation of a peer; an asymmetric partition (cut inbound only, or outbound
//! only) is available from the same mechanism for the ReadIndex asymmetry cases.
//!
//! # Why the mask is loaded from a file, and why atomicity is load-bearing
//!
//! The server refreshes the mask every tick from `data-dir/testing-partition`,
//! so a harness can flip a partition *while the cluster is running* (form →
//! isolate the leader → observe the stale-leader read window → heal). That
//! runtime-flippability is the capability a static `taskset` run could not give.
//!
//! But a partition that *briefly self-heals* is worse than no partition: the
//! isolated stale leader could, for one tick, reach quorum and serve a read —
//! the exact read a ReadIndex E2E requires to be refused. A test would go green
//! while the property under test was violated, and rarely. So the mask has a
//! three-layer safety, every layer biased toward "still partitioned":
//!
//! 1. **Atomic writes.** [`write_partition_file`] writes a temp file and renames
//!    it over the target; a reader sees the old mask or the new one, never a
//!    torn half-line (the `status.tmp` → rename pattern from `runtime.rs`).
//! 2. **Fail-closed reads.** [`PartitionMask::refresh_from`] on any I/O or parse
//!    error KEEPS the previous mask. It never drifts to 0 (= fully connected),
//!    because the unsafe direction of a flaky read is exactly the one that would
//!    reconnect a node that should stay cut off. Same family as "refcount only
//!    over-estimates" and "Unconfirmed is not Failed" — but note the "safe side"
//!    points a different way in each. The one invariant they share, stated
//!    without ambiguity: **when uncertain, bias toward making a green result
//!    harder to get, never easier.** Here that means staying partitioned.
//! 3. **Latching engagement.** The mask is [`Option`]-shaped. `None` means the
//!    harness has not taken control yet (the file has never appeared): the node
//!    forms and serves normally, which is what pre-partition setup needs. The
//!    first time the file appears the mask latches into the "present" state, and
//!    from then on a missing or unreadable file is treated as an error (layer 2:
//!    keep the last mask), NOT as "back to None". Without this, an E2E that
//!    starts nodes before writing the partition file would run with an implicit
//!    mask of 0 during the gap.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// The on-disk file name, relative to a node's data dir.
pub const PARTITION_FILE: &str = "testing-partition";

/// A bitset of peer node ids (1..=64) the local node is cut off from.
///
/// Stored as a single atomic word so the per-message check on the transport hot
/// path is lock-free and allocation-free. Node id `n` occupies bit `n - 1`;
/// ids outside `1..=64` are unrepresentable and rejected at parse time rather
/// than silently ignored (an id we cannot mask must not look "not partitioned").
#[derive(Debug)]
pub struct PartitionMask {
    /// The live bitset consulted by the transport. `0` = no peer masked.
    bits: AtomicU64,
    /// Whether the harness has ever taken control (layer 3). Once `true`, a
    /// missing/unreadable file keeps the last `bits` instead of clearing them.
    engaged: AtomicU64,
}

impl PartitionMask {
    /// A fresh mask in the pre-engagement state: nothing masked, harness not yet
    /// in control. A node with this mask forms and serves normally.
    pub fn new() -> Self {
        Self {
            bits: AtomicU64::new(0),
            engaged: AtomicU64::new(0),
        }
    }

    /// Is the peer `to` currently cut off? The transport calls this on every
    /// send and every receive.
    pub fn is_masked(&self, node_id: u64) -> bool {
        match bit_for(node_id) {
            Some(bit) => self.bits.load(Ordering::Relaxed) & bit != 0,
            // An id we cannot represent is treated as reachable: masking is an
            // explicit, bounded set, and refusing to mask an out-of-range id is
            // safer than pretending the whole word applies to it.
            None => false,
        }
    }

    /// Reload the mask from `data_dir/testing-partition`, applying the three-layer
    /// safety. Called once per server tick. Returns nothing: the failure modes
    /// are handled internally by biasing toward "still partitioned".
    pub fn refresh_from(&self, data_dir: &Path) {
        let path = data_dir.join(PARTITION_FILE);
        match std::fs::read_to_string(&path) {
            Ok(contents) => match parse_mask(&contents) {
                Some(bits) => {
                    self.bits.store(bits, Ordering::Relaxed);
                    self.engaged.store(1, Ordering::Relaxed);
                }
                // File present but unparsable: the harness is mid-write with a
                // non-atomic writer, or wrote garbage. Layer 2 — keep the last
                // mask. If we have never engaged, staying at 0 is correct (the
                // harness has not defined a partition yet); if we have engaged,
                // keeping the last real mask is the fail-closed choice.
                None => {}
            },
            // File missing or unreadable. If engaged, layer 3 says keep the last
            // mask (a partition already declared must not evaporate because the
            // file was momentarily unreadable). If never engaged, there is
            // nothing to keep and the node stays fully connected — normal
            // pre-partition operation.
            Err(_) => {}
        }
    }

    /// Test-only introspection: has the harness taken control at least once?
    #[cfg(test)]
    fn is_engaged(&self) -> bool {
        self.engaged.load(Ordering::Relaxed) != 0
    }
}

impl Default for PartitionMask {
    fn default() -> Self {
        Self::new()
    }
}

/// Bit for a node id, or `None` if the id is outside the maskable range 1..=64.
fn bit_for(node_id: u64) -> Option<u64> {
    if (1..=64).contains(&node_id) {
        Some(1u64 << (node_id - 1))
    } else {
        None
    }
}

/// Parse a partition file body: whitespace/comma-separated node ids, each in
/// 1..=64. Returns `None` (parse failure → caller keeps last mask) on any token
/// that is not a valid in-range id, so a torn or garbled read can never be
/// silently interpreted as a smaller partition. An empty body is a valid
/// explicit "no peer masked" (mask = 0) — distinct from a missing file.
fn parse_mask(contents: &str) -> Option<u64> {
    let mut bits = 0u64;
    for token in contents.split(|c: char| c.is_whitespace() || c == ',') {
        if token.is_empty() {
            continue;
        }
        let id: u64 = token.parse().ok()?;
        bits |= bit_for(id)?;
    }
    Some(bits)
}

/// Atomically publish a partition set to a node's data dir: write a temp file,
/// then rename it over the target. A reader sees the old mask or the new one,
/// never a torn intermediate. Harnesses MUST use this rather than a plain
/// truncating write — a torn read parses to a keep-last (layer 2), but only if
/// the writer never leaves the file in a state a future reader treats as valid
/// and wrong.
pub fn write_partition_file(data_dir: &Path, masked: &[u64]) -> std::io::Result<()> {
    use std::io::Write;
    let body = masked
        .iter()
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let tmp = data_dir.join("testing-partition.tmp");
    let target = data_dir.join(PARTITION_FILE);
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(body.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, &target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_only_the_named_peers() {
        let m = PartitionMask::new();
        assert!(!m.is_masked(2));
        m.bits.store(bit_for(2).unwrap() | bit_for(5).unwrap(), Ordering::Relaxed);
        assert!(m.is_masked(2));
        assert!(m.is_masked(5));
        assert!(!m.is_masked(3));
        // Out-of-range ids are never masked, whatever the word holds.
        assert!(!m.is_masked(0));
        assert!(!m.is_masked(65));
    }

    #[test]
    fn parse_rejects_any_bad_token_rather_than_partial() {
        assert_eq!(parse_mask("2,5"), Some(bit_for(2).unwrap() | bit_for(5).unwrap()));
        assert_eq!(parse_mask("  3 \n"), Some(bit_for(3).unwrap()));
        assert_eq!(parse_mask(""), Some(0)); // explicit no-mask
        // A torn line ("2,5" cut to "2,5abc" or "2,") must not parse to a
        // SMALLER partition — the whole parse fails, caller keeps last.
        assert_eq!(parse_mask("2,5x"), None);
        assert_eq!(parse_mask("99"), None); // out of range
        assert_eq!(parse_mask("-1"), None);
    }

    #[test]
    fn refresh_is_fail_closed_and_latches() {
        let dir = std::env::temp_dir().join(format!("kv9-ptest-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let _ = std::fs::remove_file(dir.join(PARTITION_FILE));

        let m = PartitionMask::new();

        // Layer 3: no file yet → not engaged, nothing masked (normal formation).
        m.refresh_from(&dir);
        assert!(!m.is_engaged());
        assert!(!m.is_masked(2));

        // File appears → engage, mask applied.
        write_partition_file(&dir, &[2, 3]).unwrap();
        m.refresh_from(&dir);
        assert!(m.is_engaged());
        assert!(m.is_masked(2) && m.is_masked(3));

        // File goes missing after engagement → layer 3 keeps the last mask
        // (a declared partition must not evaporate on an unreadable read).
        std::fs::remove_file(dir.join(PARTITION_FILE)).unwrap();
        m.refresh_from(&dir);
        assert!(m.is_masked(2) && m.is_masked(3));

        // A torn/garbled file after engagement → layer 2 keeps the last mask,
        // never drifts to 0.
        std::fs::write(dir.join(PARTITION_FILE), "2,3x").unwrap();
        m.refresh_from(&dir);
        assert!(m.is_masked(2) && m.is_masked(3));

        // An explicit empty file heals the partition (mask = 0) — the ONE way a
        // partition lifts is an atomic write of the empty set.
        write_partition_file(&dir, &[]).unwrap();
        m.refresh_from(&dir);
        assert!(!m.is_masked(2) && !m.is_masked(3));

        std::fs::remove_dir_all(&dir).ok();
    }
}
