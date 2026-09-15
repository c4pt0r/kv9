# Automatic checkpoint ownership and durable upload plans

This increment connects the existing whole-engine checkpoint worker to the
[tracking-only retention ledger](RETENTION-LEDGER.md). Before uploading any SST,
the worker durably saves its exact upload plan and confirms a replicated Pending
owner. Positive typed manifest settlement transfers the complete resource
closure to a Published Version owner before clearing the local journal.

This does not complete C04/#14 or S07/#21. Legacy references, live readers,
destination installation, negative-attempt history and final abort release still
need integration. Object deletion remains disabled. The existing metadata Raft
group owns the ledger; this change adds no coordinator or service dependency.

## Exact identity and ordering

The worker freezes data, applied position, historical scope and its scheduling
revision from the same immutable engine view. Planning constructs the exact SST
bytes and canonical checkpoint descriptor without an ObjectStore argument or
external I/O. Existing 48 MiB checkpoint and 1 MiB descriptor bounds apply.

Let `M` be the exact canonical descriptor, `g` the actual predecessor manifest
generation, `R` the certified root digest, and `H` SHA-256. Identity derivation is:

| Field | Derivation |
| --- | --- |
| Operation | Existing `CheckpointManifest::change_id(g)`, binding `g` and `M`. |
| Subject | `H(M)`; it is not marked as a recovery anchor. |
| SST resource | Root `R`, SST kind, instance `H(object_key)[0..16]`, exact content digest. |
| Pending owner | `H("kv9-checkpoint-pending-v1" || R || operation)[0..16]`, generation 1. |
| Version owner | `H("kv9-checkpoint-version-v1" || R || operation)[0..16]`, generation 1. |

The closure is sorted by instance and passes the production ledger codec and
semantic bounds before journal publication. Owner scope includes the manifest's
region and configuration/range epochs. A topology generation is not a substitute
for the actual predecessor manifest generation.

The worker performs these steps, stopping on every unconfirmed outcome:

1. Persist the exact upload plan using file sync, atomic rename and parent sync.
2. Initialize only an empty ledger, register the closure, acquire and Publish the
   Pending owner through the existing term-fenced metadata API. Each operation
   needs its exact applied receipt; owner reads need fresh Raft barriers.
3. Upload the original SST bytes, then GET and verify their exact bytes and
   descriptor. Only this path can construct `PreparedFlush`.
4. Upgrade the same journal identity to the existing verified descriptor format
   and reconcile the exact `ManifestAttempt` through `ManifestSeam`.
5. On positive typed settlement, Share to Version, Publish Version, quiesce
   Pending under that exact successor, and Release Pending. Clear the journal
   only after these operations are confirmed.

Already Published Pending retries preserve their binding. Quiesced/Released
Pending retries require the exact Published Version and never reacquire
generation 1. Recovery from each partially applied transfer repeats the same
operation prefix. `EffectSettled` certifies the intended effect through the
existing seam; it does not invent a receipt for the original CAS invocation.

Typed negative settlement clears local scheduling but **retains the Pending
owner**. This is conservative retention, not a finished abort/history protocol:
there is no certified final abort release and these pins can accumulate.
Unknown outcomes retain the journal. Absence of an object, an RPC error or
suspected process death cannot release ownership.

## Durable journal compatibility

`KV9PENDING\x01` remains the descriptor of a remotely verified prepared flush.
The new pre-upload `KV9PENDING\x02` contains, in order:

| Field | Encoding |
| --- | --- |
| Magic | Literal version 2 bytes. |
| Expected generation | 8 bytes, little endian. |
| Descriptor length | 4 bytes, little endian. |
| Descriptor | Exact bounded canonical checkpoint bytes. |
| Objects | Exact bodies, in descriptor order, with descriptor-defined sizes. |
| Checksum | SHA-256 of all preceding bytes. |

Loading v2 checks the outer checksum, generation, bounds, descriptor, every
object's size/hash/column family/key range/count, and exact end of input. A
checksum-valid malformed body still refuses. Temporary partial files are not an
authoritative slot; an invalid published slot is never replaced by an empty one.
Recovery also validates root/region and the committed term at the checkpoint cut.

