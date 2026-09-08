//! Raft state-machine adapter (Phase-1 spine; ROADMAP Phase 1).
//!
//! The raft group replicates a log; the **state machine** deterministically applies each
//! committed entry. For the metadata plane the state machine is a KV backed by the
//! mocked [`kv9_engine::MemEngine`] (ROADMAP: "the raft state machine is the skeleton's
//! `MemEngine` (mock)"). The `meta` catalog engine ([`kv9_meta::MetaStore`]) runs *on
//! top of* this KV.
//!
//! Phase-1 path: `propose(cmd) → committed → apply → read` over the existing
//! [`crate::RaftGroup`] trait. Phase 1 provides both the immediate single-node group and
//! the deterministic raft-rs [`crate::RaftPeer`] adapter behind that same pull model.

use std::sync::Arc;

use kv9_engine::{ColumnFamily, Engine, MemEngine};

use kv9_common::Result;

use crate::command::Command;
use crate::command::ManifestChangePayload;
use crate::{CommittedEntry, LogIndex};

/// The apply-side storage capability (task #9, the capability-narrowing half
/// of the apply-never-touches-the-object-store invariant): EXACTLY what ordered
/// apply needs — an atomic batch write and a point read of log-established
/// state. Deliberately NOT [`kv9_engine::Engine`]: apply code is generic
/// over THIS bound, so even when the concrete engine grows richer surfaces
/// (snapshots, checksums, or one day an object-store accessor), the apply
/// face does not grow with it — reaching anything beyond these two methods
/// is a compile error inside this crate, not a review catch.
///
/// The blanket impl keeps every real engine usable without call-site
/// changes; the narrowing is in the BOUND, not the type.
///
/// # Resident guards — measured, not assumed, in both directions
///
/// Apply's face has no read-view, no scan, no snapshot (this probe fires if
/// the capability ever widens):
///
/// ```compile_fail,E0599
/// fn probe<A: kv9_raft::ApplyStore>(a: &A) {
///     let _ = a.snapshot();
/// }
/// ```
///
/// Green twin — the identical call against the full engine trait compiles,
/// pinning the red probe to "capability absent from ApplyStore":
///
/// ```
/// fn probe<E: kv9_engine::Engine>(e: &E) {
///     let _ = e.snapshot();
/// }
/// ```
pub trait ApplyStore: Send + Sync {
    fn get(&self, cf: ColumnFamily, key: &[u8]) -> Result<Option<Vec<u8>>>;
    fn write(&self, batch: kv9_engine::WriteBatch) -> Result<()>;
}

impl<E: Engine> ApplyStore for E {
    fn get(&self, cf: ColumnFamily, key: &[u8]) -> Result<Option<Vec<u8>>> {
        Engine::get(self, cf, key)
    }

    fn write(&self, batch: kv9_engine::WriteBatch) -> Result<()> {
        Engine::write(self, batch)
    }
}

/// Engine key holding the durably applied watermark. The `0x00` first byte
/// cannot collide with any `mode_byte`-encoded physical key (`'t'`/`'r'`/`'s'`),
/// so catalog scans never see it.
///
/// # Why this stores INDEX ONLY, while `AppliedPosition` docs say index
/// alone is never sufficient (asked by Ren before building the WAL reclaim
/// predicate on it — the two statements answer DIFFERENT questions)
///
/// `ids.rs`'s warning is about PROPOSAL CORRELATION: a proposal's claimed
/// index can be consumed by another leader's entry, because the claim is
/// made before commit — deciding "is the entry at this position MINE"
/// requires term+index, always.
///
/// This watermark never asks that question. It records how far the
/// COMMITTED prefix has been applied, and raft's Log Matching + Leader
/// Completeness make that prefix immutable: an entry that advanced this
/// watermark was applied, hence committed, hence present at that index in
/// every future log of every leader. Index reuse only ever happens to
/// UNCOMMITTED suffixes — which were never applied and never advanced this
/// value. So for prefix-coverage comparisons (restart replay skip here;
/// the WAL segment reclaim predicate `max_applied_position <= watermark`,
/// OBJECT-STORAGE §6.3) the index is a complete coordinate; the term
/// component exists for receipts, not for coverage.
///
/// If a future change ever lets this watermark advance on an UNCOMMITTED
/// entry, that change — not the key format — is the bug, and it breaks the
/// argument above; the discriminator/driver apply loops only ever feed
/// committed entries here.
pub const APPLIED_INDEX_KEY: &[u8] = b"\x00kv9\x00applied_index";

/// Engine key holding one region's authoritative manifest
/// `(generation, last_change_id)` pair (task #9). ONE key on purpose:
/// precondition P1 (atomic read) is structural at rest — a single get returns
/// both fields from one applied snapshot, so a torn pair is unrepresentable
/// in storage. The key prefix is module-PRIVATE and the only writer is the
/// `ManifestChange` arm of `apply_command` (precondition P4: the pair is
/// written only by one successful CAS, both fields together); readers go
/// through [`MemStateMachine::manifest_pair`].
fn manifest_pair_key(region: u64) -> Vec<u8> {
    let mut k = b"\x00kv9\x00manifest_pair\x00".to_vec();
    k.extend_from_slice(&region.to_be_bytes());
    k
}

/// Engine key holding the changeset installed at one region generation
/// (add-only: a new generation is a new key; nothing rewrites an old one).
fn manifest_gen_key(region: u64, generation: u64) -> Vec<u8> {
    let mut k = b"\x00kv9\x00manifest_gen\x00".to_vec();
    k.extend_from_slice(&region.to_be_bytes());
    k.push(0);
    k.extend_from_slice(&generation.to_be_bytes());
    k
}

/// One region's authoritative manifest pair, decoded from its single key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestPair {
    /// Current generation: number of successful manifest CASes applied.
    pub generation: u64,
    /// The change that performed the g-1 → g transition (empty at generation
    /// 0 — no change has ever applied). Written ONLY together with
    /// `generation`, in the same value of the same key.
    pub last_change_id: Vec<u8>,
}

impl ManifestPair {
    fn encode(&self) -> Vec<u8> {
        let mut v = self.generation.to_be_bytes().to_vec();
        v.extend_from_slice(&self.last_change_id);
        v
    }

    fn decode(bytes: &[u8]) -> kv9_common::Result<ManifestPair> {
        if bytes.len() < 8 {
            return Err(kv9_common::Error::Raft(
                "manifest pair value shorter than its generation field".into(),
            ));
        }
        let pair = ManifestPair {
            generation: u64::from_be_bytes(bytes[..8].try_into().expect("8 bytes")),
            last_change_id: bytes[8..].to_vec(),
        };
        // Impossible states are decode errors, not data: every applied
        // transition writes a non-empty id with generation >= 1, and a
        // virgin pair is only ever synthesized (never stored) as (0, empty).
        // Accepting either asymmetry would let a corrupt value quietly enter
        // the CAS algebra.
        if pair.generation > 0 && pair.last_change_id.is_empty() {
            return Err(kv9_common::Error::Raft(
                "manifest pair with advanced generation but empty change id: \
                 no CAS writes that state — corrupt or foreign value"
                    .into(),
            ));
        }
        if pair.generation == 0 && !pair.last_change_id.is_empty() {
            return Err(kv9_common::Error::Raft(
                "manifest pair at generation 0 with a change id: no CAS \
                 writes that state — corrupt or foreign value"
                    .into(),
            ));
        }
        Ok(pair)
    }
}

