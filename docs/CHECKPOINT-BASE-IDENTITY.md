# Checkpoint identity at the frozen image

C04 increment, 2026-09-15, tracking [#14](https://github.com/c4pt0r/kv9/issues/14)
under [#9](https://github.com/c4pt0r/kv9/issues/9).

Checkpoint upload now derives its scope from the same immutable image as its
applied position and data. Startup verifies the restored image's historical
identity before replaying any uncovered WAL. A later catalog update cannot hide
an invalid base; a valid old base can still replay into a newer epoch.

Previously, the upload worker read the metadata region epoch and froze the
engine in separate operations. An intervening metadata commit could make the
descriptor disagree with its image. Existing apply-time fencing generally
rejects a stale proposal; this observation does not establish acknowledged
corruption in a previously accepted run.

## Implementation and boundary

`WalEngine::freeze_with_scope` captures the immutable map roots and applied
position under the existing writer lock. It releases the lock before invoking
the scope callback. `CheckpointWorker` supplies a callback that reads the root,
cluster, schema and metadata region from that exact view. Subsequent writes
cannot alter the view or its sealed `FrozenFlush`. Upload remains outside Raft
apply and the WAL lock.

`inspect_initial_checkpoint_base` uses the existing typed catalog decoders on
one borrowed view. It requires all four identity records, the exact certified
root descriptor and cluster, supported schema version, and the initial metadata
region in the system keyspace with empty start/end boundaries and nonzero
configuration/data epochs. The manifest must match the entire resulting scope.
The small backing `MemEngine` supplies only the `MetaTxn` schema interface;
`begin_at` reads exclusively from the provided view and never commits it.
This is an identity check, not an exhaustive foreign-key/catalog-integrity scan.

During both legacy and segmented recovery, the engine verifies remote SSTs and
then passes a snapshot of the restored base to the identity callback. Only a
successful callback permits WAL replay. The enclosing
`open_checkpoint_engine_with_base` still owns the complete engine open and
[winning publication validation](CHECKPOINT-PUBLICATION.md). Any later failure
invalidates the provisional base observation. The runtime requires both base
identity and publication observations before continuing startup, retains the
existing exclusive store guard, and performs the existing final root/store and
committed applied-term checks before Serving.

The current image covers the whole initial engine under `META_REGION_0` (numeric
region ID 1), including its logical Raw KV routing ranges. The metadata region
row identifies that initial owner; it does not turn each routing row into an
independent Raft group. This change adds no disk format, network operation on
client requests, Raft acknowledgment shortcut or service dependency.

## Safety argument

Let `I_c` be the immutable image atomically captured with applied position `c`,
`scope(I_c)` its decoded identity, and `M` the uploaded descriptor. The existing
freeze and SST restore contracts bind the image bytes to `c` and verify remote
content. Under those premises:

1. The producer computes `M.scope = scope(I_c)` using only `I_c`. Interleaved
   writes change the live image but cannot change `I_c`, so descriptor/image
   correspondence is preserved after the writer lock is released.
2. Recovery restores that selected image and checks its root, schema and scope
   before applying the suffix. If the base check fails, the open returns an
   error before the first tail callback. Therefore a suffix that repairs a
   wrong root or advances an epoch cannot retroactively authorize the base.
3. A successful base check is provisional. Serving requires successful complete
   engine recovery and the existing actual winning publication at `p > c`, with
   its matching committed command and final retained history. A callback
   side effect, uploaded object, committed losing proposal or partial replay
   cannot satisfy those conditions.
4. Comparing against `scope(I_c)`, rather than the final catalog, permits a
   legitimate epoch change in the suffix. This establishes the location of the
   identity check; it does not prove a future split/migration ownership chain.

[CheckpointBase.tla](../proofs/tla/checkpoint_base/CheckpointBase.tla) and its
[TLAPS proof](../proofs/tlaps/checkpoint_base/CheckpointBaseProof.tla) check this
protocol with **7 theorem statements / 27 strict obligations**. The proof is
parameterized by an arbitrary positive finite image history. Epoch identifiers
abstract the complete scope tuple; root identifiers abstract canonical root
identity; `CBImageValid` abstracts the typed schema/owner checks. These
abstractions rely on the implementation correspondence and are not a mechanical
Rust refinement or a proof of hash collision resistance.

| Model action | Implementation correspondence |
| --- | --- |
| `CBFreeze`, `CBAdvance` | Atomic immutable image/cut capture with independent later writes |
| `CBMint` | Callback reads the frozen view to derive the descriptor scope |
| `CBRestore`, `CBCheck` | Verified remote restore, then base identity inspection |
| `CBReplay`, `CBFinish` | Uncovered WAL replay and successful complete engine open |
| `CBGrant` | Base observation plus the enclosing committed-publication validator |
| `CBFail` | Any failed restore/check/replay refuses startup |

`CBPublication` is an explicit lower-layer premise supplied by the separately
proved local publication protocol and its implementation tests. The base model
does not independently prove Raft, ordered atomic WAL apply, remote-store
persistence, destination admission, or log retention. The existing publication
proof has also been rechecked against updated source pins; source hashes bind
inputs but do not themselves prove correspondence.

Three bounded TLC instances cover a valid history, a wrong old root followed by
a repaired catalog, and a missing publication. Five mutated protocols produce
counterexamples for using a newer live scope, accepting a later repaired root,
skipping base schema validity, granting before completion, and granting an
unpublished image. The strict proof auditor rejects an omitted proof, a custom
axiom, and an unsafe scope derivation.

## Validation

The local library run passes **636 tests**: engine 138, metadata 29, Raft 262,
and server 207. Four tests requiring external fixtures are reported as ignored
in that run. Two applicable new tests were then explicitly run against real
MinIO and passed. All-target Clippy with warnings denied and formatting pass.

The new tests cover concurrent writes during scope derivation without retaining
the writer lock, historical view stability, all four manifest scope fields,
same-cluster/different-root refusal, each missing identity record, and ten
malformed record/owner variants. The real-MinIO tests exercise both WAL layouts:
base refusal precedes every tail callback, a wrong root cannot be repaired by a
later tail, and a valid old epoch recovers to the newer final epoch. Those
component fixtures deliberately write invalid identity records and use the
low-level adoption seam; they are not claims of a complete serving catalog or
consensus authority. Process and Chaos acceptance use the actual runtime.

The separate three-voter process test also passes on the same binary: physical
checkpoint/WAL reclamation, leader loss and failover, full process restart,
unuploaded durable tail replay, overwrite/delete recovery and fresh writes.
All three voters log successful historical checkpoint recovery on at least two
restarts. Peak observed local allocation is 95,707,136 bytes and the supervisor
confirms all owned processes exited. This is a distinct bounded correctness gate.

The actual Chaos Mesh `PodChaos/container-kill` cell passes with **296 successful
serial operations**, including 280 filler writes that cause physical WAL segment
rotation and reclamation. Voter 3 exits 137 and restarts in a new container with
the same Pod, PVC and durable store identity. Its selected checkpoint is at cut
499, publication 500, generation 210; a fresh write at 501 reaches all three
voters. An independent audit replays every client operation, compares actual
before/after lifecycle command output, checks injection/container identities,
and hashes the selected remote SST bytes. The owned namespace is removed and
eight protected namespaces plus the older fault remain unchanged.

That cell preserves its original preparation baseline across retries. Maximum
measured filesystem decrease is 431,124,480 bytes, within its separate 1 GiB
budget; minimum available space is 9,260,679,168 bytes, above its 8 GiB floor.
This is actual S3 plus single-host process-failure coverage with a memory-backed
object store, not object-store power-loss, cross-host or full21 acceptance.

The default-feature binary is pinned by SHA-256:
`70a61b09c72d0e3e79717ee7f84f7d87c7c77ad6b347013aa8678ad656ce3966`.
The [validation packet](checkpoint-base-v1/README.md) records actual terminals,
inputs and failures, including
the initial invalid memory-uploader test construction, the wrong Cargo package
name used for a binary build, and two incomplete initial proof obligations.
The final proof strengthens its inductive invariant and uses explicit Boolean
initial values; no theorem was removed to make it pass. Two Chaos preparation
failures also remain recorded: a source-map field-name mismatch and Docker
interpreting an image ID as a registry name. Both occurred before fixture or
workload creation. The final helper validates the exact source-map schema and
uses an ID-verified retained image tag; retry accounting retains the original
baseline and exact new binary binding.

## Remaining work

C04 stays open. The next step is a bounded, versioned portable anchor envelope
binding the validated historical image and publication to root, configuration
and range identity. Destination store/install authority remains a separate
capability. Epoch/ownership transition chains, the replicated outer retention
ledger, atomic snapshot installation and safe protocol-log truncation remain
required before dynamic multi-Raft and automatic range splits.

This increment makes no new QPS or Redis parity claim. The complete matched
write screen still needs its original capacity and acceptance gates; local
proofs and small recovery workloads do not substitute for that comparison.
