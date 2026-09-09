# Metadata planning: protocol, model and proof obligations

## Contract and implementation

Catalog constraints are checked while building an atomic batch from a local
snapshot. `CatalogTxn` application does not re-run uniqueness or allocation
checks. A stale plan can therefore overwrite catalog rows or allocate an already
used ID even though the underlying Raft log is consistent.

The production runtime serializes planners with a **per-node** catalog mutex.
While holding it, it appends a Noop to the same Raft log and waits for that exact
entry to apply. Only then does it read the catalog and build the transaction.
It submits the batch using the barrier's term as its expected planning term.
Leadership, term comparison and append happen under the same Raft peer lock.
A successful response requires the exact submitted `(term, index)` receipt.

A timeout releases the mutex but does not cancel an appended write. A later
planner's ordered barrier drains that write if it survives in the log. Merely
reading an applied committed prefix does not necessarily include an earlier
uncommitted proposal. A leadership change invalidates a plan even when its
original node later becomes leader again.

| Model operation | Rust implementation | Abstraction and remaining obligation |
|---|---|---|
| `Begin` | `RuntimeBackend::create_keyspace`, `lock_catalog_txn`, start of `prepare_catalog` | Combines mutex acquisition and barrier append; models one attempt per request |
| `Barrier` | `prepare_catalog`, `propose_and_wait`, `NodeDriver::wait_applied` | Exact Noop application, including earlier ambiguous proposals |
| `Plan`, `NextId`, `NamesThrough` | `Node::build_create_keyspace_command`, `MetaTxn::allocate_id`, catalog insert checks | One allocator and unique name; region/default transaction-group rows, PK/FK generality and bootstrap are not yet represented |
| `Submit` | `RuntimeBackend::commit_catalog`, `RaftPeer::propose_in_term` | Leader/term check and append are one action, matching the one-lock implementation |
| `Commit`, `Elect` | raft-rs 0.7 log commitment and leader changes | Abstract consensus contract, not a proof of upstream Raft or its kv9 persistence integration |
| `Apply` | `RaftStateMachine` atomic `CatalogTxn` batch application | Per-node ordered applied prefix; atomic engine data/position recovery remains a separate obligation |
| `Finish` | exact `ApplyWaitOutcome::Applied` receipt | Receipt identity includes the actual submission term, independently of the stored planning term |
| `Timeout` | `ApplyWaitError::Unconfirmed` and scope exit | Unknown outcome, mutex released, command retained; no automatic retry is assumed |

Two existing Rust tests exercise relevant implementation boundaries:
`catalog_planning_barrier_drains_an_earlier_unconfirmed_command` runs the ordered
barrier against a real Raft peer with application paused, and
`catalog_proposal_checks_planning_term_before_append` checks that stale plans do
not enter the real log. They are concrete regression evidence, not a mechanical
refinement proof of the TLA+ transitions.

## State and failure assumptions

The [TLA+ model](../proofs/tla/MetadataPlanning.tla) has one accepted Raft log,
a monotonically increasing term, a leader identity, a committed index and an
independently advancing applied index for each coordinator. Each request has a
host, phase, barrier position/term, planned ID and submitted position/term.
`succeeded` records acknowledged requests; `unknownPending` is a coverage-only
ghost set recording writes that timed out before commitment. Neither set enables
protocol actions.

The model assumes:

1. Elections preserve the committed prefix, choose at most one accepted leader
   per term, and may retain or discard any uncommitted suffix. An election appends
   a current-term Noop. Only current-term entries directly advance commitment.
2. Each node applies a prefix of the committed log in order. Committed entries
   and acknowledged state survive recovery. These are consensus/persistence
   assumptions, not conclusions of this catalog model.
3. No catalog writer bypasses the same planning mutex, barrier and term fence.
   There is one atomic allocator update plus catalog insertion per modeled write.
   Initially the catalog is valid and its normalized next ID is 1.
4. Name equality is stable, request identities are unique, IDs do not overflow,
   and no deletion or rename occurs in this model. A local snapshot is coherent.