/// What a reconciliation QUERY of the authoritative pair can conclude about
/// one attempt `(expected_generation, change_id)` (task #9, the four-row
/// table). Pure function of ONE atomically-read pair plus the attempt's own
/// immutable identity — decidability rests on preconditions P1–P4, each
/// guarded separately; this function cannot check them and does not try.
///
/// Only ONE row is a positive success and only ONE is a negative proof:
/// - `MyChangeApplied`: exact window, mine — the unique `expected →
///   expected+1` transition was mine.
/// - `KnownNotApplied`: exact window, NOT mine — under strict CAS the unique
///   transition from `expected` belongs to another change; had mine applied,
///   `last_change_id` would be mine; if mine has not arrived it will
///   stale-refuse forever (its predecessor is spent). Authoritative negative
///   BY TRANSITION INVARIANTS, not by absence (the refined general rule).
/// - `Unknown` covers BOTH remaining rows and is not one state: pending
///   (`current == expected`; the change may be committed-unapplied or commit
///   after this query) and superwindow (`current > expected+1`; history
///   lost — mine-applied-then-superseded and never-arrived are
///   indistinguishable in this coordinate). Neither clears a slot; neither
///   licenses a NEW identity. Safe convergence: re-send the SAME identity
///   and wait for its receipt.
///
/// A pair BEHIND the attempt (`current < expected`) is also `Unknown`:
/// under P4 it cannot happen, so observing it means an expectation was
/// fabricated or a precondition broke — never grounds for a settled verdict
/// (fail closed, do not guess which).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconcileObservation {
    MyChangeApplied {
        /// The generation my change produced (== expected_generation + 1).
        generation: u64,
    },
    KnownNotApplied {
        /// The change that won my predecessor (diagnostic, not authority).
        winner_change_id: Vec<u8>,
    },
    Unknown,
}

/// Classify one atomically-read pair against one attempt's identity.
/// See [`ReconcileObservation`] for the table; the pair must come from
/// [`MemStateMachine::manifest_pair`] (one key, one get — P1).
pub fn classify_reconciliation(
    pair: &ManifestPair,
    expected_generation: u64,
    change_id: &[u8],
) -> ReconcileObservation {
    if change_id.is_empty() {
        // An empty identity matches nothing: generation-0 pairs hold an empty
        // last_change_id, and "empty == empty" would read virgin state as
        // MyChangeApplied. The propose entry refuses empty ids; this guard is
        // the query-side twin.
        return ReconcileObservation::Unknown;
    }
    let Some(exact_window) = expected_generation.checked_add(1) else {
        // u64::MAX predecessor cannot have a successor generation: nothing
        // can ever settle this attempt from the query; typed Unknown, never
        // a wrap or a debug panic (review probe: this panicked).
        return ReconcileObservation::Unknown;
    };
    if pair.generation == exact_window {
        if pair.last_change_id == change_id {
            ReconcileObservation::MyChangeApplied {
                generation: pair.generation,
            }
        } else {
            ReconcileObservation::KnownNotApplied {
                winner_change_id: pair.last_change_id.clone(),
            }
        }
    } else {
        ReconcileObservation::Unknown
    }
}

/// The apply-side verdict of one [`crate::Command::ManifestChange`] (task #9).
///
/// Separate variants, never a flag: `Applied` is the ONLY variant that may be
/// reported upward as "this proposal succeeded". `AlreadyApplied` is the
/// idempotent-duplicate outcome — a WAL-replayed or re-sent change whose
/// identity already performed its transition must never be reported as newly
/// accepted (crash-point-3 receipt half). `Stale` is the CAS refusal. All
/// three advance the applied watermark; none is an apply error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestVerdict {
    /// This change performed the `expected → expected+1` transition.
    Applied {
        region: kv9_common::RegionId,
        /// The generation AFTER this apply (== expected_generation + 1).
        generation: u64,
    },
    /// This change's identity already performed its transition earlier —
    /// nothing written now, and NOT a new acceptance.
    AlreadyApplied {
        region: kv9_common::RegionId,
        generation: u64,
    },
    /// The CAS predecessor did not match; nothing written.
    Stale {
        region: kv9_common::RegionId,
        current_generation: u64,
    },
    /// The change is malformed or impossible AT THIS APPLY POSITION — a
    /// deterministic, typed, logical refusal (watermark still advances;
    /// every replica computes the same verdict from the same entry).
    /// Deliberately NOT an apply error: poisoning every replica over one
    /// bad proposal would turn an input problem into an outage.
    Invalid {
        region: kv9_common::RegionId,
        reason: ManifestInvalidReason,
    },
}

/// Why a manifest change was refused as invalid (Copy so it rides the ring).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestInvalidReason {
    /// Empty change ids are unidentifiable: they would compare equal to a
    /// virgin pair's empty last_change_id. Checked BEFORE any CAS arm — on a
    /// virgin region an empty id at expected=0 must not reach Applied
    /// (review probe: it did).
    EmptyChangeId,
    /// The claimed replicated watermark is at or beyond this manifest
    /// entry's own log position: a manifest cannot vouch for WAL it could
    /// not yet have absorbed, and persisting the claim would later authorize
    /// recycling log the manifest does not cover (review probe: index-1
    /// entry claiming watermark 100 applied and persisted).
    WatermarkBeyondEntry,
    /// The generation counter cannot advance (u64 exhausted) — typed and
    /// deterministic, never a wrap or a debug panic.
    GenerationExhausted,
}

/// What applying one committed entry MEANT — one EXCLUSIVE outcome
/// (review round: this was two independent `Option`s declared mutually
/// exclusive only in a comment, and the sole consumer wildcarded the
/// impossible `(Some, Some)` state — a broken exclusivity elsewhere would
/// have returned a fence rejection AS a manifest verdict. The enum makes
/// the conflicting state unrepresentable instead of trusted away).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyOutcome {
    /// Applied normally (data written, or a no-effect command).
    Plain,
    /// The entry was a [`Command::Fenced`] whose fence FAILED adjudication:
    /// logically rejected — no data written — but the applied watermark
    /// advanced like any applied entry. Carries the REJECTED REGION so the
    /// receipt path surfaces a typed `StaleEpoch {{ region }}` without
    /// re-deriving anything from the original command (apply-time facts
    /// only; review contract).
    FenceRejected(kv9_common::RegionId),
    /// The entry was a [`crate::Command::ManifestChange`]: the
    /// discriminator's verdict rides the receipt path (apply-time facts
    /// only; no second apply channel).
    Manifest(ManifestVerdict),
}

/// The outcome of applying one committed entry to the state machine (ROADMAP Phase 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyResult {
    /// The log index that was applied (monotonic; the applied watermark advances to it).
    pub applied_index: LogIndex,
    /// Bytes returned to the proposer, if the command produces a read-back value
    /// (e.g. a conf-change ack). `None` for plain writes.
    pub response: Option<Vec<u8>>,
    /// What applying this entry meant — exclusive by type.
    pub outcome: ApplyOutcome,
}

impl ApplyResult {
    pub fn write_ok(index: LogIndex) -> Self {
        ApplyResult {
            applied_index: index,
            response: None,
            outcome: ApplyOutcome::Plain,
        }
    }

