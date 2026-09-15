# Replicated retention ledger: tracking-only v1

This increment adds explicit retention registration and ownership transitions to
the existing metadata Raft group. It is a prerequisite of C04/#14 and S07/#21.
It does not close either issue. The ledger tracks registered resources and
owners. Legacy checkpoints/pending references and live readers have not been
completely backfilled or fenced into it. There is no object deletion operation
or complete-reference certificate.
The subsequent [automatic checkpoint-owner increment](CHECKPOINT-OWNERS.md)
now connects the current worker to this ledger before remote I/O. The evidence
below describes the original explicitly registered ledger stage; complete
legacy/reference backfill and reader integration remain open.

## Committed operation path

`RuntimeBackend::apply_retention` requires Serving, holds the existing catalog
planner mutex, commits and observes an ordered Noop, and captures one metadata
view. It validates the request against the certified root in that same view,
plans the complete resource closure, and submits one `CatalogTxn` in the
planning term. A response requires the exact applied `(term,index)` receipt.

The initial Noop drains earlier ambiguous proposals and local application lag.
A ReadIndex alone cannot drain an earlier uncommitted suffix. The term fence
also rejects a plan if its original node becomes leader again in a later term.
The existing [metadata planning argument](METADATA-PLANNING.md) supplies these
ordering premises; this increment adds concrete ledger tests at both boundaries.
The global ledger is hosted by the existing replicated metadata group, without
introducing a separate coordinator or service.

An exact duplicate does not advance the ledger revision. The runtime commits a
**new confirmation Noop** and returns `changed=false` with that new position.
It never reconstructs an earlier invocation's receipt from an owner row. An
unconfirmed outcome is not rollback evidence. The blocking client makes one RPC
per call and does not automatically retry an ambiguous request.

## Identity, storage and limits

The reserved namespace is `\0kv9\0retention_v1\0`, independent of catalog schema
version 1. A checksummed header binds the certified root, ledger revision and
the only supported coverage mode: tracking-only. Explicit initialization is
allowed only when the entire namespace is empty. Missing headers over existing
state refuse; recovery never silently initializes an empty replacement.

Each resource binds root, kind, immutable instance ID and content digest. Its
immutable registration and current checksummed `RetentionRecord` are written in
the same batch. An owner ID binds its kind, scope and epochs, operation and
subject digests, optional node/store/process lifetime and exact sorted resource
closure. Reader records require an explicit local lifetime. These fields are
labels and identity checks; arbitrary supplied epochs or `subject_is_anchor`
do not certify admission, a recovered image or an actual published reference.

Owner registration survives release. The same ID cannot be rebound to a
different descriptor or closure. Reacquisition increments the generation exactly
once and is possible only after release; old-generation publication or release
cannot modify the new generation. Missing registration/state halves, bad checksums,
unknown encodings and inconsistent owner/resource crosslinks refuse.

Limits are 64 resources per closure, 16 KiB per request frame, 8 KiB per owner
record and 8 MiB per atomic batch. Existing resource records retain the 4,096
owner limit including released tombstones. Exhaustion refuses the operation;
it does not evict tombstones. These are per-operation limits, not a proof of a
bounded total ledger size or a replacement for tenant admission/backpressure.

## Transitions

| Request | Committed effect and checks |
| --- | --- |
| Initialize | Explicitly create a tracking-only header in an empty namespace. |
| Register | Register exact resource instances; this alone creates no owner. |
| Acquire | Atomically add the owner's Held state to its entire registered closure. |
| Publish | Mark an exact Held generation Published in every crosslink. This is registration of eligibility, not manifest installation. |
| Share | Add a destination with the exact same subject and whole closure while preserving an eligible source. |
| QuiesceAfterTransfer | Require an exact, distinct Published successor retaining the same subject and closure before quiescing the source. |
| Release | Release only the exact Quiesced generation across all resources; never infer quiescence from time or suspected process death. |

Successor-backed quiescence preserves at least one Published owner for every
registered publication obligation. It does not prove that actual users stopped
using the source. Reader draining, typed attempt settlement and destination
admission need their own capabilities before these records may authorize any
physical cleanup. Final abort/drain release and permanent retirement are not
exposed by this tracking interface.

## Public administration

The authenticated gRPC service adds `ApplyRetention` and `GetRetentionOwner`.
Both use existing metadata admission accounting; malformed identities and
oversized frames refuse at the boundary. Reads establish a fresh Raft barrier
and verify the exact requested root before returning a canonical owner
observation. No response or decoded bytes mint a live-use/deletion capability.

`kv9_server::retention::RetentionClient` accepts typed `LedgerRequest` values or
canonical `KV9RTX01` request bytes. The CLI accepts the same bounded frames:

```sh
KV9_CLIENT_TOKEN=... kv9 client retention-apply \
  --addr 127.0.0.1:20160 --root-digest <64-hex> --request-file request.bin
KV9_CLIENT_TOKEN=... kv9 client retention-owner \
  --addr 127.0.0.1:20160 --root-digest <64-hex> --owner-id <32-hex>
```

