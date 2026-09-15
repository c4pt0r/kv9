# ADR: retain separate protocol and application recovery ownership

Date: 2026-09-15. Status: accepted direction for current storage increments;
log unification implementation is deferred. Tracks C04 / [#14](https://github.com/c4pt0r/kv9/issues/14).

## Decision

Keep the Raft protocol log and positioned engine WAL separate while delivering
bounded storage, snapshot installation and retention ownership. Preserve the
shared-log target in DESIGN section 6.4 for later multi-group batching. Revisit
physical unification only with a protocol that satisfies the
[recovery and retention contract](RECOVERY-RETENTION-CONTRACT.md), a migration
plan and matched throughput/latency/durability measurements.

This extends the decision already recorded in [SEGMENTED-WAL.md](SEGMENTED-WAL.md)
to configuration snapshots, pending/history ownership and backup retention.
It does not introduce a third authoritative log or a singleton storage service.

## Problem

The logs currently record different facts. `DiskRaftStorage` persists protocol
entries, term/vote, commit and configuration transitions. `WalEngine` persists
applied effects together with their exact `(term,index)`. A durable proposal
is not necessarily committed; a committed command may be rejected by the state
machine's epoch/generation checks. Replaying raw commands as already-applied
effects would conflate those outcomes.

Group commit already batches work; this decision does not propose adding it
again. Engine segmentation already replaces normal tail-copy reclamation with
topology publication and covered closed-file unlink. The protocol log still
requires snapshot/truncation work. Removing one physical write can help, but
two logical authorities do not disappear when they share one file.

## Alternatives

| Choice | Benefit | Cost and required authority |
| --- | --- | --- |
| Retain two logs now | Reuse atomic applied data/position recovery and existing Ready ordering; migrate engine layout independently | Write amplification, two persistence paths and retained protocol history; account for each in measurement |
| One shared physical log with separate logical record kinds | Potentially amortize fsync, framing and multi-group batching | Per-group protocol/apply indexes, atomic group descriptors, common scheduling/backpressure, cross-group pin tracking and corruption isolation |
| Reconstruct all applied state only from the protocol log plus snapshots | May eliminate persisted application deltas | Deterministic replay of accepted and rejected commands, all local/unpositioned state migration, bounded recovery time, complete snapshot/configuration/history authority |

The second alternative remains the architectural target. Neither it nor the
third may acknowledge a write based only on local append, substitute Redis-like
asynchronous replication, or omit the successful quorum/commit/apply fences.

## Required crash ordering

| Boundary | Separate-log rule | Obligation a unified implementation must retain |
| --- | --- | --- |
| Term/vote and Ready | Persist required protocol state before dependent outbound messages | Same durable-before-send relation, including higher term and voting after restart |
| Replication and commit | Preserve the Raft quorum/log-matching authority | A common fsync is not a quorum certificate |
| State-machine apply | Apply committed entries in order; persist effects and exact position atomically | Recovery cannot observe data without its position, or position without its data; rejected/no-op progress remains exact |
| Success receipt | Publish only after the existing successful persistence/commit/apply path | No speculative client success from a durable but uncommitted record |
| Checkpoint / snapshot | Select complete certified state/configuration/history before discarding covered input | Application checkpoint is not an implicit protocol snapshot |
| Segment reclamation | Whole local segments follow selected topology; Raft truncation needs separate snapshot authority | Every group and every reader/pending/transfer/backup owner must release a shared segment before deletion |
| Interrupted publication | Fence ambiguous writer; reopen against a valid complete root | All references and per-group indexes agree with whichever root survives |

The unified design needs both logical log offsets and Raft `(term,index)` per
group. A global maximum byte offset or maximum applied index cannot establish
coverage for a different group. Store-specific unpositioned data also requires
its own migration authority. Retention must account for an idle/pinned group's
old records without silently deleting them to reclaim another group's space.

## Migration conditions

1. Inventory protocol state, indexed membership, optional lease records,
   application effects, local unpositioned records, pending attempts and all
   recovery/retention roots. Define bounded, versioned codecs and old-writer
   refusal before allowing the new writer.
2. Freeze exclusive local ownership or use a separately proved live migration
   protocol. Copy/translate into an unselected layout; retain every source
   needed for either crash outcome. Never allocate new store identities merely
   because a file is missing.
3. Validate per-group entries, commit bounds, exact applied positions, term/vote,
   configuration at snapshot cuts, image hashes and pending/history closure.
   Preserve rejected and unknown operation semantics.
4. Fsync replacement files and indexes, atomically select one complete generation,
   then sync all required namespaces before returning write authority. Fault
   every cut; an ambiguous outcome requires recovery, not continued writes.
5. Retire old files only after durable selection and all independent pins permit
   it. Publish explicit offline-upgrade and downgrade restrictions. Mixed-version
   cluster participation requires the later negotiated capability protocol.

## Evidence needed to revisit the decision

Measure the selected runtime first: bytes and fsyncs per successful write,
protocol/apply persistence latency, lock/queue time, group sizes, recovery
duration, retained bytes and pin debt. Compare same topology, replica count,
storage medium, durability and exact client semantics at low and loaded
concurrency. Report p50/p95/p99 and errors/unknowns alongside throughput.

Accept physical unification only after parameterized safety/progress proofs,
source refinement and actual Chaos Mesh histories cover every voter, interrupted
migration, snapshots, disk errors, partitions and membership handoff. Keep the
object store as the only allowed service dependency exception. Per-node workers
may restart or be replaced without giving one node permanent authority.

This ADR makes no measured speedup claim and changes no current write path.