    /// The entry was a manifest change; its discriminator verdict rides the
    /// receipt (watermark advanced in all three cases).
    pub fn manifest(index: LogIndex, verdict: ManifestVerdict) -> Self {
        ApplyResult {
            applied_index: index,
            response: None,
            outcome: ApplyOutcome::Manifest(verdict),
        }
    }

    /// The entry's fence failed adjudication: watermark advanced, nothing
    /// written, and the rejected region rides the receipt.
    pub fn fence_rejected(index: LogIndex, region: kv9_common::RegionId) -> Self {
        ApplyResult {
            applied_index: index,
            response: None,
            outcome: ApplyOutcome::FenceRejected(region),
        }
    }
}

/// Adjudicates a [`crate::RegionFence`] against the region state at the calling
/// entry's ordered-apply position (task #48 layer 2).
///
/// Implementations live ABOVE this crate (the server provides one backed by the
/// region catalog, comparing with `RegionEpoch::is_fresh_as` — the same predicate
/// the router's `check_epoch` uses, so propose-side and apply-side verdicts cannot
/// drift). The verdict must be a pure function of state established by the same
/// log: every replica applies the same entries in the same order, reads the same
/// epoch, and reaches the same verdict.
pub trait FenceAdjudicator: Send + Sync {
    /// `Ok(true)` — the proposer's expected epoch is still fresh, the fenced ops
    /// may apply. `Ok(false)` — authoritatively stale: the entry is logically
    /// rejected (watermark still advances). `Err` — the authoritative epoch
    /// could NOT be read (engine/decode/catalog failure): this is a third
    /// state, not a verdict, and it propagates as an apply error (driver
    /// poisons; data and watermark untouched). Crushing it into `false` would
    /// dress a node-local read failure as a deterministic rejection and
    /// consume the log position; into `true`, an unfenced write — either way
    /// replicas whose reads fail diverge from replicas whose reads succeed
    /// (review round).
    fn is_fresh(&self, fence: &crate::RegionFence) -> Result<bool>;
}

/// A deterministic raft state machine (ROADMAP Phase 1).
///
/// Every replica applies the *same* committed entries in the *same* order, reaching the
/// same state. `apply` must be deterministic and side-effect-free beyond its own state.
pub trait StateMachine: Send + Sync {
    /// Apply one committed entry, advancing the applied watermark (ROADMAP Phase 1).
    fn apply(&mut self, entry: &CommittedEntry) -> Result<ApplyResult>;

    /// The highest log index applied so far.
    fn applied_index(&self) -> LogIndex;
}

/// The Phase-1 metadata state machine: a KV over the mocked [`MemEngine`].
///
/// The catalog engine writes/read through the same engine, so a committed
/// `Command::CatalogTxn` lands atomically here and is then visible to
/// [`kv9_meta::MetaStore`] reads. Swapping `MemEngine` for the real disaggregated engine
/// is Phase-2 and does not change this type's shape (it is generic over [`Engine`]).
pub struct MemStateMachine<E: ApplyStore = MemEngine> {
    engine: Arc<E>,
    applied: LogIndex,
    /// Adjudicates [`Command::Fenced`] entries. `None` — the default — makes a
    /// fenced entry a typed APPLY ERROR (driver poisons): adjudicator presence
    /// is node-local configuration, not log state, so a node without one must
    /// refuse rather than pick a verdict that differently-configured replicas
    /// would not share. The server injects the catalog-backed adjudicator at
    /// startup, before any fenced proposal is possible.
    adjudicator: Option<Arc<dyn FenceAdjudicator>>,
}

impl MemStateMachine<MemEngine> {
    /// A fresh state machine over a new in-memory engine (ROADMAP Phase 1 first task).
    pub fn new() -> Self {
        MemStateMachine::with_engine(Arc::new(MemEngine::new()))
            .expect("a fresh MemEngine has no watermark to corrupt")
    }
}

impl Default for MemStateMachine<MemEngine> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E: ApplyStore> MemStateMachine<E> {
    /// Build a state machine over an existing shared engine (so the `meta` catalog and
    /// the raft apply loop observe the *same* KV).
    ///
    /// Recovers the durably applied watermark from the engine: data and
    /// watermark are written in ONE atomic batch (see [`Self::apply_command`]),
    /// so on a durable engine they are physically inseparable — a restarted
    /// node resumes from where its data actually is, instead of reporting 0
    /// over a full store (the "durable data, volatile watermark" mismatch).
    ///
    /// Construction is fallible: an engine read error or a malformed watermark
    /// value REFUSES to open — silently coercing either to "watermark 0" would
    /// re-apply the whole log over unknown state (guessing is worse than
    /// stopping). A missing key is genuinely fresh and starts at 0.
    pub fn with_engine(engine: Arc<E>) -> Result<Self> {
        let applied = match engine.get(ColumnFamily::Default, APPLIED_INDEX_KEY)? {
            None => 0,
            Some(v) => {
                let bytes: [u8; 8] = v.try_into().map_err(|v: Vec<u8>| {
                    kv9_common::Error::Engine(format!(
                        "corrupt applied watermark: {} bytes (want 8)",
                        v.len()
                    ))
                })?;
                u64::from_be_bytes(bytes)
            }
        };
        Ok(MemStateMachine {
            engine,
            applied: LogIndex(applied),
            adjudicator: None,
        })
    }

    /// Install the fence adjudicator (server startup; see [`FenceAdjudicator`]).
    /// Until this is called a committed [`Command::Fenced`] entry is a typed
    /// apply error, not a verdict.
    pub fn set_fence_adjudicator(&mut self, adjudicator: Arc<dyn FenceAdjudicator>) {
        self.adjudicator = Some(adjudicator);
    }

    /// The backing engine — the KV the `meta` catalog reads/writes (ROADMAP Phase 1).
    pub fn engine(&self) -> &Arc<E> {
        &self.engine
    }