A sealed `PreparedFlush` may upgrade v2 to v1 only with the identical descriptor
and predecessor generation. Staging cannot replace a different unresolved
attempt or downgrade v1 to v2. After a PUT that created an object but returned an
error, reopening retains the original bytes and identity and can repeat the
upload and verification. No new snapshot is needed to recover that local plan.

Old readers reject v2. This is explicit fail-closed compatibility, not a mixed
version rollout/backfill fence. Full recovery still depends on the existing
verified checkpoint and retained Raft history. A node's pending journal is local
scheduling state: a follower defers its owner mutation until it leads again.
Other voters can continue serving and checkpointing; cluster-wide job takeover
is not implemented here.

## Preventing checkpoint feedback

Ledger writes must be included in snapshots and WAL replay, but cannot themselves
keep scheduling checkpoints. `MemEngine::write_applied` excludes only Default
column-family keys under the existing manifest prefix or exact retention-v1
prefix from the scheduling counter. Identical bytes in another column family
and unknown retention versions still increment it. All mutations and applied
positions remain in the immutable state and durable log.

The worker stores the revision captured with the frozen view, and sets its last
completed revision to that value only after positive settlement and owner
transfer. If no data write follows the frozen cut, bookkeeping leaves the
revision unchanged, so subsequent ticks do not create another checkpoint. If a
data write follows the cut, the new revision remains outstanding. This argument
assumes the existing saturating counter has not exhausted; counter rollover and
unconditional progress are not proved. Restart may cause an additional flush
because the completed scheduling counter is process-local.

The filter adds a prefix predicate to applied-batch scheduling. No throughput
or latency improvement is claimed. Encoding and validating a bounded plan also
temporarily keeps multiple copies of its bytes; this is not a peak-memory or
dataset-larger-than-RAM result.

## Proof and implementation boundary

`proofs/tla/checkpoint_owner/CheckpointOwner.tla` models a durable journal slot,
Pending/Version phases, remote creation, verified preparation, manifest effects
and typed settlement evidence. Operation symbols denote immutable validated
root/scope/predecessor/descriptor/object bindings. Replay of an already applied
step stutters. Restart preserves the durable projection; volatile scheduler
state and partial temporary writes are outside it.

| Model event | Implementation / composition premise |
| --- | --- |
| `CPlan` | `stage_planned`, validated atomic journal publication, single local worker. |
| `CAcquire` / `CPin` | `CheckpointOwners::before_io`; ledger initialization, exact binding and fresh receipts compose with the ledger/metadata proofs. |
| `CPut` / `CVerify` | Owner confirmation before `PendingFlush::recover`; exact remote GET validation before constructing a prepared capability. |
| `CApply` / evidence | Existing `ManifestSeam` and ordered manifest application; no caller-supplied Boolean mints settlement. |
| Share / Publish / quiesce / release | Positive-settlement helper and the ledger's complete-closure successor checks. |
| `CClear` | Positive transfer complete, or typed negative settlement retaining owners. |
| `CRestart` | Atomic local journal recovery and replicated ledger recovery are premises; this theorem does not prove filesystem or Raft internals. |

Seven TLAPS theorems discharge 43 strict obligations: initialization, inductive
preservation, temporal invariance, covered remote objects, settled-only journal
clearing, no replacement of an active identity, and durable restart identity and
ownership. TLC explores one- and two-operation instances (27 and 405 distinct
states). Six single-fault models must produce actual invariant counterexamples;
proof holes, unapproved axioms and the unsafe remote-I/O proof must be rejected.

The first protocol control expected the transfer invariant, but TLC detected
the earlier coverage violation when quiescing the last Published owner. That
failed runner attempt is retained; only the expected first invariant was
corrected. The model, invariant order and fault remain unchanged.

This is a source-mapped safety proof, not a mechanized Rust refinement or a proof
of complete legacy coverage, reader drainage, physical deletion, end-to-end
liveness or the scheduling counter. The scheduling argument above has a
separate engine regression. Negative evidence does not prove that an uncertain
original attempt never committed.

