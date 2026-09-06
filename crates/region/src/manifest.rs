//! The ManifestChange proposal seam (task #9; contract docs/OBJECT-STORAGE.md §7).
//!
//! The region runtime holds an engine and THIS seam — never a drain, never
//! the state machine. The call direction is fixed by contract: the engine
//! durably uploads a prepared SST FIRST, then the runtime proposes a
//! `ManifestChange` here; ordered apply installs already-durable references.
//! Apply never touches the object store (the tripwire and, later, the
//! capability-narrowed apply face guard that from the other side).
//!
//! # The in-flight slot
//!
//! One attempt per region at a time, structurally: [`ManifestSeam`] owns a
//! per-region slot that is the ONLY propose entry. A second concurrent
//! propose is a typed refusal — no queueing, no silence. The slot protects
//! the one-generation-wide decidable window (a later change overwriting
//! `last_change_id` before the previous attempt settles degrades decidable
//! outcomes into permanent superwindow Unknown) and serializes prepared-state
//! ownership; it does NOT protect exact-window correctness — the CAS algebra
//! does that on its own (Tess's correction, card rev 19).
//!
//! # Settlement
//!
//! Only settled outcomes clear the slot: `MyChangeApplied`, `AlreadyApplied`
//! (a receipt fact — the identity's transition happened earlier; NEVER
//! reported as newly accepted), `WindowRefused` (exact-window non-mine — an
//! authoritative negative from transition invariants), `ReceiptRefused` (the
//! discriminator's stale verdict for THIS attempt). `Unknown` clears nothing:
//! the slot stays occupied and the ONLY safe continuation is re-sending the
//! SAME identity ([`ManifestSeam::converge`]) — never a new change built on
//! the assumption the old one failed.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kv9_common::{Error, Result};
use kv9_raft::driver::{ApplyWaitError, ApplyWaitOutcome, ManifestNode};
use kv9_raft::{classify_reconciliation, Command, ManifestVerdict, ReconcileObservation};

/// One manifest attempt: the immutable identity `(region, expected_generation,
/// change_id)` (P3) plus the changeset and replicated watermark needed to
/// send — and, from a slot, RE-send — the SAME proposal.
#[derive(Debug, Clone)]
pub struct ManifestAttempt {
    pub region: u64,
    /// Content-derived, non-empty; rebuilt from durable prepared state on
    /// recovery.
    pub change_id: Vec<u8>,
    pub expected_generation: u64,
    /// Opaque until the engine's `PreparedSst` shape freezes.
    pub changeset: Vec<u8>,
    /// Replicated applied coordinates — never local segment/offset.
    pub watermark_term: u64,
    pub watermark_index: u64,
}

/// A SETTLED manifest outcome — every variant clears the slot; the variant
/// split is load-bearing (never fold these into a success flag):
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettledManifest {
    /// My change performed its transition NOW. The only "newly accepted".
    MyChangeApplied { generation: u64 },
    /// My identity's transition had already happened (idempotent duplicate /
    /// convergence re-send). Success for the caller's purposes, but NOT a new
    /// acceptance — receipts, metrics and logs must keep the distinction
    /// (crash-point-3 receipt half).
    AlreadyApplied { generation: u64 },
    /// Exact-window authoritative negative: another change spent my
    /// predecessor; mine did not apply and never can (its predecessor is
    /// gone). Decided by transition invariants, not by absence.
    WindowRefused { winner_change_id: Vec<u8> },
    /// The discriminator refused THIS attempt typed (stale predecessor
    /// observed at its own apply position).
    ReceiptRefused { current_generation: u64 },
}

/// The outcome of one propose/converge call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestProposalState {
    /// Slot cleared; see [`SettledManifest`].
    Settled(SettledManifest),
    /// NOT settled: the receipt did not arrive and the authoritative pair
    /// still permits my attempt to land later (pending) or has moved beyond
    /// the decidable window (superwindow). The slot STAYS OCCUPIED —
    /// fail-closed — and [`ManifestSeam::converge`] re-sends the same
    /// identity. Treating this as failure and proposing a NEW identity is
    /// the exact duplicate/loss window the contract closes.
    Unknown,
}