    /// Apply an already-decoded command at `index` (used by the propose→apply path and
    /// by tests that construct commands directly, avoiding the entry codec stub).
    ///
    /// Idempotent under redelivery: an entry at or below the applied watermark
    /// is skipped — after a restart, log replay fast-forwards past everything
    /// the engine already holds, and a future NON-idempotent command stays
    /// correct instead of silently double-applying. The watermark rides in the
    /// SAME atomic batch as the data (cross-CF batch atomicity is the engine's
    /// contract), so the two cannot diverge on disk.
    pub fn apply_command(&mut self, index: LogIndex, cmd: &Command) -> Result<ApplyResult> {
        if index <= self.applied {
            return Ok(ApplyResult::write_ok(index));
        }
        // EVERY applied entry advances the durable watermark — including
        // commands with no data mutations (Noop/ConfChange). Advancing those
        // only in memory would regress the watermark on restart and re-deliver
        // entries the group considers applied; correctness would again rest on
        // the all-commands-are-idempotent coincidence this change removes.
        //
        // A Fenced entry is adjudicated HERE, inside ordered apply, because the
        // verdict must be a pure function of log-established state (task #48
        // layer 2). That same rule forbids a local default: whether an
        // adjudicator is INSTALLED is node-local configuration, not log state,
        // so a node without one must refuse the entry as an apply error — the
        // driver poisons, exactly like decoding an unknown entry version —
        // rather than "reject", which would let differently-configured replicas
        // apply the same committed entry to different states.
        //
        // A true stale-fence rejection IS a log-determined verdict, and it is a
        // logical outcome, not an apply failure: the watermark rides the same
        // atomic batch as an accepted entry, so a rejected entry advances it
        // identically — treating rejection as an error would stall the
        // watermark on every replica.
        // A ManifestChange runs its generation CAS HERE, inside ordered apply
        // (task #9): the verdict is a pure function of log-established state
        // (the pair itself is only ever written by earlier entries of this
        // same log), and all three outcomes are logical verdicts that advance
        // the watermark — the `Fenced` precedent, not a second apply channel.
        if let Command::ManifestChange(p) = cmd {
            let ManifestChangePayload {
                region,
                change_id,
                expected_generation,
                changeset,
                watermark_term,
                watermark_index,
            } = p;
            let pair_key = manifest_pair_key(*region);
            let current = match self.engine.get(ColumnFamily::Default, &pair_key)? {
                Some(bytes) => ManifestPair::decode(&bytes)?,
                None => ManifestPair {
                    generation: 0,
                    last_change_id: Vec::new(),
                },
            };
            let region_id = kv9_common::RegionId(*region);
            let mut batch = kv9_engine::WriteBatch::new();
            // Validity BEFORE any CAS arm (review round 1, Tess): the empty
            // identity previously reached the Applied arm on a virgin
            // region, and a self-reported watermark at/beyond this entry's
            // own position previously persisted.
            let verdict = if change_id.is_empty() {
                ManifestVerdict::Invalid {
                    region: region_id,
                    reason: ManifestInvalidReason::EmptyChangeId,
                }
            } else if *watermark_index >= index.0 {
                ManifestVerdict::Invalid {
                    region: region_id,
                    reason: ManifestInvalidReason::WatermarkBeyondEntry,
                }
            } else if *expected_generation == current.generation {
                match current.generation.checked_add(1) {
                    None => ManifestVerdict::Invalid {
                        region: region_id,
                        reason: ManifestInvalidReason::GenerationExhausted,
                    },
                    Some(next_generation) => {
                        // The unique g → g+1 transition: both pair fields
                        // written together in ONE value of ONE key (P4 write
                        // arm; P1: a torn pair is unrepresentable at rest),
                        // in the SAME atomic batch as the changeset install
                        // and the applied watermark.
                        let next = ManifestPair {
                            generation: next_generation,
                            last_change_id: change_id.clone(),
                        };
                        batch.put(ColumnFamily::Default, pair_key, next.encode());
                        let mut installed = watermark_term.to_be_bytes().to_vec();
                        installed.extend_from_slice(&watermark_index.to_be_bytes());
                        installed.extend_from_slice(changeset);
                        batch.put(
                            ColumnFamily::Default,
                            manifest_gen_key(*region, next_generation),
                            installed,
                        );
                        ManifestVerdict::Applied {
                            region: region_id,
                            generation: next_generation,
                        }
                    }
                }
            } else if expected_generation
                .checked_add(1)
                .is_some_and(|g| g == current.generation)
                && *change_id == current.last_change_id
            {
                // Idempotent duplicate of the FULL identity: the same
                // predecessor window AND the same change id (P3 identity is
                // the pair, not the id alone — review probe: (expected=99,
                // id=A) after A applied at 0 was reported AlreadyApplied).
                // Nothing written; MUST NOT be reported as newly accepted
                // (crash-point-3 receipt half).
                ManifestVerdict::AlreadyApplied {
                    region: region_id,
                    generation: current.generation,
                }
            } else {
                ManifestVerdict::Stale {
                    region: region_id,
                    current_generation: current.generation,
                }
            };
            batch.put(
                ColumnFamily::Default,
                APPLIED_INDEX_KEY.to_vec(),
                index.0.to_be_bytes().to_vec(),
            );
            self.engine.write(batch)?;
            self.applied = index;
            return Ok(ApplyResult::manifest(index, verdict));
        }
        let (mut batch, fence_rejected): (_, Option<kv9_common::RegionId>) = match cmd {
            Command::Fenced { fence, inner } => {
                let adjudicator = self.adjudicator.as_ref().ok_or_else(|| {
                    kv9_common::Error::Raft(
                        "committed fenced entry on a node with no fence adjudicator: \
                         this node cannot compute the log-determined verdict and must \
                         not guess (install the adjudicator before enabling fenced \
                         proposals)"
                            .into(),
                    )
                })?;
                // `?` on the verdict itself: an adjudicator whose authoritative
                // read fails has no verdict to offer — that propagates as an
                // apply error (poison), never as a fabricated Fresh/Stale.
                if adjudicator.is_fresh(fence)? {
                    (inner.to_write_batch(), None)
                } else {
                    (
                        kv9_engine::WriteBatch::new(),
                        Some(kv9_common::RegionId(fence.region_id)),
                    )
                }
            }
            _ => (cmd.to_write_batch()?, None),
        };
        batch.put(
            ColumnFamily::Default,
            APPLIED_INDEX_KEY.to_vec(),
            index.0.to_be_bytes().to_vec(),
        );
        self.engine.write(batch)?;
        self.applied = index;
        if let Some(region) = fence_rejected {
            Ok(ApplyResult::fence_rejected(index, region))
        } else {
            Ok(ApplyResult::write_ok(index))
        }
    }

    /// Direct read-back from the state machine's KV (the `get` of the round-trip).
    pub fn get(&self, cf: ColumnFamily, key: &[u8]) -> Result<Option<Vec<u8>>> {
        self.engine.get(cf, key)
    }

    /// One region's authoritative manifest pair — the reconciliation query's
    /// read face (task #9). ONE engine get of ONE key: precondition P1 is
    /// structural here, a caller cannot fetch the two fields separately.
    /// Generation 0 with an empty change id = no manifest change has ever
    /// applied to this region.
    pub fn manifest_pair(&self, region: u64) -> Result<ManifestPair> {
        match self
            .engine
            .get(ColumnFamily::Default, &manifest_pair_key(region))?
        {
            Some(bytes) => ManifestPair::decode(&bytes),
            None => Ok(ManifestPair {
                generation: 0,
                last_change_id: Vec::new(),
            }),
        }
    }

    /// The changeset installed at one region generation, with its replicated
    /// watermark `(term, index)` — `None` if that generation has not been
    /// reached. Add-only: generations are never rewritten or removed in this
    /// regime (the contract's load-bearing property; any future remove path
    /// makes a durable ledger a prerequisite, OBJECT-STORAGE §7).
    pub fn manifest_at(&self, region: u64, generation: u64) -> Result<Option<(u64, u64, Vec<u8>)>> {
        match self
            .engine
            .get(ColumnFamily::Default, &manifest_gen_key(region, generation))?
        {
            None => Ok(None),
            Some(bytes) => {
                if bytes.len() < 16 {
                    return Err(kv9_common::Error::Raft(
                        "installed manifest shorter than its watermark fields".into(),
                    ));
                }
                let term = u64::from_be_bytes(bytes[..8].try_into().expect("8 bytes"));
                let idx = u64::from_be_bytes(bytes[8..16].try_into().expect("8 bytes"));
                Ok(Some((term, idx, bytes[16..].to_vec())))
            }
        }
    }
}