The follow-up model explicitly permits a refused submission after an earlier
submission of the same identity already applied. A stale receipt only rejects
its own submission. The worker already retains Pending on negative settlement;
this aligns the model with that conservative implementation without changing
Rust. All seven theorems and 43 strict obligations still pass. The finite state
counts remain 27 and 405; those counts alone do not establish event ordering.
An additional constrained TLC witness therefore requires application before
negative evidence and reaches journal clearing with Published Pending and no
Version owner. Its nine-state trace demonstrates this ordering while the
production safety invariants continue to hold. The witness's deliberately false
extra invariant is a reachability check, not a production safety failure.

The first follow-up runner stopped because broadening the negative-evidence
guard made a fault's text selector ambiguous. The corrected selector names
`CApply` explicitly; the six faults and three strict proof rejection checks
pass again. Both attempts are retained under
`/mnt/data/kv9-work/checkpoint-owner-late-refusal-proof-20260915-{first,second}`.
Their [portable proof evidence](checkpoint-owner-late-refusal-v1/README.md)
includes the original failure and the ordered witness.

## Current local validation

The full local workspace library run passed **731 tests, four ignored** on the
explicit NVMe fixture filesystem. Workspace/all-target checks, Clippy with
warnings denied, and the `PlannedFlush` capability privacy doctest also passed.
New tests exercise exact pre-upload bytes, ambiguous successful PUT recovery,
all journal truncations, checksum-valid malformed bodies and scheduling scope.

A real three-voter test injects an error after each of eight distinct applied
owner-operation prefixes, retries the exact binding, and reopens all stores.
All eight converge to Released Pending and Published Version. This tests
uncertain ledger replies and handoff recovery; the test invokes the private
post-positive helper and does not itself establish remote manifest settlement.
Its original immediate leader lookup after reopening failed; the corrected
test explicitly waits for leader agreement before making that assertion.

Four compiled engine faults also fail their exact guarding assertions while the
same test filters pass on the unmodified engine: replacing an unresolved plan,
omitting embedded object validation, reintroducing checkpoint feedback, and
excluding reserved-looking bytes in a user column family. The first runner
attempt correctly reached the malformed-body assertion but expected generic
Rust panic wording; the corrected runner requires that assertion's actual
custom message and exact source line. That failed attempt is retained.

All four affected source-pinned proof compositions pass: local anchor binding
(7 theorems / 26 obligations), frozen base identity (7 / 27), actual publication
(9 / 36), and replicated retention ledger (7 / 55). Their explicit composition
review records old and new source hashes; the new checkpoint-owner proof does
not replace these premises.

The default and `checkpoint-testing` binaries are separately frozen with 166
identical source pins. The default build passed one actual worker/MinIO/Chaos
Mesh leader-kill cell in 124.55 seconds. Its selected manifest's actual
predecessor is generation 71; exact Pending Released / Version Published
observations survive the fault. The helper derives IDs from captured descriptor
bytes and cross-checks predecessor+1 against the recovered process publication.
This run used 270 filler writes to cross the unchanged physical rotation
predicate, within its original 300-write cap. Historical filler counts are not
an acceptance requirement.

The default run does not claim a live observation of the transient acquisition
phase or a pre-upload crash. The separately frozen testing binary is not yet
used for that fault. Its concrete next gate pauses at `owned`, preserves exact
v2 bytes before PUT/GET, and kills that process with actual Chaos Mesh. A node
that rejoins as follower may defer its old pending attempt while another leader
publishes: a typed negative outcome must keep Pending pinned, not be relabeled
as successful Version handoff. The independent history/payload audit and
portable evidence are recorded in [the evidence packet](checkpoint-owners-v1/README.md).

All new logs, proof runs, retained inputs and eventual evidence packets use
`/mnt/data/kv9-work`. Active latency-sensitive fixtures and the reusable Cargo
target remain on NVMe as described in [local output placement](LOCAL-ARTIFACTS.md).
