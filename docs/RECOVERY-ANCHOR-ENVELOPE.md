# Initial recovery anchor envelope

C04 increment, 2026-09-15, tracking [#14](https://github.com/c4pt0r/kv9/issues/14)
under [#9](https://github.com/c4pt0r/kv9/issues/9).

Startup now assembles one bounded, versioned recovery description from the
actual restored checkpoint and its validated committed publication. It binds
the certified root, historical owner epoch, image cut, publication, full
configuration at the cut, and exact canonical SST manifest. A decoded description
cannot substitute for a successful local recovery.

This is a prerequisite for snapshot transfer and retention ownership. It does
not yet implement a transferable recovery bundle, destination installation,
protocol-log truncation or a replicated retention ledger. C04 remains open.

## Runtime and authority

`kv9_meta::recovery::open_initial_checkpoint_engine` owns one complete engine
open. It obtains the [historical base identity](CHECKPOINT-BASE-IDENTITY.md)
before uncovered WAL replay and the [actual winning publication](CHECKPOINT-PUBLICATION.md)
only after successful complete replay and retained-history validation. It then
requires both observations, checks their image/configuration/root correspondence,
and produces `LocalRecoveryAnchor`, whose fields and constructor are private.
Ordinary recovery without a selected remote checkpoint produces no anchor.

The public `RecoveryAnchor` descriptor is plain data. Its fields may be changed,
and even a correctly checksummed fabricated publication can decode. Only the
complete local open can mint the separate local observation. Neither type grants
permission to install on a different store, delete objects or discard protocol
history. The existing store guard, durable root admission and final committed
applied-term checks still precede Serving.

Three historical positions remain distinct: image cut `c`, latest indexed
configuration publication `k <= c` (or the exact certified initial configuration),
and actual winning manifest publication `p > c`. The local replay endpoint
`r >= p` is recorded only in the local observation. It is excluded from the wire
identity: two replicas recovering the same image and publication must not get
different anchor identities merely because one has replayed a longer suffix.

The initial image covers the whole physical engine under `META_REGION_0`
(numeric ID 1), with empty physical range boundaries across keyspaces. Logical
Raw KV routing rows are not independent Raft groups. A future per-range format
must explicitly add the required ownership and transition authority.

## Version 1 wire format

All integers are unsigned big-endian. Lengths/counts are `u32`; positions contain
`u64 term, u64 index`; tags accept only bytes 0 and 1.

| Field, in order | Size / bound |
| --- | --- |
| Magic `KV9ANCH\0`, version 1, required capabilities, body length | 8 + 2 + 8 + 4 bytes |
| Canonical root length, root bytes, root SHA-256 | 4 + at most 65,536 + 32 bytes |
| Owner configuration epoch and data epoch | Two nonzero `u64` values |
| Image cut `c`, actual publication `p` | Two positions; nonzero terms/indices, `p.index > c.index`, monotone terms |
| Manifest generation and change identity | Nonzero `u64` + 32 bytes |
| Configuration publication present, optional `k` | 1 + zero or 16 bytes |
| Incoming voters, outgoing voters, learners, next learners | Four count-prefixed lists, at most 1,024 `u64` IDs each |
| Joint configuration auto-leave | 1 byte |
| Canonical manifest length, manifest bytes, manifest SHA-256 | 4 + at most 1,048,576 + 32 bytes |
| Frame SHA-256 | 32 bytes over all preceding bytes |

Required capabilities must equal `0b111`: initial whole-engine scope, required
retained protocol history, and externally unbound retention. Missing or unknown
bits refuse version 1. This version requires local protocol history from index
1 through publication and the existing recovery history checks; it cannot act
as a self-contained substitute after protocol compaction.

The outer decoder refuses frames over 2 MiB and validates header, exact body
length and checksum before allocating variable fields. It bounds nested lengths
and counts before their allocations, consumes every byte, and requires canonical
root encoding. Membership lists must be strictly increasing, nonzero and preserve
all four Raft configuration sets plus auto-leave. Overlap constraints preserve
joint-consensus learner semantics; duplicate IDs are rejected, never silently
deduplicated. Unknown protobuf configuration fields cannot be projected into v1.

The metadata decoder additionally requires byte-for-byte canonical manifest
re-encoding, matching root/cluster/region/epochs/cut, and the exact change identity
derived from the manifest and predecessor generation. A valid outer checksum
cannot hide a conflicting nested identity or an ignored future JSON field.
Checksums establish content integrity under their assumptions, not authentication.

The field bounds imply a conservative maximum of **1,147,128 bytes**:
`232 + root_bytes + manifest_bytes + optional_position_bytes + 8 * total_members`.
The 232 fixed bytes include all headers, length/count fields, digests, epochs,
image/publication positions, generation, change identity and tags. The root's own
structural bounds can reduce the realizable maximum further.

## Proof and validation

[AnchorBinding.tla](../proofs/tla/anchor_binding/AnchorBinding.tla) models one
selected immutable descriptor, base inspection, completed publication recovery,
local observation construction and independent public decoding. Its
[strict TLAPS inventory](../proofs/tlaps/anchor_binding/inventory.json) proves
**seven statements / 26 obligations**, including same-image binding, completion
before construction, decoding's inability to mint a local observation, and the
encoded-size bound. Five TLC configurations explore 45, 30, 36, 36 and 28 states.
Seven unsafe model mutations produce actual counterexamples; omitted proof,
custom axiom, premature construction and a false size bound are rejected.

The model's root/configuration/publication predicates explicitly depend on the
existing lower-level recovery protocols. Its frame-validity predicate abstracts
the bounded canonical codec and nested checks. Source pins connect the reviewed
implementation; this is not mechanical Rust refinement, a byte-parser proof,
hash-collision proof, installation proof or ownership-transition-chain proof.
Configuration (11 / 95), base identity (7 / 27) and publication (9 / 36) protocols
also pass with their current composition inputs. No theorem was removed after
the initial arithmetic proof attempts failed; grouping the equivalent fixed
header arithmetic made the SMT obligation tractable.

Local checks pass **684 library tests** (four existing ignored fixture tests
reported separately), the private-constructor compile-fail example, all-target
Clippy with warnings denied, and the default binary build. Tests cover joint
configuration, initial membership, every frame truncation, one-bit-per-byte
corruption, valid-checksum unsupported capabilities, oversized nested counts,
conflicting nested identities and noncanonical manifests.

The new default binary also passes three-voter MinIO/process recovery and actual
Chaos Mesh `PodChaos/container-kill`. The killed voter recovers generation 211,
image cut 500, publication 501 and a logged anchor identity on the same Pod,
PVC and store. All three process voters log the new anchor across complete
restarts. These are actual S3 and single-host process-failure gates with a
memory-backed object store; they do not prove object-store power-loss durability,
general concurrent linearizability or full21 acceptance.

[Commands, source bindings, failures and retained evidence](recovery-anchor-v1/README.md)
keep the protocol and environment results independently inspectable. There is
no new QPS result, write-candidate promotion or hosted CI run in this increment.

## Next development boundary

1. Implement the replicated outer retention ledger over the existing bounded
   per-resource records. Define deterministic acquire/transfer/release/restart
   commands, owner-generation fencing, and durable bindings to exact anchor
   identities. Preserve unknown outcomes and forbid deletion until complete
   owner enumeration and object-instance retirement are implemented.
2. Add ownership/epoch transition evidence and a sealed transferable closure of
   image, configuration and retained protocol history. Validate remote receivers
   without treating descriptor decoding as authority.
3. Implement destination admission and crash-recoverable atomic installation;
   prove selection and retention composition, then inject failures at each
   publication/install/truncation boundary with Chaos Mesh.
4. Proceed to dynamic multi-Raft, membership/routing and automatic range splits
   with those ownership and recovery prerequisites in place. The complete matched
   write-performance comparison remains a separate capacity-dependent gate.