A mutation prints `mutation_term/index`; a duplicate prints
`confirmation_term/index`. Readback prints a bounded `KV9OWN01` observation as
hex. The metadata crate checks root, owner identity, checksum, exact lengths,
semantic bounds and canonical reencoding when decoding it.

## Proof scope and source mapping

`proofs/tla/retention_ledger/RetentionLedger.tla` models the committed ownership
state for arbitrary nonempty owner/resource/subject sets and a generation bound.
Owners denote immutable validated descriptors and exact closures. The model
retains per-resource owner generation/phase crosslinks independently of the
owner rows, plus ghost publication obligations and stale-token detection.

| Model | Implementation and premise |
| --- | --- |
| Immutable `LClosure` / `LSubject` | `OwnerBinding::validate`, immutable owner registration, and `Planner::acquire` reject descriptor or closure reuse. Root/codec validation precedes planning. |
| `LSet` / `LPinsAfter` | `Planner::update_resources`, owner state staging, and `LedgerPlan::into_batch` form one atomic `CatalogTxn`; the engine supplies atomic data/position application. |
| `LAcquire` | Initial generation 1 or exact successor generation after Released; existing exact Held/Published retries stutter. |
| `LPublish` | Exact Held generation becomes Published, adding a ghost eligibility obligation. Actual manifest publication is outside the model. |
| `LShare` | An already validated source token remains intact while the destination acquires the identical subject/closure. The source-token check itself composes with the existing per-resource retention proof. |
| `LQuiesce` / `LRelease` | Exact-generation phase checks, whole-subject Published successor before quiescence, and Quiesced-only release. |
| `LSpec` | Ordered committed operations; metadata freshness, Raft consensus, atomic persistence and verified recovery are compositional premises. Unknown requests may eventually take effect or remain absent. |

The seven TLAPS theorems prove initialization, inductive preservation, temporal
invariance, exact crosslinks, successor coverage, stale-token rejection and
source preservation during sharing. The strict runner audits imported modules,
assumptions, theorem inventory, source hashes and absence of proof holes before
checking all 55 obligations. Three finite TLC instances cover complete and
partial closures and distinct subjects. Six actual counterexamples remove
atomic resource updates, Quiesced-only release, generation matching, Published
successors, complete successor closures and subject matching. Proof-hole,
custom-axiom and unsafe-transfer controls must also be rejected.

This is a source-mapped committed-state proof, not a mechanized Rust refinement,
proof of upstream Raft, complete-reference backfill, reader drainage, physical
deletion or unconditional progress. The initial failed drafts and malformed
negative-control draft are retained with their actual outputs.

## Local evidence and remaining integration

New metadata tests cover exact retries, immutable identity, partial validation
failure, missing/corrupt state, transfer cycles, bounded codecs, positioned
atomic writes and real WAL tail cuts. Real three-voter runtime/gRPC tests cover
eight concurrent acquisitions, distinct duplicate confirmations, authentication,
wrong roots, follower refusal, quorum loss, leader stop, all-store reopening,
ambiguous apply drainage and plans spanning two leadership changes.

Local validation covers 726 library tests across the workspace attempt, an exact
six-test environment discriminator and the remaining transaction library; four
existing fixture tests remain ignored. This is not a clean single full-suite
run: the original data-volume TMPDIR attempt retains six runtime election/apply
timeouts. All six pass with the same frozen test executable and four test threads
on the original NVMe fixture filesystem, after bulk tool installation completes.
The failed run records a successful Raft WAL sync as long as 2.542 seconds;
the exact contribution of disk placement versus concurrent installation is not
isolated. Assertions and timeouts were not relaxed. All-target Clippy passes.

The ledger protocol passes seven theorems / 55 strict obligations, all three
finite models, six real model counterexamples and three proof rejections.
Four affected component families also pass with current source pins: retention
10/59, anchor 7/26, base 7/27 and publication 9/36. The separate compiled-source
runner passes nine baseline tests and four single-defect controls, each with an
exact-filter green baseline and the expected guarding assertion: immutable
binding reuse, an unpublished successor, a partial successor closure and skipped
late-resource validation. These controls do not modify the frozen server.

The new default-feature binary completes one actual Chaos Mesh leader kill and
checkpoint recovery, including 16 successful retention operations and the
existing 296-operation serial Raw workload. Before the fault, the source is
Published and the destination Held; after takeover both observations agree
exactly. A duplicate confirms without advancing the ledger revision. Publishing
the destination, quiescing the source and releasing it then complete under the
new leader. This is one leader-failure cell, not the full21 fault matrix or
physical power-loss acceptance. [Exact gates, original failures and evidence](retention-ledger-v1/README.md)
separate the runtime result from its independent audit. No performance promotion
follows from these correctness results.

[Automatic checkpoint ownership](CHECKPOINT-OWNERS.md) now saves exact upload
bytes durably, confirms the Pending owner before external I/O, and transfers to
Version after positive typed settlement. Its scheduling filter prevents ledger
bookkeeping from recursively triggering checkpoints. Negative settlements keep
their owners pinned; certified abort/history release is still open. Complete
reference coverage, bounded aggregate reader guards, ownership transition
history, destination admission and atomic installation remain on the C04/S03/S05
path.