/// The proposal seam. Holds the node's [`ManifestNode`] face (propose +
/// receipt + P1 pair read) and the per-region in-flight slots.
pub struct ManifestSeam {
    node: Arc<dyn ManifestNode>,
    slots: Mutex<HashMap<u64, ManifestAttempt>>,
}

impl ManifestSeam {
    pub fn new(node: Arc<dyn ManifestNode>) -> ManifestSeam {
        ManifestSeam {
            node,
            slots: Mutex::new(HashMap::new()),
        }
    }

    /// Propose a manifest change for `region`. Refuses typed if the region's
    /// slot is occupied (one in-flight attempt per region — the caller with
    /// the refused proposal retries AFTER the current attempt settles, with a
    /// then-current expected generation) or if `change_id` is empty (an empty
    /// identity matches virgin state's empty `last_change_id`; the
    /// discriminator and classifier both refuse it, so the entry does too).
    ///
    /// On a receipt this settles and clears the slot; on timeout/replacement
    /// it returns [`ManifestProposalState::Unknown`] with the slot OCCUPIED —
    /// call [`ManifestSeam::converge`] to finish.
    pub fn propose_manifest_change(
        &self,
        attempt: ManifestAttempt,
        deadline: Duration,
    ) -> Result<ManifestProposalState> {
        if attempt.change_id.is_empty() {
            return Err(Error::Raft(
                "a manifest change needs a non-empty content-derived change id \
                 (an empty identity would match virgin regions' empty last_change_id)"
                    .into(),
            ));
        }
        let region = attempt.region;
        {
            let mut slots = self.slots.lock().expect("manifest slots poisoned");
            if let Some(holder) = slots.get(&region) {
                return Err(Error::Raft(format!(
                    "region {region} already has an in-flight manifest attempt \
                     (expected generation {}); one attempt per region — settle or \
                     converge it first, never race a second proposer (task #9 slot)",
                    holder.expected_generation
                )));
            }
            slots.insert(region, attempt);
        }
        self.drive_attempt(region, deadline)
    }

    /// Restart path: install a RECOVERED in-flight attempt (its identity
    /// rebuilt from the engine's durable prepared state — content-derived,
    /// recomputable) and settle it. The slot must be granted through here
    /// before any NEW proposal for the region: reconcile-first is what keeps
    /// `last_change_id` sufficient across restarts.
    pub fn resume_in_flight(
        &self,
        attempt: ManifestAttempt,
        deadline: Duration,
    ) -> Result<ManifestProposalState> {
        if attempt.change_id.is_empty() {
            return Err(Error::Raft(
                "a recovered manifest attempt needs its non-empty change id".into(),
            ));
        }
        let region = attempt.region;
        {
            let mut slots = self.slots.lock().expect("manifest slots poisoned");
            if slots.contains_key(&region) {
                return Err(Error::Raft(format!(
                    "region {region} already has an in-flight manifest attempt; \
                     resume is for a fresh seam recovering a crashed one"
                )));
            }
            slots.insert(region, attempt);
        }
        self.converge(region, deadline)
    }