impl<E: ApplyStore> StateMachine for MemStateMachine<E> {
    fn apply(&mut self, entry: &CommittedEntry) -> Result<ApplyResult> {
        // Phase-1: the committed entry carries opaque bytes; decode to a Command, then
        // apply its write batch.
        let cmd = Command::decode(&entry.data)?;
        self.apply_command(entry.index, &cmd)
    }

    fn applied_index(&self) -> LogIndex {
        self.applied
    }
}

/// Drive one `propose → commit → apply` cycle over a [`crate::RaftGroup`] into a
/// [`StateMachine`] (ROADMAP Phase 1 spine).
///
/// Drains all currently-ready committed entries and applies them in order, returning the
/// last [`ApplyResult`]. On the [`crate::SingleNodeRaft`] stub, a `propose` commits
/// immediately, so this is the whole path; real consensus commits asynchronously.
pub fn drive_apply<R, S>(raft: &R, sm: &mut S) -> Result<Vec<ApplyResult>>
where
    R: crate::ReadyConsume + ?Sized,
    S: StateMachine,
{
    let ready = raft.take_ready()?;
    let mut out = Vec::with_capacity(ready.len());
    for entry in &ready {
        match entry.kind {
            // Barriers and conf changes never reach the state machine; the
            // production driver routes them (conf → apply_conf_change) and
            // reports applied progress separately. This helper only feeds
            // command payloads.
            crate::EntryKind::Noop
            | crate::EntryKind::ConfChangeV1
            | crate::EntryKind::ConfChangeV2 => {}
            crate::EntryKind::Command => out.push(sm.apply(entry)?),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RaftGroup, ReadyConsume, SingleNodeRaft};
    use kv9_common::{NodeId, RegionId};

    fn manifest_cmd(region: u64, id: &[u8], expected: u64, idx: u64) -> (LogIndex, Command) {
        (
            LogIndex(idx),
            Command::ManifestChange(ManifestChangePayload::for_harness(
                region,
                id.to_vec(),
                expected,
                format!("refs-of-{}", String::from_utf8_lossy(id)).into_bytes(),
                7,
                // Strictly BELOW the entry's own position: a manifest cannot
                // vouch for WAL it could not yet have absorbed.
                idx.saturating_sub(1),
            )),
        )
    }

    fn verdict(sm: &mut MemStateMachine, at: LogIndex, cmd: &Command) -> ManifestVerdict {
        match sm.apply_command(at, cmd).unwrap().outcome {
            ApplyOutcome::Manifest(v) => v,
            other => panic!("a manifest change must carry a manifest verdict, got {other:?}"),
        }
    }

    /// Task #9 discriminator row 1: a matching CAS applies, advances the pair
    /// atomically (one key, both fields), installs the changeset add-only,
    /// and reports the ONLY success-reporting verdict.
    #[test]
    fn a_matching_manifest_cas_applies_and_advances_the_pair() {
        let mut sm = MemStateMachine::new();
        let (at, cmd) = manifest_cmd(9, b"A", 0, 1);
        match verdict(&mut sm, at, &cmd) {
            ManifestVerdict::Applied { region, generation } => {
                assert_eq!(region, RegionId(9));
                assert_eq!(generation, 1);
            }
            other => panic!("expected Applied, got {other:?}"),
        }
        let pair = sm.manifest_pair(9).unwrap();
        assert_eq!(pair.generation, 1);
        assert_eq!(pair.last_change_id, b"A".to_vec());
        let (term, widx, changeset) = sm.manifest_at(9, 1).unwrap().expect("gen 1 installed");
        assert_eq!((term, widx), (7, 0));
        assert_eq!(changeset, b"refs-of-A".to_vec());
        // Watermark advanced like any applied entry.
        assert_eq!(sm.applied_index(), at);
    }

    /// Task #9 discriminator row 2 (crash-point-3 receipt half): a re-sent
    /// identity whose transition already happened is AlreadyApplied — a
    /// DISTINCT variant from Applied, nothing written, generation unmoved.
    /// The variant split is the guard: a caller matching Applied for
    /// "success" cannot be handed a duplicate.
    #[test]
    fn a_repeated_manifest_identity_is_already_applied_never_newly_accepted() {
        let mut sm = MemStateMachine::new();
        let (at1, cmd) = manifest_cmd(9, b"A", 0, 1);
        assert!(matches!(
            verdict(&mut sm, at1, &cmd),
            ManifestVerdict::Applied { .. }
        ));
        // Same identity re-sent (same change_id, same expected predecessor) at
        // a later log position — the safe-convergence path.
        let (at2, resend) = manifest_cmd(9, b"A", 0, 2);
        match verdict(&mut sm, at2, &resend) {
            ManifestVerdict::AlreadyApplied { region, generation } => {
                assert_eq!(region, RegionId(9));
                assert_eq!(generation, 1);
            }
            other => panic!("expected AlreadyApplied, got {other:?}"),
        }
        assert_eq!(sm.manifest_pair(9).unwrap().generation, 1);
        // Watermark still advanced (logical verdict, not an apply error).
        assert_eq!(sm.applied_index(), at2);
    }

    /// Task #9 discriminator row 3: a spent predecessor refuses typed with
    /// the CURRENT generation in the verdict; nothing written.
    #[test]
    fn a_stale_manifest_predecessor_refuses_typed_and_writes_nothing() {
        let mut sm = MemStateMachine::new();
        let (at1, a) = manifest_cmd(9, b"A", 0, 1);
        verdict(&mut sm, at1, &a);
        let (at2, b) = manifest_cmd(9, b"B", 0, 2);
        match verdict(&mut sm, at2, &b) {
            ManifestVerdict::Stale {
                region,
                current_generation,
            } => {
                assert_eq!(region, RegionId(9));
                assert_eq!(current_generation, 1);
            }
            other => panic!("expected Stale, got {other:?}"),
        }
        let pair = sm.manifest_pair(9).unwrap();
        assert_eq!(pair.generation, 1);
        assert_eq!(pair.last_change_id, b"A".to_vec());
        assert_eq!(sm.manifest_at(9, 2).unwrap(), None);
        assert_eq!(sm.applied_index(), at2);
    }

    /// The CAS chain: sequential changes each spend the predecessor the
    /// previous one produced; generations count successful applies exactly.
    #[test]
    fn sequential_manifest_changes_chain_generations() {
        let mut sm = MemStateMachine::new();
        for (i, id) in [b"A", b"B", b"C"].iter().enumerate() {
            let (at, cmd) = manifest_cmd(9, *id, i as u64, (i + 1) as u64);
            match verdict(&mut sm, at, &cmd) {
                ManifestVerdict::Applied { generation, .. } => {
                    assert_eq!(generation, (i + 1) as u64)
                }
                other => panic!("expected Applied at gen {i}, got {other:?}"),
            }
        }
        let pair = sm.manifest_pair(9).unwrap();
        assert_eq!(pair.generation, 3);
        assert_eq!(pair.last_change_id, b"C".to_vec());
        // Add-only: every generation's install is still readable.
        for g in 1..=3u64 {
            assert!(sm.manifest_at(9, g).unwrap().is_some(), "gen {g} present");
        }
    }

    /// Regions are independent pairs: a change on one region neither reads
    /// nor spends another region's predecessor.
    #[test]
    fn manifest_pairs_are_per_region() {
        let mut sm = MemStateMachine::new();
        let (at1, a) = manifest_cmd(9, b"A", 0, 1);
        verdict(&mut sm, at1, &a);
        let (at2, b) = manifest_cmd(10, b"B", 0, 2);
        assert!(matches!(
            verdict(&mut sm, at2, &b),
            ManifestVerdict::Applied { generation: 1, .. }
        ));
        assert_eq!(sm.manifest_pair(9).unwrap().last_change_id, b"A".to_vec());
        assert_eq!(sm.manifest_pair(10).unwrap().last_change_id, b"B".to_vec());
    }

    /// The discriminator's empty-identity guard runs BEFORE any CAS arm
    /// (review probe, Tess: empty id + expected=0 on a virgin region walked
    /// the first arm and returned Applied{1}). Both expected values must be
    /// the same typed Invalid — never Applied, never AlreadyApplied via
    /// empty==empty.
    #[test]
    fn an_empty_identity_never_matches_virgin_state() {
        for expected in [0u64, 1] {
            let mut sm = MemStateMachine::new();
            let (at, cmd) = manifest_cmd(9, b"", expected, 1);
            match verdict(&mut sm, at, &cmd) {
                ManifestVerdict::Invalid {
                    reason: ManifestInvalidReason::EmptyChangeId,
                    ..
                } => {}
                other => {
                    panic!("empty identity at expected={expected} must be Invalid, got {other:?}")
                }
            }
            assert_eq!(sm.manifest_pair(9).unwrap().generation, 0);
            // Logical refusal: the watermark still advanced.
            assert_eq!(sm.applied_index(), at);
        }
    }

    /// Review probe (Tess): a self-reported replicated watermark at or
    /// beyond the manifest entry's OWN log position must refuse typed —
    /// persisting it would later authorize recycling WAL the manifest
    /// cannot cover. Below the position is legal.
    #[test]
    fn a_watermark_at_or_beyond_own_entry_is_invalid() {
        let mut sm = MemStateMachine::new();
        let cmd = |wm: u64| {
            Command::ManifestChange(ManifestChangePayload::for_harness(
                9,
                b"A".to_vec(),
                0,
                b"refs".to_vec(),
                7,
                wm,
            ))
        };
        // Equal to own index: refused.
        match verdict(&mut sm, LogIndex(5), &cmd(5)) {
            ManifestVerdict::Invalid {
                reason: ManifestInvalidReason::WatermarkBeyondEntry,
                ..
            } => {}
            other => panic!("wm==index must be Invalid, got {other:?}"),
        }
        // Far beyond: refused, nothing persisted.
        match verdict(&mut sm, LogIndex(6), &cmd(100)) {
            ManifestVerdict::Invalid {
                reason: ManifestInvalidReason::WatermarkBeyondEntry,
                ..
            } => {}
            other => panic!("wm>index must be Invalid, got {other:?}"),
        }
        assert_eq!(sm.manifest_pair(9).unwrap().generation, 0);
        // Below: applies.
        assert!(matches!(
            verdict(&mut sm, LogIndex(7), &cmd(6)),
            ManifestVerdict::Applied { generation: 1, .. }
        ));
    }

    /// Review probe (Tess): P3 identity is (expected_generation, change_id),
    /// not the id alone. After A applies at expected=0, the identity
    /// (expected=99, id=A) is Stale — reporting AlreadyApplied would settle
    /// a DIFFERENT attempt on the strength of a matching id.
    #[test]
    fn already_applied_requires_the_full_predecessor_window() {
        let mut sm = MemStateMachine::new();
        let (at1, a) = manifest_cmd(9, b"A", 0, 1);
        verdict(&mut sm, at1, &a);
        let (at2, wrong_window) = manifest_cmd(9, b"A", 99, 2);
        match verdict(&mut sm, at2, &wrong_window) {
            ManifestVerdict::Stale {
                current_generation, ..
            } => assert_eq!(current_generation, 1),
            other => panic!("(expected=99, id=A) must be Stale, got {other:?}"),
        }
        // The true full identity still settles AlreadyApplied.
        let (at3, same) = manifest_cmd(9, b"A", 0, 3);
        assert!(matches!(
            verdict(&mut sm, at3, &same),
            ManifestVerdict::AlreadyApplied { generation: 1, .. }
        ));
    }

    /// Review probe (Tess): u64::MAX arithmetic is checked and typed at both
    /// layers — no debug panic, no wrap; apply still advances the watermark.
    #[test]
    fn generation_boundaries_are_checked_not_panics() {
        // Classifier side: expected=u64::MAX cannot have a successor window.
        let pair = ManifestPair {
            generation: 5,
            last_change_id: b"A".to_vec(),
        };
        assert_eq!(
            classify_reconciliation(&pair, u64::MAX, b"A"),
            ReconcileObservation::Unknown
        );
        // Apply side: the checked comparison itself must not panic with
        // expected=u64::MAX on a virgin region (falls to Stale).
        let mut sm = MemStateMachine::new();
        let (at, cmd) = manifest_cmd(9, b"A", u64::MAX, 1);
        match verdict(&mut sm, at, &cmd) {
            ManifestVerdict::Stale {
                current_generation, ..
            } => assert_eq!(current_generation, 0),
            other => panic!("expected Stale, got {other:?}"),
        }
        assert_eq!(sm.applied_index(), at);
    }

    /// Impossible pair states are decode errors, not accepted data.
    #[test]
    fn manifest_pair_decode_rejects_impossible_states() {
        // generation>0 with empty id.
        let mut v = 1u64.to_be_bytes().to_vec();
        assert!(ManifestPair::decode(&v).is_err());
        // generation==0 with an id.
        v = 0u64.to_be_bytes().to_vec();
        v.extend_from_slice(b"A");
        assert!(ManifestPair::decode(&v).is_err());
        // Legal states decode.
        let mut ok = 1u64.to_be_bytes().to_vec();
        ok.extend_from_slice(b"A");
        assert!(ManifestPair::decode(&ok).is_ok());
        assert!(ManifestPair::decode(&0u64.to_be_bytes()).is_ok());
    }

    /// The four-row query table, one test per SETTLED row and one per Unknown
    /// row — PAIRED on purpose (card acceptance): an implementation that
    /// merges exact-window-decidable with superwindow-undecidable reds one of
    /// these four, whichever direction it merged in.
    #[test]
    fn reconciliation_exact_window_mine_is_my_change_applied() {
        let pair = ManifestPair {
            generation: 5,
            last_change_id: b"A".to_vec(),
        };
        assert_eq!(
            classify_reconciliation(&pair, 4, b"A"),
            ReconcileObservation::MyChangeApplied { generation: 5 }
        );
    }

    /// Exact window, NOT mine: the unique transition from my predecessor
    /// belongs to another change — an authoritative negative from transition
    /// invariants, and DECIDABLE (this row must never degrade to Unknown).
    #[test]
    fn reconciliation_exact_window_non_mine_is_known_not_applied() {
        let pair = ManifestPair {
            generation: 5,
            last_change_id: b"B".to_vec(),
        };
        assert_eq!(
            classify_reconciliation(&pair, 4, b"A"),
            ReconcileObservation::KnownNotApplied {
                winner_change_id: b"B".to_vec()
            }
        );
    }

    /// Pending: my predecessor is not yet spent. The change may be
    /// committed-unapplied or commit AFTER this query — absence is not a
    /// negative answer; this row must never settle.
    #[test]
    fn reconciliation_pending_generation_is_unknown() {
        let pair = ManifestPair {
            generation: 4,
            last_change_id: b"Z".to_vec(),
        };
        assert_eq!(
            classify_reconciliation(&pair, 4, b"A"),
            ReconcileObservation::Unknown
        );
    }

    /// Superwindow: history lost — mine-applied-then-superseded and
    /// never-arrived are indistinguishable in this coordinate. This row must
    /// never settle EITHER WAY (calling it preempted would clear a slot on a
    /// state where mine may have applied).
    #[test]
    fn reconciliation_superwindow_is_unknown() {
        let pair = ManifestPair {
            generation: 7,
            last_change_id: b"B".to_vec(),
        };
        assert_eq!(
            classify_reconciliation(&pair, 4, b"A"),
            ReconcileObservation::Unknown
        );
        // And with MY id visible as the latest — still not exact-window, so
        // still Unknown: a superwindow match is not my window's transition.
        let pair_mine = ManifestPair {
            generation: 7,
            last_change_id: b"A".to_vec(),
        };
        assert_eq!(
            classify_reconciliation(&pair_mine, 4, b"A"),
            ReconcileObservation::Unknown
        );
    }

    /// A pair BEHIND the attempt cannot happen under P4; observing it is a
    /// broken precondition, never a settled verdict.
    #[test]
    fn reconciliation_behind_pair_is_unknown() {
        let pair = ManifestPair {
            generation: 2,
            last_change_id: b"Z".to_vec(),
        };
        assert_eq!(
            classify_reconciliation(&pair, 4, b"A"),
            ReconcileObservation::Unknown
        );
    }

    /// An empty identity matches nothing: virgin pairs hold an empty
    /// last_change_id, and empty==empty must not read generation-0 state as
    /// someone's success.
    #[test]
    fn reconciliation_empty_identity_is_unknown() {
        let pair = ManifestPair {
            generation: 1,
            last_change_id: Vec::new(),
        };
        assert_eq!(
            classify_reconciliation(&pair, 0, b""),
            ReconcileObservation::Unknown
        );
    }

    /// The applied watermark rides in the same batch as the data: a state
    /// machine re-created over the SAME engine resumes at the durable
    /// watermark instead of 0 (the durable-data/volatile-watermark mismatch).
    #[test]
    fn applied_watermark_recovers_with_the_engine() {
        let engine = Arc::new(MemEngine::new());
        let mut sm = MemStateMachine::with_engine(Arc::clone(&engine)).unwrap();
        let cmd = Command::Put {
            cf: 0,
            key: b"k".to_vec(),
            value: b"v1".to_vec(),
        };
        sm.apply_command(LogIndex(3), &cmd).unwrap();
        drop(sm);

        let sm2 = MemStateMachine::with_engine(Arc::clone(&engine)).unwrap();
        assert_eq!(sm2.applied_index(), LogIndex(3));
        // Control (sensitivity): a fresh engine reports 0 — recovery reads
        // real state, not a constant.
        let fresh = MemStateMachine::with_engine(Arc::new(MemEngine::new())).unwrap();
        assert_eq!(fresh.applied_index(), LogIndex(0));
    }

    /// A corrupt watermark value refuses to open (typed error) — never
    /// silently coerces to 0 and replays the log over unknown state.
    #[test]
    fn corrupt_watermark_refuses_to_open() {
        let engine = Arc::new(MemEngine::new());
        let mut batch = kv9_engine::WriteBatch::new();
        batch.put(
            ColumnFamily::Default,
            APPLIED_INDEX_KEY.to_vec(),
            vec![1, 2, 3], // wrong width
        );
        Engine::write(engine.as_ref(), batch).unwrap();
        assert!(MemStateMachine::with_engine(Arc::clone(&engine)).is_err());
        // Control: a valid 8-byte watermark opens fine.
        let mut batch = kv9_engine::WriteBatch::new();
        batch.put(
            ColumnFamily::Default,
            APPLIED_INDEX_KEY.to_vec(),
            9u64.to_be_bytes().to_vec(),
        );
        Engine::write(engine.as_ref(), batch).unwrap();
        assert_eq!(
            MemStateMachine::with_engine(engine)
                .unwrap()
                .applied_index(),
            LogIndex(9)
        );
    }

    /// Commands with NO data mutations (Noop) must still advance the durable
    /// watermark — otherwise a restart regresses it and re-delivers entries
    /// the group considers applied.
    #[test]
    fn empty_batch_commands_persist_the_watermark() {
        let engine = Arc::new(MemEngine::new());
        let mut sm = MemStateMachine::with_engine(Arc::clone(&engine)).unwrap();
        sm.apply_command(
            LogIndex(1),
            &Command::Put {
                cf: 0,
                key: b"k".to_vec(),
                value: b"v".to_vec(),
            },
        )
        .unwrap();
        sm.apply_command(LogIndex(2), &Command::Noop).unwrap();
        drop(sm);
        // Restart: the watermark reflects the Noop, not just the last data write.
        let sm2 = MemStateMachine::with_engine(engine).unwrap();
        assert_eq!(sm2.applied_index(), LogIndex(2));
    }

    /// Redelivery at or below the watermark is skipped — replay after restart
    /// cannot double-apply. Sensitivity: the skipped command carries a
    /// DIFFERENT value; if it were re-applied the assertion would see it.
    #[test]
    fn replayed_entries_below_watermark_are_skipped() {
        let engine = Arc::new(MemEngine::new());
        let mut sm = MemStateMachine::with_engine(Arc::clone(&engine)).unwrap();
        sm.apply_command(
            LogIndex(5),
            &Command::Put {
                cf: 0,
                key: b"k".to_vec(),
                value: b"original".to_vec(),
            },
        )
        .unwrap();

        // Restarted state machine over the same engine replays the log; a
        // conflicting rewrite of index 5 must be ignored.
        let mut sm2 = MemStateMachine::with_engine(Arc::clone(&engine)).unwrap();
        sm2.apply_command(
            LogIndex(5),
            &Command::Put {
                cf: 0,
                key: b"k".to_vec(),
                value: b"DOUBLE-APPLIED".to_vec(),
            },
        )
        .unwrap();
        assert_eq!(
            sm2.get(ColumnFamily::Default, b"k").unwrap(),
            Some(b"original".to_vec()),
            "entries at/below the watermark must not re-apply"
        );
        // …while a NEW index applies normally (a watermark, not a wall).
        sm2.apply_command(
            LogIndex(6),
            &Command::Put {
                cf: 0,
                key: b"k".to_vec(),
                value: b"next".to_vec(),
            },
        )
        .unwrap();
        assert_eq!(
            sm2.get(ColumnFamily::Default, b"k").unwrap(),
            Some(b"next".to_vec())
        );
    }

    /// Phase-1 milestone (ROADMAP): the first concrete task — a single-node raft with a
    /// `MemEngine` state machine and a `propose(put) → apply → get` round-trip, through
    /// the real encode → propose → take_ready → decode → apply path.
    #[test]
    fn propose_put_apply_get_roundtrip() {
        let raft = SingleNodeRaft::new(NodeId(1), RegionId(1));
        let mut sm = MemStateMachine::new();

        let cmd = Command::Put {
            cf: 0,
            key: b"k".to_vec(),
            value: b"v".to_vec(),
        };
        // The real path proposes the command and applies via take_ready → decode.
        raft.propose(&cmd).unwrap();
        let results = drive_apply(&raft, &mut sm).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(
            sm.get(ColumnFamily::Default, b"k").unwrap(),
            Some(b"v".to_vec())
        );
    }

    /// The typed variant of the round-trip that does not depend on the entry codec: it
    /// applies the command directly at its committed index. This documents the Phase-1
    /// spine working end to end today.
    #[test]
    fn propose_put_apply_get_roundtrip_typed() {
        let raft = SingleNodeRaft::new(NodeId(1), RegionId(1));
        let mut sm = MemStateMachine::new();

        let index = raft.propose(&Command::Noop).unwrap();
        let _ = raft.take_ready().unwrap();
        let cmd = Command::Put {
            cf: 0,
            key: b"k".to_vec(),
            value: b"v".to_vec(),
        };
        sm.apply_command(index, &cmd).unwrap();

        assert_eq!(sm.applied_index(), index);
        assert_eq!(
            sm.get(ColumnFamily::Default, b"k").unwrap(),
            Some(b"v".to_vec())
        );
    }

    /// A fixed-verdict adjudicator for exercising both fence outcomes.
    struct Verdict(bool);
    impl FenceAdjudicator for Verdict {
        fn is_fresh(&self, _fence: &crate::RegionFence) -> Result<bool> {
            Ok(self.0)
        }
    }

    /// An adjudicator whose authoritative read always fails — the third state.
    struct ReadFails;
    impl FenceAdjudicator for ReadFails {
        fn is_fresh(&self, _fence: &crate::RegionFence) -> Result<bool> {
            Err(kv9_common::Error::Engine(
                "authoritative epoch read failed".into(),
            ))
        }
    }

    fn fenced_put(key: &[u8], value: &[u8]) -> Command {
        Command::Fenced {
            fence: crate::RegionFence {
                region_id: 1,
                conf_ver: 1,
                version: 1,
            },
            inner: crate::FencedInner::Write {
                ops: vec![crate::KvOp::Put {
                    cf: 0,
                    key: key.to_vec(),
                    value: value.to_vec(),
                }],
            },
        }
    }

    /// The load-bearing pair for task #48 layer 2: a rejected fence writes no
    /// data but MUST advance the durable watermark exactly like an accepted
    /// entry — the mutant that turns rejection into an apply error (or skips
    /// the watermark write on the rejection path) must go red on the watermark
    /// assertions, the stall symptom Ren predicted.
    #[test]
    fn a_rejected_fence_advances_the_watermark_and_writes_nothing() {
        let engine = Arc::new(MemEngine::new());
        let mut sm = MemStateMachine::with_engine(Arc::clone(&engine)).unwrap();
        sm.set_fence_adjudicator(Arc::new(Verdict(false)));

        let result = sm
            .apply_command(LogIndex(1), &fenced_put(b"k", b"v"))
            .expect("a rejected fence is a logical outcome, never an apply error");
        assert_eq!(
            result.outcome,
            ApplyOutcome::FenceRejected(kv9_common::RegionId(1)),
            "the verdict must be typed AND name the rejected region"
        );
        assert_eq!(
            sm.get(ColumnFamily::Default, b"k").unwrap(),
            None,
            "a rejected fence must write nothing"
        );
        assert_eq!(
            sm.applied_index(),
            LogIndex(1),
            "a rejected fence must still advance the applied watermark"
        );
        // The advance must be DURABLE (same atomic batch as an accepted entry):
        // a state machine re-opened over the same engine resumes past the
        // rejected entry instead of re-delivering it.
        drop(sm);
        let reopened = MemStateMachine::with_engine(engine).unwrap();
        assert_eq!(
            reopened.applied_index(),
            LogIndex(1),
            "the rejected entry's watermark advance must be durable"
        );
    }

    #[test]
    fn a_fresh_fence_applies_the_inner_ops() {
        let mut sm = MemStateMachine::new();
        sm.set_fence_adjudicator(Arc::new(Verdict(true)));
        let result = sm
            .apply_command(LogIndex(1), &fenced_put(b"k", b"v"))
            .unwrap();
        assert_eq!(result.outcome, ApplyOutcome::Plain);
        assert_eq!(
            sm.get(ColumnFamily::Default, b"k").unwrap(),
            Some(b"v".to_vec()),
            "a fresh fence must apply the inner ops"
        );
    }

    /// No adjudicator installed = typed apply ERROR, and the watermark must NOT
    /// advance. Adjudicator presence is node-local configuration, not log
    /// state: a local "reject" default would let differently-configured
    /// replicas apply the same committed entry to different states, and an
    /// empty-batch fallback would report success while silently dropping the
    /// write (review round). The driver's poison-on-apply-error is the correct
    /// consequence — same family as decoding an unknown entry version.
    #[test]
    fn without_an_adjudicator_a_fence_is_a_typed_apply_error() {
        let mut sm = MemStateMachine::new();
        let err = sm
            .apply_command(LogIndex(1), &fenced_put(b"k", b"v"))
            .expect_err("a node without an adjudicator must refuse, not guess");
        assert!(
            err.to_string().contains("no fence adjudicator"),
            "the refusal must name the missing adjudicator: {err}"
        );
        assert_eq!(sm.get(ColumnFamily::Default, b"k").unwrap(), None);
        assert_eq!(
            sm.applied_index(),
            LogIndex(0),
            "a refused entry must not advance the watermark"
        );
    }

    /// The adjudicator's authoritative read failing is a THIRD state, not a
    /// verdict: the error must propagate with the adjudicator's own message
    /// recognizable in it.
    #[test]
    fn an_adjudicator_read_failure_is_an_apply_error_not_a_verdict() {
        let mut sm = MemStateMachine::new();
        sm.set_fence_adjudicator(Arc::new(ReadFails));
        let err = sm
            .apply_command(LogIndex(1), &fenced_put(b"k", b"v"))
            .expect_err("a failed authoritative read must propagate, not become a verdict");
        assert!(
            err.to_string().contains("authoritative epoch read failed"),
            "the adjudicator's own error must survive recognizably: {err}"
        );
    }

    /// Split from the error-identity test on purpose: this one ignores the
    /// call's Ok/Err entirely and asserts only STATE, so the mutant that
    /// crushes `Err` into `Ok(false)` — dressing a node-local read failure as
    /// a deterministic rejection — cannot hide behind the `expect_err` firing
    /// first. Under that mutant the fabricated rejection consumes the log
    /// position, and THIS test's watermark assertion is the one that goes red
    /// (with multiple assertions in one test, "it went red" doesn't say which
    /// one guards; one mutant, one owning assertion).
    #[test]
    fn a_failed_adjudicator_read_consumes_nothing() {
        let mut sm = MemStateMachine::new();
        sm.set_fence_adjudicator(Arc::new(ReadFails));
        let _ = sm.apply_command(LogIndex(1), &fenced_put(b"k", b"v"));
        assert_eq!(
            sm.get(ColumnFamily::Default, b"k").unwrap(),
            None,
            "a failed read must not write data"
        );
        assert_eq!(
            sm.applied_index(),
            LogIndex(0),
            "a failed read must not consume the log position"
        );
    }
}
