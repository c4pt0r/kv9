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
//! from a masked node is dropped, exactly as if the wire were cut. Both
//! directions consult the SAME mask, so this mechanism cuts symmetrically
//! only. If a ReadIndex scenario later needs an asymmetric partition (inbound
//! only / outbound only), that is a new, direction-aware protocol with its own
//! tests — not a configuration of this one.
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
//!
//! A partition therefore lifts by exactly one route: an atomic write of the
//! [`HEAL_TOKEN`]. Empty, truncated, and missing all read as "keep the last
//! mask" — so the read side stays fail-closed without depending on every writer
//! using the atomic path (a raw `> testing-partition` or a `touch` cannot
//! reconnect a peer).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// The on-disk file name, relative to a node's data dir.
pub const PARTITION_FILE: &str = "testing-partition";

/// Env var naming the directory a node reads its partition file from. A harness
/// sets it per node process at launch (pointing at that node's data dir); a node
/// launched without it never partitions, so a production/normal run is
/// unaffected even in a `testing`-feature build.
pub const PARTITION_DIR_ENV: &str = "KV9_TESTING_PARTITION_DIR";

/// The one literal that lifts a partition to fully connected. Healing is an
/// action that must be spelled, never a byte-level accident: a truncated write,
/// a `touch`, or an empty body all parse to "keep the last mask" (fail-closed),
/// so the ONLY way a peer reconnects is a harness writing this word. This closes
/// the last fail-open path — empty and missing are two kinds of "said nothing",
/// and neither may masquerade as "said connected".
pub const HEAL_TOKEN: &str = "connected";

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
        // Any outcome other than a clean parse is "keep the last mask":
        //  - IO error (missing/unreadable): a declared partition must not
        //    evaporate because a read blipped; before first engagement there is
        //    nothing to keep and the node stays connected (normal formation);
        //  - parse failure (torn write, garbage, or an empty/near-miss body):
        //    "said nothing" never lifts or shrinks a partition.
        // Only a clean parse — an id list, or the spelled heal token — updates
        // the live mask and latches engagement.
        if let Ok(contents) = std::fs::read_to_string(&path) {
            if let Some(bits) = parse_mask(&contents) {
                self.bits.store(bits, Ordering::Relaxed);
                self.engaged.store(1, Ordering::Relaxed);
            }
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

/// A [`PartitionMask`] paired with the directory it refreshes from — the shape
/// the transport actually holds. Constructed from [`PARTITION_DIR_ENV`]: when
/// the env var is unset the directory is `None` and [`Self::refresh`] is a no-op,
/// so the mask never engages and the node behaves normally. This is what keeps a
/// `testing`-feature binary safe to run un-partitioned.
#[derive(Debug)]
pub struct PartitionState {
    mask: PartitionMask,
    dir: Option<PathBuf>,
}

impl PartitionState {
    /// Read the partition directory from the environment. Called once when the
    /// transport is built.
    pub fn from_env() -> Self {
        Self {
            mask: PartitionMask::new(),
            dir: std::env::var_os(PARTITION_DIR_ENV).map(PathBuf::from),
        }
    }

    /// Reload the mask from disk if a directory is configured. Called at the
    /// start of every `drain` (every driver tick).
    pub fn refresh(&self) {
        if let Some(dir) = &self.dir {
            self.mask.refresh_from(dir);
        }
    }

    /// Is the peer currently cut off? Checked on send and on drain.
    pub fn is_masked(&self, node_id: u64) -> bool {
        self.mask.is_masked(node_id)
    }

    /// Directly install a partition set, bypassing the file/env path — for
    /// in-process transport tests that must not depend on process-global env.
    /// Since `dir` is `None` in such a state, a later `refresh` is a no-op and
    /// this mask stands.
    #[cfg(test)]
    pub(crate) fn force_mask(&self, ids: &[u64]) {
        let bits = ids
            .iter()
            .filter_map(|&n| bit_for(n))
            .fold(0u64, |a, b| a | b);
        self.mask.bits.store(bits, Ordering::Relaxed);
        self.mask.engaged.store(1, Ordering::Relaxed);
    }
}

impl Default for PartitionState {
    fn default() -> Self {
        Self::from_env()
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

/// Parse a partition file body. The grammar is strict — ids separated by
/// exactly one comma or one whitespace character — and three outcomes exist:
///
/// - the exact heal token → `Some(0)` (fully connected — the only fail-OPEN
///   result, and it must be spelled);
/// - a well-formed non-empty list of valid in-range ids → `Some(bits)` (that
///   partition set, including an explicitly smaller one — a harness naming
///   fewer ids is a deliberate act, not a degenerate read);
/// - anything else — empty, a torn/garbled line, an out-of-range or
///   non-numeric token, or a leading/trailing/doubled separator → `None`,
///   which the caller turns into "keep the last mask".
///
/// Empty tokens are REJECTED, not skipped (review round): `"2,"` is what a
/// torn write of `"2,3"` looks like, so skipping the empty tail would accept a
/// truncation as a deliberately smaller partition — quietly reconnecting a
/// peer the harness meant to keep cut. A separator with nothing after it is a
/// torn line, and a torn line keeps the last mask.
///
/// The asymmetry is the point: an empty body is NOT "no peer masked". Empty and
/// missing are both "said nothing", and neither may reconnect a partitioned
/// peer. Only [`HEAL_TOKEN`] does that, so a truncated write can never be read
/// as a heal.
fn parse_mask(contents: &str) -> Option<u64> {
    let trimmed = contents.trim();
    if trimmed == HEAL_TOKEN {
        return Some(0);
    }
    if trimmed.is_empty() {
        return None; // "said nothing" — keep last, do not heal
    }
    let mut bits = 0u64;
    for token in trimmed.split(|c: char| c.is_whitespace() || c == ',') {
        // An empty token means a leading, trailing, or doubled separator —
        // the shape of a torn write. Reject the whole line.
        if token.is_empty() {
            return None;
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
    // Validate BEFORE publishing (review round): an unmaskable id would be
    // written successfully, then rejected by the next tick's parse, which
    // keeps the previous mask — so the controller would believe the topology
    // switched while it did not. `Ok` from this function must mean "the file
    // now says what you asked"; an invalid request leaves the target file
    // untouched.
    if let Some(bad) = masked.iter().find(|&&n| bit_for(n).is_none()) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("node id {bad} is outside the maskable range 1..=64; partition file unchanged"),
        ));
    }
    // The empty set is a heal, and a heal must be spelled — never an empty body,
    // which reads back as "keep last" (fail-closed).
    let body = if masked.is_empty() {
        HEAL_TOKEN.to_string()
    } else {
        masked
            .iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(",")
    };
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
        m.bits
            .store(bit_for(2).unwrap() | bit_for(5).unwrap(), Ordering::Relaxed);
        assert!(m.is_masked(2));
        assert!(m.is_masked(5));
        assert!(!m.is_masked(3));
        // Out-of-range ids are never masked, whatever the word holds.
        assert!(!m.is_masked(0));
        assert!(!m.is_masked(65));
    }

    #[test]
    fn parse_rejects_any_bad_token_rather_than_partial() {
        assert_eq!(
            parse_mask("2,5"),
            Some(bit_for(2).unwrap() | bit_for(5).unwrap())
        );
        assert_eq!(parse_mask("  3 \n"), Some(bit_for(3).unwrap()));
        // Only the heal token lifts a partition; empty is "said nothing".
        assert_eq!(parse_mask(HEAL_TOKEN), Some(0));
        assert_eq!(parse_mask("  connected \n"), Some(0));
        assert_eq!(parse_mask(""), None); // NOT a heal — keep last
        assert_eq!(parse_mask("   "), None);
        // A torn line ("2,5" cut to "2,5abc" or "2,") must not parse to a
        // SMALLER partition — the whole parse fails, caller keeps last.
        assert_eq!(parse_mask("2,5x"), None);
        assert_eq!(parse_mask("99"), None); // out of range
        assert_eq!(parse_mask("-1"), None);
        // Empty tokens are torn-write shapes, not skippable noise (review
        // round: skipping them accepted "2," — the truncation of "2,3" — as a
        // deliberately smaller partition, quietly reconnecting node 3).
        assert_eq!(parse_mask("2,"), None, "trailing separator = torn write");
        assert_eq!(parse_mask(",2"), None, "leading separator = torn write");
        assert_eq!(parse_mask("2,,3"), None, "doubled separator = torn write");
        // A near-miss of the heal token does not heal.
        assert_eq!(parse_mask("connect"), None);
    }

    /// `Ok` from the writer must mean "the file now says what you asked".
    /// Publishing an unmaskable id and letting the next tick's parse reject it
    /// would leave the controller believing the topology switched while the
    /// node kept its previous mask (review round).
    #[test]
    fn writer_rejects_unmaskable_ids_without_touching_the_file() {
        let dir = std::env::temp_dir().join(format!("kv9-pwtest-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let _ = std::fs::remove_file(dir.join(PARTITION_FILE));

        write_partition_file(&dir, &[2]).unwrap();
        let before = std::fs::read_to_string(dir.join(PARTITION_FILE)).unwrap();

        let err = write_partition_file(&dir, &[65])
            .expect_err("an unmaskable id must be rejected at write time");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        assert!(
            err.to_string().contains("65"),
            "the rejection must name the offending id: {err}"
        );
        // Mixed lists are all-or-nothing: one bad id poisons the whole request.
        write_partition_file(&dir, &[2, 0]).expect_err("id 0 is unmaskable");

        let after = std::fs::read_to_string(dir.join(PARTITION_FILE)).unwrap();
        assert_eq!(
            before, after,
            "a rejected write must leave the partition file unchanged"
        );
        std::fs::remove_dir_all(&dir).ok();
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

        // A RAW empty write (bypassing the atomic helper — a `touch`, a
        // `> file`, a truncated write) must NOT heal: this is the protection
        // Ren caught. Empty is "said nothing", not "connected".
        std::fs::write(dir.join(PARTITION_FILE), "").unwrap();
        m.refresh_from(&dir);
        assert!(m.is_masked(2) && m.is_masked(3));

        // The ONE route that lifts a partition: an atomic write of the empty
        // set, which the helper renders as the spelled heal token.
        write_partition_file(&dir, &[]).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join(PARTITION_FILE)).unwrap(),
            HEAL_TOKEN
        );
        m.refresh_from(&dir);
        assert!(!m.is_masked(2) && !m.is_masked(3));

        std::fs::remove_dir_all(&dir).ok();
    }
}
