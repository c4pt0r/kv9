# Durable protocol snapshots: S05 / D03 prerequisite

Checkpoint: 2026-09-16. Tracks [#19](https://github.com/c4pt0r/kv9/issues/19)
and [#24](https://github.com/c4pt0r/kv9/issues/24), under [#9](https://github.com/c4pt0r/kv9/issues/9).

## Behavior

`DiskRaftStorage::install_protocol_snapshot` now stores a bounded Raft snapshot
and its exact HardState in one checksummed append-log record. Recovery selects
that complete protocol tuple, including the snapshot cut/term, full joint
configuration, payload, and persistent election term/vote. Later entries resume
after the snapshot boundary. The old physical log is retained; no bytes are
reclaimed by this increment.

This is the storage half of installation. It does not download an engine image,
authorize a new store, certify source publication or pin remote objects. No
production network path invokes it yet. Peer construction deliberately refuses
a snapshot-backed store until the coordinated engine installation journal is
connected. The next step must compose this primitive with that journal and
validated source/destination authority before enabling learner attachment.

The review also found two existing gaps relevant to that integration:

- Ordinary MsgSnapshot reached raft-rs even though kv9's Ready consumer did not
  install snapshot data into the engine. Receive now drops it before raft-rs can
  alter commitment or membership. A separate Ready guard fences any unexpected
  uncoordinated snapshot. No snapshot acknowledgment is sent by this path.
- Delegating DiskRaftStorage snapshot generation to MemStorage could synthesize
  an empty image at the current commit or relabel the requested cut. Disk storage
  now returns only the exact persisted image at its actual cut, or
  SnapshotTemporarilyUnavailable. A newer commit does not manufacture an image.

Existing full-log replication and Safe ReadIndex continue to use their existing
paths. There is no new public API, learner promotion shortcut or relaxed fsync.

## Record and recovery contract

Outer append-log kind **6** contains `KV9RSN01`, a length-prefixed protobuf
Snapshot and a length-prefixed HardState. The existing outer checksum covers
the entire pair and both lengths. Snapshot protobuf bytes are bounded to 2 MiB;
HardState bytes to 64. Every configuration set has at most 1,024 nonzero unique
node IDs. Full incoming/outgoing voters, learners, next learners and auto-leave
are retained; impossible overlaps/joint states refuse. Empty image payloads,
unknown protobuf fields and noncanonical record encoding refuse.

The snapshot has a nonzero cut and term; its index cannot be the maximum u64.
HardState commit must equal the cut and its term must cover the snapshot term.
An installation must advance the prior durable commit, cannot lower the durable
election term, and must preserve a nonzero existing vote in the same term. The
snapshot term cannot predate the prior committed term. Source/root/range/image
authority remains an upper-layer obligation; these structural checks alone
cannot authenticate a fabricated image.

An exact retry with unchanged HardState returns without I/O. A conflicting image
at the same cut or a stale retry after later protocol progress refuses. Validation
precedes append. Under the exclusive writer lock, the complete record is written
and synchronized before publishing memory state. Any write/sync failure disables
the writer until reopen; it cannot resume appending behind uncertain bytes.

Reopen validates both structure and transition before selecting a frame. A torn
final frame selects the preceding state; a checksum-valid invalid record refuses
recovery without truncating away the evidence. Valid surviving unsynchronized
bytes are synchronized by the existing recovery path before returning the store.
The configuration replay guard advances to the snapshot cut, and historical
configuration lookup explicitly reports compacted history rather than inventing
a configuration-change position. Fixed-configuration lease state and protocol
snapshot bases cannot be combined in either order.

Older builds reject the new record kind. Existing stores receive no kind-6
record through today's runtime. This is not mixed-version rolling-install or
downgrade acceptance. Payload integrity/authentication, durable object ownership,
engine installation and physical log-generation retirement need their own gates.

## Validation and limits

The complete local workspace run passes **904 tests/doctests**, with 28 existing
ignored tests. Nine new tests cover protocol replay and later tail entries,
immutable snapshot replies, idempotence and stale retries, invalid configurations
and election rollback, the receive/startup guards, real-file reopen, every byte
boundary of a torn frame, checksum-valid invalid records, and lease exclusion.
The actual persistence implementation runs through **180 fault/crash cases**:
before/after write and sync, short writes, EIO/ENOSPC, complete loss/retention and
16 seeded partial-retention schedules. Successful synchronization must survive;
failed attempts may recover old or new, but no mixed tuple or premature memory
publication is accepted.

A subsequent test-fixture initializer cleanup for Clippy changes no production
behavior; all nine focused tests and strict all-target Clippy pass afterward.
Formatting and diff checks pass. Three actual Rust mutations are rejected at
named assertions: stepping a snapshot before installation, synthesizing an
empty image, and forgetting an existing same-term vote. Original compiler/proof
and Clippy failures remain in the evidence record.

The [checked abstract model](../proofs/lean/protocol-snapshot/README.md) proves
**11 theorem statements**, with nine semantic mutations and two proof-policy
controls. Preparation and fixed-voter activation proofs are rechecked with
reviewed source correspondence. This is not verified Rust extraction, a complete
Raft proof or a remote-installation proof. Atomic framing, filesystem durability,
exclusive ownership and upper-layer image authority remain explicit premises.

The single actual Chaos Mesh campaign on the current default-feature debug
binary passes all four existing scoped-client fault windows: discovery-seed
isolation, data-leader container kill, two-way leader partition and client-only
data-leader isolation. Two persistent clients record **46 calls: 43 successes,
one UnknownWrite and two lookup failures**. The complete harness and separate
final reader exit 0; post-fault readback, three stopped-store archives and exact
namespace cleanup pass. Eight historical namespaces and their fault identities
are preserved. Two initial build-wrapper selection errors happened before
compilation/runtime and remain recorded; the accepted third build checks actual
first-party compiler artifact features and unchanged source before/after build.

The [portable validation packet](protocol-snapshot-v1/README.md) retains logs,
proof controls, source/binary bindings, complete fault histories, original
failures and cleanup evidence. This is a single-host routing/failover regression
under the new guards, not remote snapshot-installation Chaos acceptance. The
serial point-history reader does not certify arbitrary concurrent operations or
atomic-batch faults. No snapshot install is enabled by the campaign.

## Next mainline

1. Build the destination-bound engine installation journal: validate a sealed
   source bundle and target incarnation, retain source/transfer pins, prepare
   the engine generation, and durably select matching engine/protocol state.
2. Connect Raft snapshot Ready to that installer with exact apply progress and
   recovery; carry configuration-at-cut and pending reconciliation evidence.
3. Qualify empty-target attachment after source-log truncation, learner catchup,
   safe promotion and old-replica removal, including coordinator loss and actual
   Chaos Mesh faults during installation and retention transfer.
4. Implement durable manual/automatic split and online placement, then execute
   the [independent 3/6/9-host benchmark](HORIZONTAL-SCALING-PLAN.md).

S05 and D03 remain open. No new QPS, Redis comparison or measured scaling result
is claimed. Daily CI remains local; no hosted workflow is dispatched here.
