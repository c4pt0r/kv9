# D01b: replicated creation intents and durable group preparation

This increment connects metadata Raft to independently prepared data-group
stores and restart discovery. It follows [shared group transport](MULTI-RAFT-TRANSPORT.md)
and advances [#22](https://github.com/c4pt0r/kv9/issues/22). It publishes no range
and starts no data-group voter. The [horizontal-scaling benchmark contract](HORIZONTAL-SCALING-PLAN.md)
remains unchanged; there are no new QPS or scaling results.

## Behavior and authority

The embedded `NodeRuntime::create_data_group_intent(operation, voters)` API
uses the existing metadata planner lock, ordered barrier and same-term Raft
commit. It reserves a task ID and region ID in one transaction and stores an
immutable task, with a nonzero caller-supplied operation ID for retries. The
descriptor binds the certified root and the exact initial node/store
incarnations. A retry on a replacement metadata leader returns the same IDs;
changed members or disk incarnations cannot reuse the operation ID.

Creation uses the existing `tasks` table, stable kind 100 and a versioned binary
payload. There is no new table/schema migration and no `regions` routing row.
The first implementation admits 3, 5 or 7 distinct active registered voters and
at most 255 task rows in its bounded planning scan. That conservative shared
table limit needs indexed/paged reconciliation before larger task populations.

`NodeRuntime::prepare_data_group(task)` consumes a separate immutable capability
read from the locally applied metadata engine. The caller cannot turn a planned
overlay or decoded descriptor into that capability. A follower may prepare
after applying the intent; current leader authority is not needed for this
immutable, non-serving operation. Missing local apply is a typed refusal.

Each local group owns `data-groups/<region>/`, its lock, a checksummed identity
record, Raft log and segmented engine WAL. Preparation synchronizes the intent
before opening storage, then synchronizes StorageReady after both stores pass
validation. A Ready record always selects recovery-only Raft open. Missing or
empty Raft logs, missing engine topology, foreign membership, voting history,
user data, checkpoints or applied history refuse an unstarted preparation.
This prevents adopting another group or silently minting fresh voters.

Startup discovers previously prepared directories and validates them against
committed metadata. It can resume a preparation interrupted before its first
record only when the exact metadata intent exists and the directory contains
no orphaned data. A corrupt recognized group becomes a failed local slot while
independent groups continue recovering. A failure during publication poisons
that group until restart; it cannot issue a readiness observation by retrying
against merely visible state. Parent store and child group locks remain held
for their owners' full lifetimes.

The APIs are embedded control-plane methods, not new unauthenticated RPCs or
CLI commands. `GroupPreparation` reports local storage readiness only. It grants
no serving, routing, membership, voting, snapshot-install or deletion authority.

## Verification

Nine focused tests cover immutable/committed readback, retry allocation,
replacement identities, codec/row binding, local ownership, sixteen before/after
publication cuts, absent/empty/orphaned logs, foreign configuration/voting/data,
per-group recovery isolation and actual metadata quorum integration.

The integration test runs three complete NodeRuntime instances with real
loopback gRPC, durable metadata Raft and engine WALs. Two intents commit and
prepare on all three stores. After dropping the metadata leader, the replacement
confirms the same operation/IDs; restarting the original store discovers both
independent preparations. No new public routing rows are visible.

The [Lean protocol component](../proofs/lean/group-preparation/README.md) proves
nine statements over arbitrary reachable preparation histories. Its explicit
refinement premises include the existing metadata committed-prefix and local
filesystem/WAL contracts. It is not a complete lifecycle or Rust extraction
proof. Source hashes, axiom checks and rejecting model controls accompany it.

Retained checks, original failed attempts and control sources are under
`/mnt/data/kv9-work/group-preparation-20260916`. The [published validation packet](group-preparation-v1/README.md) records
**865 passing workspace tests/doctests, 28 existing ignored**, nine rejecting
Rust controls, strict Clippy/formatting and exact source/toolchain bindings.

The fault cuts inject errors before/after real filesystem operations. The
leader-loss integration is an ordinary in-process runtime restart, **not actual
Chaos Mesh or physical-host-loss acceptance**. No new such acceptance is claimed.
The complete #22 issue and industrial gates remain open; GitHub CI is not run.

## Next implementation

Complete the activation protocol: capability negotiation before any data-group
message can reach an old V2 receiver, a durable one-way Started marker before
Raft can vote/send, recovery of active per-group logs and bounded shared workers
with metadata resource reservation. Then connect public range/epoch routing,
replica migration and recoverable split. Retirement must drain local handles
and retain identity/tombstone authority before resources can be reclaimed.

The current manager deliberately has no transition that runs a prepared group.
Old released V2 receivers ignored the group field; enabling data-group traffic
without the capability/activation work would be unsafe. This checkpoint does
not authorize that shortcut or mark the full RegionManager complete.