The log abstraction permits arbitrary commit/apply delay and uncommitted suffix
replacement. Requests and plans survive leader changes; pending writes may commit
after timeout. It does not model messages, quorum collection, joint consensus,
disk operations, object-store publication, node reincarnations or network topology.
Doomed branches of stale leaders' private logs are omitted. Proving that every
implementation execution projects onto this accepted log remains open.

For safe prefixes, `NamesThrough` and the last write's allocator value represent
the snapshot needed by this create-only projection. The allocator is the last
blindly written ID plus one, **not** the number of writes; this preserves the
duplicate-allocation fault in negative controls. `CatalogSafety` checks committed
entries themselves, so an overwrite cannot hide a conflicting allocation.

## Safety argument for arbitrary finite executions

The following induction argument is parameterized by finite coordinator/request
sets and any term budget. The committed-prefix, log/index shape, exact-receipt,
planner-isolation, freshness and catalog-uniqueness
parts now have [TLAPS proofs](../proofs/tlaps/README.md) importing this same model:
41 declarations and 459 checked obligations. Conditional draining remains a
written argument whose deductive mechanization is open. Full `TypeOK` bounds are
also open. TLC's finite instances do not discharge those obligations.

**Prefix preservation.** Initially the committed/applied prefixes are empty.
`Begin` and `Submit` append; `Elect` retains at least the committed prefix;
`Commit` advances within the log; `Apply` advances within the committed prefix.
Other actions do not change these values. Thus no applied/committed entry is
subsequently replaced and every local snapshot is a committed prefix.

**Planner isolation within a term.** Let request `r` have an applied barrier at
position `b` in the current term `t`. Since terms increase strictly, no election
has occurred since that barrier was appended. Only its host can append catalog
writes in this term, and its active request holds that host's mutex. No other
request on that host can begin until it releases the mutex. Requests on other
hosts cannot submit there. Consequently no catalog write can be appended between
this barrier and `r`'s submission. Earlier timed-out writes, if retained, precede
`b` and are included in the snapshot after the barrier applies.

**Plan freshness.** `Plan` runs after the exact barrier applies, while the same
mutex is held. If its planning term is still current, the isolation lemma means
its view includes every catalog write in the accepted log, including its
uncommitted suffix (any catalog write there had to precede the applied barrier).
Its allocated ID therefore equals `NextId(Len(log))`, and a successful plan's name
is absent from the log's catalog. If the term changes before `Submit`, its term
guard rejects the stale plan. This establishes `FreshPlans` without adding a
freshness check to `Submit` itself.

**Catalog preservation.** Strengthen uniqueness with positive, strictly increasing
retained write IDs in log order. A checked natural induction establishes that the
nonempty bounded write-index set has a maximum, making `LastWrite` well-defined.
The next ID exceeds every retained write ID. This holds initially. A successful
`Submit` uses the freshness lemma, appending the next ID and an absent name.
Noops leave write order unchanged; suffix truncation keeps a prefix; other
actions leave the log unchanged. Induction gives `LogSafety`, hence
`CatalogSafety` for its committed prefix. This is uniqueness of retained and
committed writes; reusing an ID from a provably discarded suffix is permitted.

**Receipt preservation.** `Finish` records success only after its submitted
entry's exact identity is in the local applied prefix. Prefix preservation then
keeps that entry, its ID and its position permanently in the committed prefix.
Induction establishes `ReceiptSafety`. An index watermark alone cannot establish
this premise because a different term's entry may occupy the same index.

`MetadataReceipt.ReceiptAlways` now establishes `Spec => []ReceiptSafety` by
initialization, transition preservation and temporal induction. Its strengthened
invariant also tracks the request phase and each retained entry's submitted
index, term and planned value. `MetadataShape.ShapeAlways` supplies log/index
well-formedness, and `MetadataPrefix.PrefixAlways` states the canonical TLA+
property `Spec => [][PrefixStable]_vars`. All imported project lemmas are checked
freshly; these are parameterized theorems, not the two finite TLC cases. They
still rely on the abstract Raft transitions described above.

