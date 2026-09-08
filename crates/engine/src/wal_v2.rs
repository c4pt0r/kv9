//! WAL v2 record layout, shared by the reader and writer in `wal`.
//!
//! # The frame
//!
//! v1 records are `magic(4) version(1) len(4, LE) payload(len) crc(4, LE over version..payload)`.
//! v2 keeps that skeleton and inserts a **kind** byte, because not every record carries a
//! position:
//!
//! ```text
//! unpositioned:  magic(4) version(1)=2 kind(1)=0 len(4,LE)                     payload(len) crc(4,LE)
//! positioned:    magic(4) version(1)=2 kind(1)=1 term(8,LE) index(8,LE) len(4,LE) payload(len) crc(4,LE)
//! ```
//!
//! The CRC covers everything from the version byte through the payload — so it covers the
//! kind byte and the position. A flipped kind byte, or a corrupted index, fails the CRC
//! rather than being read as a different record.
//!
//! # Why a kind byte rather than a sentinel position
//!
//! An unpositioned record could have been expressed as a positioned one carrying, say,
//! `(0, 0)`. That would make "no position" a *value* in the same space as real positions,
//! and every consumer would have to remember to special-case it — the same shape as an
//! engine reporting applied index `0` when it means "volatile", which
//! [`DurableAppliedPosition`](crate::replicated::DurableAppliedPosition) exists to prevent.
//! A separate kind means a reader that forgets the case does not compile, instead of
//! reclaiming up to index 0 and calling it correct.
//!
//! # Why unpositioned records exist at all
//!
//! Not every write into the engine comes from replicated apply — bootstrap and local
//! bookkeeping do not have a `(term, index)` to record, and inventing one would be a claim
//! about replicated progress that nothing made.
//!
//! The consequence is a **reclaim ban**, and it is the reason this distinction has to be
//! visible in the format rather than in a comment. A segment holding any unpositioned record
//! cannot be reclaimed on the strength of its positions: the greatest position in the
//! segment does not describe the unpositioned record, so reclaiming up to it would discard a
//! write nothing has accounted for. A reader must be able to answer "does this segment
//! contain an unpositioned record" from the frames alone, without decoding payloads.
//!
//! # What is deliberately absent
//!
//! No constant, and no space in the frame, for a "replay reached here" or
//! `local_applied_prefix` value. Per the task #13 acceptance (2c), a non-monotonic index
//! across complete, CRC-correct records fails the **whole open** with a typed error and does
//! not yield a usable prefix. A field to hold such a prefix would be the first step toward
//! handing one back.

/// Unchanged from v1: the file is the same log, the records inside it are versioned.
pub const MAGIC: [u8; 4] = *b"KV9W";

/// The version this module describes. v1 records remain readable; see [`VERSION_V1`].
pub const VERSION_V2: u8 = 2;

/// v1's version byte, named so that a reader distinguishing the two is not comparing
/// against a bare literal.
pub const VERSION_V1: u8 = 1;

/// Record kinds. The value is on the wire, so these are fixed forever.
pub mod kind {
    /// Carries no replicated position. See the module docs on the reclaim ban.
    pub const UNPOSITIONED: u8 = 0;
    /// Carries `(term, index)` — the exact [`AppliedPosition`](kv9_common::AppliedPosition)
    /// at which the batch applied.
    pub const POSITIONED: u8 = 1;
}

/// Bytes of a `(term, index)` pair on the wire: two LE `u64`s.
pub const POSITION_LEN: usize = 8 + 8;

/// `magic(4) version(1) kind(1)` — present on every v2 record, whatever its kind.
pub const PREAMBLE_LEN: usize = 4 + 1 + 1;

/// The `len` field.
pub const LEN_FIELD_LEN: usize = 4;

/// Trailing CRC-32.
pub const CRC_LEN: usize = 4;

/// Fixed bytes before the payload, for each kind.
pub const UNPOSITIONED_HEADER_LEN: usize = PREAMBLE_LEN + LEN_FIELD_LEN;
/// See [`UNPOSITIONED_HEADER_LEN`].
pub const POSITIONED_HEADER_LEN: usize = PREAMBLE_LEN + POSITION_LEN + LEN_FIELD_LEN;

