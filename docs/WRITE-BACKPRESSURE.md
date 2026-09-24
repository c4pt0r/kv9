# End-to-end write backpressure (bounded absolute log size)

Added 2026-09-24. Parent [#9](https://github.com/c4pt0r/kv9/issues/9),
P1 storage [#20](https://github.com/c4pt0r/kv9/issues/20). Compaction
([entries](../docs/AUTO-COMPACTION.md)/[bytes](../docs/BYTE-COMPACTION.md)/[age](../docs/AGE-COMPACTION.md))
and [physical reclamation](../docs/LOG-RECLAMATION.md) bound the log *after the
fact* — they advance `first_index` and shrink the file once the retained tail is
large. But nothing stops a client from writing FASTER than commit and compaction
can drain: the retained committed log (`raft_committed - log_first_index`) can
still grow without limit between drains. This closes that gap with an ABSOLUTE
bound and a typed refusal — the last core item of #20.

## What this increment adds

1. **`KV9_MAX_RAFT_LOG_ENTRIES`** (0 = off, the default): the absolute number of
   retained committed raft-log entries a data group will hold before it stops
   accepting new writes. When a group's `raft_committed - log_first_index`
   reaches this bound, the group is at capacity until compaction drains it.

2. **A pre-append, write-only gate** (`RawGroup::permit` in
   `crates/server/src/runtime/range_api.rs`). Every write path
   (`prepare_raw_write`, `raw_put`, `raw_batch_put`) passes through `permit`;
   reads take `view` and are NEVER gated. When the bound is set and the retained
   log has reached it, `permit` returns BEFORE anything is proposed — nothing is
   appended, nothing is queued, no raft round-trip is spent.

3. **A typed, retryable refusal** (`Error::WriteBackpressure { region }` in
   `crates/common/src/error.rs`), mapped to gRPC `RESOURCE_EXHAUSTED` in
   `crates/server/src/grpc.rs`. It is deliberately distinct from `StaleEpoch`
   (which asks the client to refresh routing) and from a transport error (which
   reads as "unknown"): backpressure means the write definitively did NOT happen
   and the correct client reaction is to back off and retry the SAME route. The
   bound releases the moment compaction drains the retained log below it.

The retained log therefore never exceeds the bound: a write is admitted only
while STRICTLY below it, and every entry adds at most one, so the ceiling is the
bound itself. Reads are unaffected, and the gate is a pure no-op when the bound
is 0 (the default), so existing deployments are unchanged.

## Known limits, deliberately out of scope

The gate bounds RETAINED committed entries, the quantity compaction drains; it is
not a byte cap and not an admission-plane (per-node) limit — those are the
[metadata admission floor](../docs/ADMISSION-FLOOR.md) and the compaction/reclaim
triggers, which compose with it. It is per-group and local to the leader's
`permit`. No chaos campaign and no performance claim: this is a correctness gate
whose steady-state cost is one status read per write when the bound is enabled,
and zero when it is not.

## Verification

- Machine-checked Lean model (`proofs/lean/write-backpressure/`): ten theorems —
  the retained log never exceeds the bound, writes are admitted only below it, no
  write grows the log at the bound, the refusal is typed and changes nothing,
  reads serve at any retention, draining always releases the bound, and the bound
  is reachable (non-vacuous) — with five semantic defect controls and two
  proof-policy controls.
- Real-process five-node MinIO e2e (`scripts/write-backpressure-e2e.py`): with
  auto-compaction off so nothing drains the log, sustained writes fill the
  retained log to the bound; the next write is refused with typed
  `RESOURCE_EXHAUSTED`; the log stays bounded under continued refused writes;
  reads still serve; and a committed compaction floor drains it so writes resume.