`MetadataFreshness.FreshAlways` establishes `Spec => []FreshPlans` using the
planning-control and barrier invariants. `MetadataUniqueness.CatalogAlways`
establishes `Spec => [](LogSafety /\ CatalogSafety)` using that freshness result
at the term-fenced submission point. No action acquires a new uniqueness check or
changes the blind allocator to make the proof pass. Negative controls remove the
mutex, weaken the barrier to a ReadIndex-only wait, remove the submission term
fence and stop advancing the allocator; each must fail its specific preservation
or allocation obligation before the restored full inventory passes again.

## Conditional draining argument

`FairSpec` adds weak fairness for `Commit` and for every node's `Apply` action.
Each request appends at most two entries and each election consumes one unit of
the finite term budget. Therefore every behavior has a suffix with no further
appends or elections. Let the fixed log length on that suffix be `L`. Unless the
log is empty, its last entry belongs to the current term, so `Commit` stays enabled
while the committed index is below `L`. Weak fairness and strict index increases
eventually bring it to `L`. Each node's `Apply` then stays enabled while its applied
index is below `L`; the same argument brings every node to `L`. Finitely many
nodes imply `<>[]Drained`.

There is no fairness assumption for client admission and no claim that every
client request succeeds. This property describes eventual draining after a finite
workload and finite leader changes, with eventual replication and application.
Safety uses `Spec` without fairness and must hold when progress stops. Removing
apply fairness must produce a temporal counterexample. The broader availability
proof needs eventual leader election, communicating/durable quorums, bounded
resource use and endpoint failover; it is still open.

## Counterexample obligations and finite checks

The [runner](../scripts/check-tla.py) checks distinct/shared names, two coordinator
identities, two requests and up to three terms. The two name mappings cover the
two equality patterns for two requests; they do not cover arbitrary workload
sizes. Both fingerprint polynomial runs must finish with matching exploration
counts, all five invariants, the temporal property and positive action coverage.

| Isolated change | Required counterexample |
|---|---|
| Use an applied committed-prefix read instead of waiting for the ordered barrier | First request times out with its write uncommitted; second planner misses it; both conflicting batches can commit. The control also requires a committed current-term entry, so it does not rely on reading before Raft has established that term. |
| Remove only `planningTerm = term` | A plan certified by an earlier term's barrier can be admitted when its host is leader again, after another leader has appended a conflicting catalog transaction. Role checking alone does not prevent it. |
| Remove exact receipt identity and keep only the applied-index check | A discarded write's index is replaced; applying the replacement incorrectly acknowledges the original request. |
| Remove weak fairness of application | A node can stutter forever behind a committed prefix; eventual draining is false. |

The first two changes are checked with both name mappings. Negative controls use
the corresponding final catalog/receipt property rather than stopping early at
the stronger plan-freshness invariant. Every mutation has a separate baseline
and restored run using the identical configuration. Positive reachability checks
also retain all safety invariants, preventing a model that merely refuses every
write from satisfying the acceptance gate.

## Remaining work and availability boundary

- Complete full `TypeOK` bounds and the conditional draining proof, extending the
  checked TLAPS safety inventory. The existing 12 Lean lemmas do not prove this
  metadata protocol.
- Prove/refine the assumed Raft contract: log matching, leader completeness,
  durable Ready ordering, exact receipt publication and configuration changes.
  The ignored `LightReady` commit update reproduced in
  [#35](https://github.com/c4pt0r/kv9/issues/35) is a concrete recovery-boundary
  defect outside this accepted-log model and takes priority in that audit.
- Extend the metadata model to arbitrary catalog batches, PK/FK constraints,
  bootstrap, membership, allocator bounds, cancellations and typed `Replaced`
  retry loops. One attempt per modeled request does not verify those loops.
- Audit every production catalog writer for the shared serialization protocol;
  the legacy in-memory `Catalog` facade is a separate implementation boundary.
- Map counterexamples to deterministic Rust schedules and actual Chaos Mesh
  histories; formal models do not replace the existing fault matrix.
- Establish process and host failover separately. Multiple model coordinator
  identities do not prove a deployment has no single point of failure. The
  object-store dependency remains the only permitted external availability
  exception, with its assumed guarantees explicit.

Issues [#9](https://github.com/c4pt0r/kv9/issues/9),
[#11](https://github.com/c4pt0r/kv9/issues/11) and
[#14](https://github.com/c4pt0r/kv9/issues/14) remain open until their full gates
are met.