/// Guards against a corrupt length field driving a huge allocation. Carried over from v1
/// unchanged — a single batch larger than this is a bug, not a legitimate write (DESIGN §13
/// principle 13).
pub const MAX_RECORD_LEN: u32 = 64 * 1024 * 1024;

/// Total on-disk bytes of a record, given its kind and payload length.
///
/// A function rather than each call site adding up header constants, because that addition
/// is where an off-by-one hides: a reader that advances by a slightly wrong amount lands
/// mid-frame, fails the magic check, and reports a *torn tail* — a plausible, wrong answer
/// that looks exactly like the crash this format is designed to tolerate.
pub const fn record_len(kind: u8, payload_len: usize) -> Option<usize> {
    let header = match kind {
        kind::UNPOSITIONED => UNPOSITIONED_HEADER_LEN,
        kind::POSITIONED => POSITIONED_HEADER_LEN,
        // Not a panic: an unknown kind is untrusted input from a file, and DESIGN §13
        // principle 12 is "never panic on the unknown".
        _ => return None,
    };
    Some(header + payload_len + CRC_LEN)
}

// Compile-time, because these are wire facts: a build that violates one has already made
// files it cannot read back, and a test would report that after the fact.
//
// Each of these was PROBED by perturbing the constant it guards and confirming the build
// fails. That matters because the obvious formulation of the second one --
// `POSITIONED_HEADER_LEN == UNPOSITIONED_HEADER_LEN + POSITION_LEN` -- is a tautology:
// `POSITIONED_HEADER_LEN` is *derived from* `POSITION_LEN`, so changing the position size
// moves both sides and the assertion holds. It asserted that addition is commutative.
// Pinning the literal byte counts instead is what actually refuses.
const _: () = assert!(kind::UNPOSITIONED != kind::POSITIONED);
const _: () = assert!(VERSION_V2 != VERSION_V1);
// Two little-endian u64s.
const _: () = assert!(POSITION_LEN == 16);
// The trailing CRC-32. Guarded here because it appears in no header constant -- it sits
// after the payload -- so nothing else in this block would have caught a change to it.
const _: () = assert!(CRC_LEN == 4);
// The exact frames the module documentation draws. Changing any component moves one of
// these, which is the moment to ask whether the format version should move too.
const _: () = assert!(UNPOSITIONED_HEADER_LEN == 10);
const _: () = assert!(POSITIONED_HEADER_LEN == 26);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_len_accounts_for_every_framing_byte() {
        // Written out longhand, deliberately. Restating the sum with the same constants the
        // function uses would pass whatever they were -- it would check that addition works,
        // not that the frame is the frame the module documents.
        assert_eq!(
            record_len(kind::UNPOSITIONED, 10),
            //          magic version kind len         payload crc
            Some(4 + 1 + 1 + 4 + 10 + 4)
        );
        assert_eq!(
            record_len(kind::POSITIONED, 10),
            //          magic version kind term index len         payload crc
            Some(4 + 1 + 1 + 8 + 8 + 4 + 10 + 4)
        );
    }

    #[test]
    fn an_unknown_kind_is_refused_rather_than_guessed() {
        // The byte comes off disk. Every value that is not a kind we define must produce
        // `None` -- not a default, and not a panic, because a panic on untrusted bytes turns
        // a corrupt file into a crash loop.
        for byte in 0u8..=255 {
            if byte == kind::UNPOSITIONED || byte == kind::POSITIONED {
                assert!(record_len(byte, 0).is_some(), "kind {byte} must be known");
            } else {
                assert_eq!(record_len(byte, 0), None, "kind {byte} must be refused");
            }
        }
    }

    #[test]
    fn a_zero_position_is_representable_and_is_not_how_absence_is_written() {
        // `(0, 0)` is a legal position, so it cannot double as a sentinel. This pins that
        // the two are told apart by KIND and not by value -- the property the module docs
        // claim, stated where a later "just use (0,0)" simplification would trip over it.
        assert_ne!(kind::POSITIONED, kind::UNPOSITIONED);
        assert_eq!(
            record_len(kind::POSITIONED, 0),
            Some(POSITIONED_HEADER_LEN + CRC_LEN)
        );
        assert_ne!(
            record_len(kind::POSITIONED, 0),
            record_len(kind::UNPOSITIONED, 0),
            "the two kinds must not encode to the same length, or a reader could not \
             recover framing from length alone"
        );
    }
}
