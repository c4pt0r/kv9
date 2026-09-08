//! The seam between replicated apply and the storage engine (task #13).
//!
//! # What this replaces, and why
//!
//! Today the apply position is written as ordinary data: `crates/raft/src/state_machine.rs`
//! puts `APPLIED_INDEX_KEY` into the `Default` column family in the same batch as the
//! mutations, holding `index.0.to_be_bytes()` — **eight bytes, index only**.
//!
//! Two problems, and they are different problems.
//!
//! The first is that an index alone cannot identify where apply reached. `AppliedPosition`
//! is documented in `kv9_common::ids` as `(term, index)` precisely because *"after a
//! failover the new leader may reuse an index"*. A reclaim decision only needs ordering, so
//! index alone is sufficient **for that**; establishing that a specific entry applied needs
//! the pair. Storing only the index makes the second question unanswerable and does not
//! announce that it has.
//!
//! The second is that a key in a column family is reachable by anything that can write a
//! key. The position is not user data — it is a claim about how far replicated apply has
//! got, and the only writer entitled to make it is replicated apply. Keeping it in the CF
//! means every scan has to remember to exclude `0x00..`, which `Engine::checksum` already
//! documents as a live hazard for the scrubber.
//!
//! # What this module does NOT contain
//!
//! No `local_applied_prefix`, and no "replay succeeded up to the first violation" return
//! shape. That absence is deliberate and was ruled (Tess, task #13 acceptance 2c): a
//! non-monotonic index across *complete, CRC-correct* records is a violation of the global
//! ordering invariant, not a torn tail, and it fails the **whole open** with a typed error.
//!
//! The distinction is the whole point. A torn tail is one record that did not finish being
//! written; the records before it are individually intact *and* correctly ordered, so
//! keeping them is sound. An order violation says the records disagree with each other —
//! and an API that handed back "the prefix up to the first violation" would feed partial
//! state from a damaged file into recovery and, from there, into **irreversible reclaim**.
//! The two therefore travel different code paths and produce different errors; they must
//! not share a "replay stopped here" return.

use kv9_common::{AppliedPosition, Result};

use crate::write_batch::WriteBatch;
use crate::Engine;

/// How far an engine has **durably** applied.
///
/// Modelled on [`Durability`](crate::Durability), and for the same reason: so that a
/// volatile engine cannot be *mistaken* for a durable one. A volatile engine asked for a
/// number would answer with one — very likely `0`, or the position it happens to hold in
/// memory — and that answer reads as a legitimate watermark at exactly the moment it is
/// used to authorise truncation or object reclaim. Making the volatile case its own variant
/// means a caller computing a reclaim bound has to handle it, and cannot obtain a number to
/// misuse (DESIGN §13 principle 16).
///
/// [`Volatile`](Self::Volatile) is therefore not "unknown". It is a positive statement that
/// no durable claim exists, and it is distinct from
/// [`AppliedNothing`](Self::AppliedNothing), which is a durable claim that nothing has
/// applied yet — a fresh engine that has fsynced its emptiness. A caller may reclaim
/// nothing on either, but only one of them can ever become a number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurableAppliedPosition {
    /// This engine keeps its applied position in memory only.
    ///
    /// Nothing may be truncated or reclaimed on this engine's account, at any position.
    Volatile,
    /// Durable, and nothing has applied yet.
    AppliedNothing,
    /// Durably applied through exactly this `(term, index)`.
    ///
    /// "Through" is inclusive: the entry at this position is on stable storage along with
    /// every mutation it carried.
    AppliedThrough(AppliedPosition),
}

/// An [`Engine`] that records the replicated position at which each batch applied.
///
/// # Why the position rides with the batch
///
/// [`write_applied`](Self::write_applied) takes the batch and its position **together**, in
/// one call, because they must land atomically. Two calls — write the data, then record the
/// position — has a crash window in the middle, and both orderings lose:
///
/// * data first: a crash leaves data applied and the position behind it, so replay re-applies
///   entries the engine already has. Survivable only while every apply is idempotent, which
///   is an assumption about *callers* rather than a property of the engine.
/// * position first: a crash leaves the position ahead of the data, and the log above it may
///   be truncated. That is silent data loss, and nothing later can detect it.
///
/// # The gap this trait does not yet close
///
/// [`Engine::write`] remains available on any `ReplicatedEngine`, and it does not move the
/// position. So a replicated engine written through the wrong method silently accumulates
/// data its position does not describe. Today that is prevented by *convention* — the raft
/// apply path must use `write_applied` exclusively — and a convention is exactly the thing
/// this project keeps converting into a type.
///
/// It is recorded here rather than quietly accepted, because the honest options both cost
/// something: splitting the traits so a replicated engine does not expose `write` breaks
/// every reader that holds a `dyn Engine`, and sealing `write` behind a capability is the
/// larger change. **Left as a named question for the implementation head, not decided by
/// silence.**
pub trait ReplicatedEngine: Engine {
    /// Apply `batch` and record that it applied at `at`, atomically.
    ///
    /// No reader and no restart may observe one without the other.
    ///
    /// Rejecting a non-monotonic `at` is a **recovery-time** obligation, not a write-time
    /// one; see the module docs on why a violation fails the whole open rather than
    /// truncating to a prefix.
    fn write_applied(&self, batch: WriteBatch, at: AppliedPosition) -> Result<()>;

    /// How far this engine has durably applied.
    ///
    /// The answer a caller may build a truncation or reclaim bound from — and, for a
    /// volatile engine, the answer that refuses to be one.
    fn applied_position(&self) -> Result<DurableAppliedPosition>;
}