    /// Converge an occupied slot: query the authoritative pair FIRST (the
    /// attempt may have settled while we were away), then — only from the
    /// pending row — re-send the SAME identity and wait for its receipt.
    /// Superwindow observations stay Unknown (slot occupied): this coordinate
    /// cannot decide them, and nothing here fabricates a decision.
    pub fn converge(&self, region: u64, deadline: Duration) -> Result<ManifestProposalState> {
        let attempt = {
            let slots = self.slots.lock().expect("manifest slots poisoned");
            match slots.get(&region) {
                Some(a) => a.clone(),
                None => {
                    return Err(Error::Raft(format!(
                        "region {region} has no in-flight manifest attempt to converge"
                    )))
                }
            }
        };
        // P1: one atomic pair read; the classifier is a pure function of it.
        let pair = self.node.manifest_pair(region)?;
        match classify_reconciliation(&pair, attempt.expected_generation, &attempt.change_id) {
            ReconcileObservation::MyChangeApplied { generation } => {
                // Applied while we were away — an earlier send landed. This is
                // NOT a new acceptance (we did not observe the applying
                // receipt); report it as the already-applied fact it is.
                self.clear(region);
                Ok(ManifestProposalState::Settled(
                    SettledManifest::AlreadyApplied { generation },
                ))
            }
            ReconcileObservation::KnownNotApplied { winner_change_id } => {
                self.clear(region);
                Ok(ManifestProposalState::Settled(
                    SettledManifest::WindowRefused { winner_change_id },
                ))
            }
            ReconcileObservation::Unknown => {
                if pair.generation != attempt.expected_generation {
                    // Superwindow (or a behind-pair precondition break):
                    // undecidable in this coordinate; the slot stays occupied
                    // rather than guessing. (The positive effect query — the
                    // add-only escape hatch — arrives with the SST reference
                    // shape; until then superwindow attempts wait here.)
                    return Ok(ManifestProposalState::Unknown);
                }
                // Pending: my predecessor is unspent. Re-send the SAME
                // identity — CAS + change-id idempotency make this harmless
                // in every interleaving — and wait for its receipt.
                self.drive_attempt(region, deadline)
            }
        }
    }

    /// Propose the slot's attempt and settle on its receipt. The slot is
    /// already held by the caller path.
    fn drive_attempt(&self, region: u64, deadline: Duration) -> Result<ManifestProposalState> {
        let attempt = {
            let slots = self.slots.lock().expect("manifest slots poisoned");
            slots.get(&region).expect("caller holds the slot").clone()
        };
        let cmd = Command::ManifestChange {
            region: attempt.region,
            change_id: attempt.change_id.clone(),
            expected_generation: attempt.expected_generation,
            changeset: attempt.changeset.clone(),
            watermark_term: attempt.watermark_term,
            watermark_index: attempt.watermark_index,
        };
        let at = match self.node.propose_command(&cmd) {
            Ok(at) => at,
            Err(e) => {
                // Nothing entered the log from THIS send... is not knowable
                // in general (a propose error can race an accepted entry), so
                // the slot stays occupied and the error surfaces; converge
                // retries. Only a pre-propose refusal by the entry itself
                // (empty id, occupied slot) is a settled negative, and those
                // never reach here.
                return Err(e);
            }
        };
        match self.node.wait_manifest(at, deadline) {
            Ok(ApplyWaitOutcome::Manifest { verdict, .. }) => {
                let settled = match verdict {
                    ManifestVerdict::Applied { generation, .. } => {
                        SettledManifest::MyChangeApplied { generation }
                    }
                    ManifestVerdict::AlreadyApplied { generation, .. } => {
                        SettledManifest::AlreadyApplied { generation }
                    }
                    ManifestVerdict::Stale {
                        current_generation, ..
                    } => SettledManifest::ReceiptRefused { current_generation },
                };
                self.clear(region);
                Ok(ManifestProposalState::Settled(settled))
            }
            Ok(ApplyWaitOutcome::Applied(at)) => Err(Error::Raft(format!(
                "manifest proposal received a plain Applied receipt at term {} index {} — \
                 the verdict was lost; receipt correlation broke (fail closed, slot held)",
                at.term, at.index
            ))),
            Ok(ApplyWaitOutcome::FenceRejected { at, .. }) => Err(Error::Raft(format!(
                "manifest proposal received a fence verdict at term {} index {} — \
                 receipt correlation broke (fail closed, slot held)",
                at.term, at.index
            ))),
            // Replaced says OUR (term,index) claim was consumed by another
            // entry — it says nothing about whether an earlier/later send of
            // this identity lands. Unknown; converge re-queries and re-sends.
            Ok(ApplyWaitOutcome::Replaced) => Ok(ManifestProposalState::Unknown),
            Err(ApplyWaitError::Unconfirmed { .. }) => Ok(ManifestProposalState::Unknown),
            Err(ApplyWaitError::Failed(e)) => Err(e),
        }
    }

