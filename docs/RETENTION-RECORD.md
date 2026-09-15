# Per-resource retention records

C04 implementation increment, 2026-09-15. Tracks
[#14](https://github.com/c4pt0r/kv9/issues/14) under the
[recovery/retention contract](RECOVERY-RETENTION-CONTRACT.md).

`kv9_common::retention` now implements bounded resource records and deterministic
pin transitions. It is a component for the future replicated ledger. It is not
connected to production manifest apply, snapshot installation or GC, and it
cannot mint a durable pin or physical deletion capability. C04 remains open.

## Record and transition contract

A record binds a root digest, resource kind, independent 128-bit instance and
content digest. The root digest identifies the cluster's exact root. The ledger
must index one instance independently of its content digest and refuse identity
rebinding; changing content is not permission to overwrite an existing instance.
The decoder requires the exact expected identity. Re-uploading identical content
after retirement needs a fresh object instance/key.

Owner IDs are nonzero opaque 128-bit identities. The enclosing ledger must bind
them to complete owner descriptors (kind, operation, scope, applicable
incarnation) and serialize cross-resource publication/settlement. An owner ID
must not be reused for a different operation. A token names that owner and a
nonzero generation; it is a request identity, not authentication or a durable
receipt.

| Transition | Implemented behavior |
| --- | --- |
| Acquire | New owner starts at generation 1; a released owner may advance exactly one generation. An exact retry of a held/published owner is unchanged. Quiesced/released owners cannot be revived by the old generation. |
| Publish | Requires the exact generation already held; repeated publication is unchanged. |
| Quiesce | Moves held/published to quiesced irreversibly within that generation. The caller must first close admission/remove publication and prove use ended or transferred. A paused reader does not satisfy that premise. |
| Release | Requires exact-generation quiescence; repeated release is unchanged. The released slot remains as a generation tombstone. |
| Share | Acquires destination protection while preserving the held/published source. Target validation/capacity failure changes neither owner. Source quiescence/release is a later operation. |
| Retire | Checks the sampled record revision and requires every owner released. Retirement is permanent, including after encode/decode; all subsequent acquisitions refuse. |

Every changed transition increments a checked revision. Overflow refuses without
mutation. Exact retries do not increment it. Once retired, repeated retirement
observes the terminal state even with an older sampled revision; that observation
still does not authorize external DELETE.

The record keeps at most 4,096 owner slots, including tombstones. A new owner at
capacity is refused; no old slot is silently evicted. Existing released slots
can advance generation if revision/generation limits permit. Large-scale owner
history compaction requires a later proof that stale messages cannot revive or
release the compacted identities. This bounded refusal is not a claim of
unlimited owner churn or completed storage-capacity acceptance.

The `KV9PIN01` format has a 102-byte header, sorted 25-byte owner slots and a
32-byte SHA-256 over all preceding bytes. Maximum size is 102,534 bytes. Decode
checks length and version before processing entries; it rejects wrong resource
identity, duplicate/unsorted owners, unknown kinds/phases, zero IDs/generations,
impossible retirement with live pins and inconsistent generation/revision state.
Missing or corrupt persisted state must not call the new-resource constructor.

This codec is for one resource record, not the complete recovery-anchor envelope.
The outer ledger still must bind committed state, pin sets, owner descriptors,
history evidence and the filesystem/snapshot publication protocol.

## Parameterized proof and source mapping

[PinRetention.tla](../proofs/tla/retention/PinRetention.tla) models arbitrary
nonempty owner sets and arbitrary positive generation limits, including the
Rust `u64` boundary. The [TLAPS inventory](../proofs/tlaps/retention/inventory.json)
contains **10 theorem declarations and 59 checked obligations**. The strict runner
pins the actual [Rust source](../crates/common/src/retention.rs), verifies parsed
assumptions/theorem declarations and standard-module hashes, refuses proof holes
and custom axioms, and uses fresh strict/no-fingerprint TLAPS execution.

| Model state/action | Rust correspondence and refinement boundary |
| --- | --- |
| `pinPhase`, `pinGeneration` | Owner map slots; an absent map entry maps to phase `Absent`, generation 0. Stored generations use `NonZeroU64`. |
| `PinAcquire` | `acquire` checks permanent retirement, owner phase and exact next generation before `apply` writes one slot. |
| `PinPublish` | `exact` plus the held-to-published transition. No absent or released owner may publish. |
| `PinQuiesce`, `PinRelease` | Exact-generation phase transitions. Actual view draining / successful settlement is an explicit external premise; the model does not manufacture it. |
| `PinShare` | Source exact-generation eligibility plus destination acquire; source stays unchanged. Distinct owners prevent a self-handoff. |
| `PinRetire` | No unreleased slots and permanent retirement. Revision equality and overflow checks impose additional refusals. |
| Stuttering | Exact retries, invalid requests, exhausted capacity/revisions and other refusals leave logical state unchanged. All fallible checks precede mutation. |
| `PinDelete` | Ghost external execution, deliberately absent from the Rust API. The eventual ledger/executor must justify durable retirement before this action. |
| Fixed resource | The immutable identity and bounded codec; identity/checksum validation is tested separately and is not mechanically proved by TLAPS. |

`PinInvariantInit`, `PinInvariantStep` and `PinInvariantAlways` establish type,
generation/phase binding, absence of owners on retired instances, deletion's
retirement prerequisite and exact release identity by induction. The other
theorems establish permanent retirement, acquire-before-publication, sharing
overlap, generation monotonicity, generation-fenced release and unreferenced
deleted instances. `PinRetiredDeletionProgress` proves eventual ghost execution
only with its explicit weak-fairness premise after retirement. It does not
promise owner release, quorum availability or object-store completion.

The proof omits byte-level codec execution, allocator/process failure, local
file/namespace persistence, Raft commit, complete anchors, cross-resource atomic
publication and actual reader quiescence. The enclosing durable ledger must
compose those obligations before production use. Source hashing and compiled
fault controls strengthen the correspondence; they are not a compiler/Rust
refinement proof.

## Actual local checks

- `cargo test --offline -p kv9-common`: **36 tests pass**, including seven new
  retention tests; no ignored tests. Those seven cover publication/retirement,
  stale messages, sharing, exhaustion, stale sampled revision, every partial
  byte cut/every single-byte corruption of a sample, and valid-checksum semantic
  corruption. A maximum-capacity record also round-trips.
- Common all-target Clippy with warnings denied, workspace formatting and diff
  checks pass. These are scoped component checks, not a new workspace/server gate.
- Actual TLC checks: two owners / two generations explore **229 generated,
  111 distinct states**; three owners / two generations explore **3,352 generated,
  963 distinct states**. Both complete their conditional temporal check.
- Five actual model counterexamples: reacquire a retired instance, publish
  without an owner, release a stale generation, retire a pinned instance, and
  omit the fairness needed for eventual delete execution.
- Three proof controls reject: a parsed omitted proof, a custom false axiom,
  and an actual failed strict proof after removing the retirement owner guard.
- Four isolated Rust source faults compile and fail their exact selected tests:
  omit retirement fencing, omit generation matching, release the source before
  destination validation, and retire despite outstanding owners. The standalone
  unchanged-source harness first passes all seven retention tests. Its test
  population overlaps the 36 common tests; do not add them together.

The first proof draft failed one of 59 obligations because the temporal proof
did not unfold `PinSpec` at the fairness step. The corrected proof adds that
definition; it does not weaken the model, invariant or fairness premise. Both
drafts and the failure remain retained alongside the final gate. Existing
performance and Chaos cohorts were not rerun.

The [portable evidence](retention-record-v1/evidence.tar.gz) contains the copied
proof/model inputs, semantic audits, positive/negative outputs, source-control
compilation/test logs and original failed proof draft. Its
[member inventory](retention-record-v1/inventory.json) records 111 files and
SHA-256 hashes. Build binaries and model-state caches remain local. The packet
covers the component checks described above, not a new server/E2E qualification.

Reproduce the component and proof checks locally:

```sh
CARGO_TARGET_DIR=/path/to/new-target CARGO_INCREMENTAL=0 \
  CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
  cargo test --offline -p kv9-common
python3 scripts/check-retention-protocol.py \
  --jar /path/to/tla2tools-v1.7.4.jar \
  --tlapm /path/to/tlapm --output /path/to/new-proof-results
python3 scripts/check-retention-source-controls.py \
  --deps /path/to/new-target/debug/deps --output /path/to/new-source-results
```

The source-control harness compiles copied module bytes against the retained
common/sha2/thiserror dependencies; it records their hashes and compiler identity.
It makes no change to the worktree and accepts only compilation success followed
by the named assertion failure. An arbitrary compiler failure cannot pass it.

## Remaining integration

Next implement complete recovery-anchor validation and the outer replicated
owner ledger. Persist pin acquisition before references become visible; persist
successor ownership/settlement before releasing predecessors. Preserve pending
winner/effect facts across snapshots, and bind configuration, publication and
image cuts independently. Do not wire the boolean retired state directly to
MinIO DELETE.

Actual Chaos Mesh scenarios remain required once the ledger is connected to
manifest/snapshot/recovery paths: every-voter takeover, interrupted ownership
transfer, pending recovery after history advancement, disk errors and delayed
physical deletion across leadership changes. This pure component starts no
service, adds no singleton, and grants no new production deletion authority.
No C04 or downstream checkbox closes from this increment. The full write
performance comparison retains its original capacity and workload requirements.
