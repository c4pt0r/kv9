# Offline joint engine/protocol installation

Updated 2026-09-16. S05 [#19](https://github.com/c4pt0r/kv9/issues/19) and
D03 [#24](https://github.com/c4pt0r/kv9/issues/24), following `5417f8c`.

This increment prepares and recovers an **offline immutable data-group image**
at an exact destination. It installs the engine and protocol base together on
local storage, but returns diagnostics only. It does not admit a remote sender,
start a Raft peer, transfer retention owners or enable network snapshots. The
existing snapshot receive/startup guards stay in place.

## Storage and recovery

`JointInstaller` borrows an actual exclusive `StoreGuard` and acquires the same
`data-groups/<region>/group-lock` as RegionManager. It validates the bound root,
node and store incarnation, full DataRange including tenant, keyspace, creation,
epochs, bounds and sealing state, manifest scope/cut and target membership.
The source description is canonical and bounded; decoding it is not authority.

Only an empty group or this installer's existing format is accepted. Existing
flat active/prepared storage is refused. Before creating a generation, the
installer durably publishes `group-record` with the `KV9INS01` format, exact
identity/range and checksum. Existing RegionManager readers reject this format;
there is no fallback to flat WALs. Integrating an active group needs a separate
validated lifecycle transition.

Each attempt has its own immutable `install-<generation>` directory:

1. Persist the canonical snapshot/HardState description. Create a fresh empty
   engine WAL and canonical checkpoint reference. Restore every SST through the
   actual MinIO client, verify object digests and metadata, and check the exact
   DataRange plus every key's Raw namespace/range and column family.
2. Persist the protocol snapshot/HardState using the previous checkpoint's
   checksummed durable record. Its cut, image term, complete ConfState and election
   term/vote must match the selected description.
3. Seal hashes of the engine WAL, checkpoint and protocol log. Sync files and
   directories. Reopen and independently validate the complete pair before
   publication. A replaced selection must advance the committed cut without
   lowering the election term or changing a nonzero vote in the same term.
4. Write and fsync a fresh selector temporary binding the destination, generation
   and image digest. Rename it over `installed-generation`, then fsync the parent
   directory before returning success.

A failure after mutation fences the live installer until reopen. Before rename,
recovery selects the previous complete pair, or reports pending for an initially
empty target. Between rename and directory sync, filesystem crash semantics allow
either complete selection. A successful directory sync retains the new selection.
Recovery resyncs a visible selector, verifies exact selected files and objects,
and refuses missing/corrupt files instead of initializing them or choosing an old
image. Exact retries reuse the selected generation and perform no new publication.

All previous and partial generations remain retained. Eight generations per group
bound unfinished/replaced attempts; exhausting that budget is an explicit refusal.
No automatic cleanup, cancellation, source-pin release or physical log reclamation
is enabled. Immutable generations are not yet writable runtime storage.

## Image and resource limits

The data-image payload is `KV9DIMG1`, canonical DataRange plus canonical checkpoint
manifest, enclosed in the existing bounded protocol snapshot record. Its full
image cut equals the Snapshot metadata cut. The protocol cap remains 2 MiB; the
engine checkpoint remains at most 48 MiB of SST bytes. The selected record binds
all fields, rather than accepting a caller-supplied checkpoint path.

SST recovery is **eager and fully resident**, not on-demand or lightweight attach.
Installation currently restores twice (preparation and final verification), and
recovery restores again. `object_bytes` is the selected manifest's logical SST
size, **not measured network traffic**. The test probe additionally reopens the
engine to check every fixture value. No transfer-throughput claim follows.

A validation test caught the fact that an empty scan end is not positive infinity
in the engine API. Validation now streams each verified SST's actual inclusive
key bounds and checks its count; tests cover long key streams and foreign rows.

## Checked model

[Installation.lean](../proofs/lean/joint-install/Installation.lean) proves the
finite-trace ordering projection with separate engine/protocol durability flags,
sealed generation, visible/durable selector, success receipt and failed owner.
It also checks whole-pair recovery, exact destination, corrupt-selection refusal,
and election term/vote preservation. Filesystem sync/rename semantics, exclusive
ownership, immutable files and verified images are explicit premises. This is
not extracted/verified Rust or a complete Raft, migration or source-authority proof.

The runner checks 13 theorems, audits axiom dependencies and requires 10 semantic
mutations plus two proof-policy controls to fail. Existing protocol-snapshot,
group-preparation, group-activation and data-range models are rechecked after
reviewing the read-only directory accessor and internal codec visibility changes.

## Validation

The [portable validation packet](joint-install-v1/README.md) records all accepted
checks and retained failed attempts, including exact source and binary hashes.
Local Rust tests cover store/group locking and incarnation binding, refusing flat
history, exact ownership/key validation, real-MinIO installation cuts, missing or
corrupt selected files, term/vote/cut refusal, object loss/corruption, retained
attempt bounds and idempotent retries. Three isolated Rust defect controls check
that removing owner poisoning, same-term vote protection or key-range validation
fails the intended assertion.

Actual Chaos Mesh acceptance is scoped to the offline installer component. Its
test-only executable never launches a peer. Process kills cannot establish
hardware power-loss behavior, quorum availability, network snapshot installation,
concurrent linearizability or effective horizontal scaling.

## Next mainline work

1. Add a committed migration intent and exact source/destination retention owners.
   A decoded manifest or this local receipt must not authorize voting or source
   pin release. Bind owner subject and complete object closure to the same image.
2. Capture source engine state, historical ConfState and a unified applied cut
   under the driver owner. Engine command position can lag no-op/configuration
   positions; do not attach a later configuration to an older image by assumption.
3. Integrate snapshot Ready coordination and a private validated startup capability
   with RegionManager, preserving the group lock and driver applied position.
   Specify how immutable selected generations become appendable runtime storage
   without making startup hash checks reject legitimate tails or accept lost tails.
4. Catch up a genuinely empty learner after source log truncation, promote from
   actual durable application/configuration evidence, remove the old replica,
   recover across leader/coordinator loss, then implement split and placement.

The [3/6/9-host benchmark contract](HORIZONTAL-SCALING-PLAN.md) remains the
acceptance criterion for effective horizontal scaling: fixed three voters,
physical durability, unchanged p99 budgets, resource accounting and online
expansion. There are no new QPS or measured scale-out results in this checkpoint.