    fn clear(&self, region: u64) {
        self.slots
            .lock()
            .expect("manifest slots poisoned")
            .remove(&region);
    }

    /// Whether a region currently holds an in-flight attempt (observability;
    /// never a substitute for the typed refusal at the propose entry).
    pub fn in_flight(&self, region: u64) -> bool {
        self.slots
            .lock()
            .expect("manifest slots poisoned")
            .contains_key(&region)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kv9_common::{NodeId, RegionId};
    use kv9_raft::driver::NodeDriver;
    use kv9_raft::transport::{InProcHub, RaftTransport};
    use kv9_raft::{MemStateMachine, RaftGroup, RaftPeer, Role};

    const TICK: Duration = Duration::from_millis(2);
    const WAIT: Duration = Duration::from_secs(5);
    const SHORT: Duration = Duration::from_millis(120);

    /// One elected single-node driver with its pump thread running, plus the
    /// seam over its `ManifestNode` face.
    fn seam_over_driver() -> (Arc<NodeDriver>, ManifestSeam) {
        let hub = InProcHub::new();
        let peer =
            std::sync::Arc::new(RaftPeer::new(NodeId(1), RegionId(1), &[NodeId(1)]).unwrap());
        let endpoint = hub.endpoint(NodeId(1));
        let driver = NodeDriver::new(
            peer,
            Arc::new(endpoint) as Arc<dyn RaftTransport>,
            MemStateMachine::new(),
        )
        .expect("drain token minted once per peer");
        driver.peer().campaign().unwrap();
        for _ in 0..100 {
            driver.tick_and_step().unwrap();
            if driver.status().role == Role::Leader {
                break;
            }
        }
        assert_eq!(driver.status().role, Role::Leader);
        driver.spawn(TICK);
        let seam = ManifestSeam::new(driver.clone() as Arc<dyn ManifestNode>);
        (driver, seam)
    }

    fn attempt(region: u64, id: &[u8], expected: u64, widx: u64) -> ManifestAttempt {
        ManifestAttempt {
            region,
            change_id: id.to_vec(),
            expected_generation: expected,
            changeset: format!("refs-{}", String::from_utf8_lossy(id)).into_bytes(),
            watermark_term: 1,
            watermark_index: widx,
        }
    }

    fn applied(state: &ManifestProposalState) -> u64 {
        match state {
            ManifestProposalState::Settled(SettledManifest::MyChangeApplied { generation }) => {
                *generation
            }
            other => panic!("expected MyChangeApplied, got {other:?}"),
        }
    }

    /// The happy path: durable-upload-then-propose settles as the ONLY
    /// newly-accepted outcome and clears the slot.
    #[test]
    fn a_proposed_manifest_change_settles_applied_and_clears_the_slot() {
        let (_driver, seam) = seam_over_driver();
        let state = seam
            .propose_manifest_change(attempt(5, b"A", 0, 1), WAIT)
            .unwrap();
        assert_eq!(applied(&state), 1);
        assert!(!seam.in_flight(5));
    }

    /// Slot: while one attempt is in flight (apply frozen), a second propose
    /// for the same region refuses typed — no queueing, no second entry in
    /// raft from this seam. A DIFFERENT region is untouched (per-region).
    #[test]
    fn a_concurrent_second_proposer_is_refused_typed() {
        let (driver, seam) = seam_over_driver();
        driver.pause_apply(true);
        let first = seam
            .propose_manifest_change(attempt(5, b"A", 0, 1), SHORT)
            .unwrap();
        assert_eq!(first, ManifestProposalState::Unknown);
        assert!(seam.in_flight(5));
        let second = seam.propose_manifest_change(attempt(5, b"B", 0, 2), SHORT);
        match second {
            Err(kv9_common::Error::Raft(msg)) => {
                assert!(msg.contains("in-flight"), "refusal names the slot: {msg}")
            }
            other => panic!("expected typed slot refusal, got {other:?}"),
        }
        // Convergence after the freeze lifts: the SAME identity re-sends; the
        // frozen first entry applies first, the re-send lands AlreadyApplied —
        // reported as such, never as newly accepted.
        driver.pause_apply(false);
        let state = seam.converge(5, WAIT).unwrap();
        assert_eq!(
            state,
            ManifestProposalState::Settled(SettledManifest::AlreadyApplied { generation: 1 })
        );
        assert!(!seam.in_flight(5));
    }

    /// Re-proposing a settled identity through a FREE slot flows through the
    /// discriminator's AlreadyApplied row — distinct from MyChangeApplied.
    #[test]
    fn a_resent_settled_identity_reports_already_applied() {
        let (_driver, seam) = seam_over_driver();
        let first = seam
            .propose_manifest_change(attempt(5, b"A", 0, 1), WAIT)
            .unwrap();
        assert_eq!(applied(&first), 1);
        let resend = seam
            .propose_manifest_change(attempt(5, b"A", 0, 2), WAIT)
            .unwrap();
        assert_eq!(
            resend,
            ManifestProposalState::Settled(SettledManifest::AlreadyApplied { generation: 1 })
        );
    }

    /// A spent predecessor refuses via the discriminator's receipt — typed,
    /// slot cleared, generation named.
    #[test]
    fn a_stale_predecessor_settles_receipt_refused() {
        let (_driver, seam) = seam_over_driver();
        seam.propose_manifest_change(attempt(5, b"A", 0, 1), WAIT)
            .unwrap();
        let stale = seam
            .propose_manifest_change(attempt(5, b"B", 0, 2), WAIT)
            .unwrap();
        assert_eq!(
            stale,
            ManifestProposalState::Settled(SettledManifest::ReceiptRefused {
                current_generation: 1
            })
        );
        assert!(!seam.in_flight(5));
    }

    /// Restart path, negative half: a recovered attempt whose predecessor was
    /// spent by ANOTHER change settles WindowRefused from the exact-window
    /// query row — an authoritative negative, no re-send needed.
    #[test]
    fn a_recovered_attempt_beaten_in_its_window_settles_window_refused() {
        let (driver, seam) = seam_over_driver();
        seam.propose_manifest_change(attempt(6, b"A", 0, 1), WAIT)
            .unwrap();
        // A fresh seam (the restart): its recovered attempt C also expected
        // generation 0, which A spent.
        let seam2 = ManifestSeam::new(driver.clone() as Arc<dyn ManifestNode>);
        let state = seam2
            .resume_in_flight(attempt(6, b"C", 0, 2), WAIT)
            .unwrap();
        assert_eq!(
            state,
            ManifestProposalState::Settled(SettledManifest::WindowRefused {
                winner_change_id: b"A".to_vec()
            })
        );
        assert!(!seam2.in_flight(6));
    }

    /// Restart path, positive half: a recovered attempt that had ALREADY
    /// applied settles from the query row as AlreadyApplied — the receipt was
    /// lost with the crash, and this is not a new acceptance.
    #[test]
    fn a_recovered_applied_attempt_settles_already_applied() {
        let (driver, seam) = seam_over_driver();
        seam.propose_manifest_change(attempt(7, b"A", 0, 1), WAIT)
            .unwrap();
        let seam2 = ManifestSeam::new(driver.clone() as Arc<dyn ManifestNode>);
        let state = seam2
            .resume_in_flight(attempt(7, b"A", 0, 1), WAIT)
            .unwrap();
        assert_eq!(
            state,
            ManifestProposalState::Settled(SettledManifest::AlreadyApplied { generation: 1 })
        );
    }

    /// The entry guards: empty identity and converge-without-a-slot are typed
    /// refusals, not silent nonsense.
    #[test]
    fn entry_guards_refuse_typed() {
        let (_driver, seam) = seam_over_driver();
        let mut empty = attempt(5, b"X", 0, 1);
        empty.change_id = Vec::new();
        assert!(seam.propose_manifest_change(empty, WAIT).is_err());
        assert!(seam.converge(5, WAIT).is_err());
    }
}
