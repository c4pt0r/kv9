//! The seam between replicated apply and the storage engine (task #13).
//!
//! # Durable positioned apply
//!
//! `WalEngine` records the batch and exact `(term, index)` together in a WAL-v2
//! frame. One CRC covers version, kind, position, length and mutations. Recovery
//! publishes data and position together and refuses non-advancing complete records.
//! The apply position is engine state, outside user column families and scans.
//! Unpositioned writes remain readable for tools and v1 compatibility; they never
//! advance replicated progress or authorize log reclamation.
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
/// # Apply capability boundary
///
/// [`Engine::write`] remains available on the full engine for local tools and
/// legacy compatibility. Production Raft apply holds the narrower `ApplyStore`
/// trait from kv9-raft, whose only mutation is `write_applied`. Compile-fail
/// probes guard that boundary; unpositioned histories remain non-reclaimable.
pub trait ReplicatedEngine: Engine {
    /// Apply `batch` and record that it applied at `at`, atomically.
    ///
    /// No reader and no restart may observe one without the other.
    ///
    /// # Ordering
    ///
    /// An `at` whose index does not advance past the last applied one is **refused**, and
    /// refused *before* any mutation is made, so a rejected call leaves no partial batch.
    /// This generic engine contract compares **index only** (task #13, 2c).
    /// A gap in indices is legal; repeating or going backwards is not. Terms in
    /// an actual Raft log are nondecreasing, but validating Raft history belongs
    /// to the runtime, which checks the recovered pair against committed entries.
    ///
    /// This is a write-time check *in addition to* the recovery-time obligation, not
    /// instead of it. They catch different things and neither implies the other: this one
    /// says a live caller fed a position out of order, which is a bug in the caller and
    /// surfaces immediately as apply poison; the recovery-time one says the records in a
    /// *file* disagree with each other, which is discovered only on replay and fails the
    /// whole open (see the module docs on why it must not yield a usable prefix).
    fn write_applied(&self, batch: WriteBatch, at: AppliedPosition) -> Result<()>;

    /// How far this engine has durably applied.
    ///
    /// The answer a caller may build a truncation or reclaim bound from — and, for a
    /// volatile engine, the answer that refuses to be one.
    fn applied_position(&self) -> Result<DurableAppliedPosition>;
}
