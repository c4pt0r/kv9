# Deferred engine-apply sync (#20): the measured lever, taken

Updated 2026-09-17. Parent [#9](https://github.com/c4pt0r/kv9/issues/9),
issue [#20](https://github.com/c4pt0r/kv9/issues/20). The follow-up the
proposal-batching increment's finding demanded: measure where write
latency actually goes, then take THAT lever.

## The measurement chain (all runs retained)

A write-stage-traced fixture located the cost: apply groups already
coalesce (~25 commands/entry via the existing raw-prefix grouping), and
each group pays TWO serial fsyncs on an 8.4ms-median-fsync disk — the
raft-log sync, then the engine-apply record sync. Three fixture errors
were found and published on the way: the node metrics export attributes
`engine_wal_record_sync` to the METADATA catalog engine (a status-level
`data_engine_io` aggregate now reports the data-group engines); the
benchmark's `create-keyspace` was the LEGACY path that never touches
data groups (the corrected fixture uses a data-group keyspace — which
also means the proposal-batching bench never exercised its aggregator);
and the deferral policy was initially applied before
`enable_segmentation` switched the WAL backing, landing on the
discarded backing.

## The mechanism

With `KV9_DATA_SYNC_DEFER_BYTES > 0` (default 0 = strict), DATA-GROUP
engines defer their apply-record fsync until the unsynced bytes reach
the window OR a 100ms age bound — whichever first. The age bound is a
correctness-of-latency fix discovered by measurement: a 4MiB window
without it accumulated dirty pages that entangled OTHER files' fsyncs
in one giant journal commit (C=1 p99 +939ms); with it, measured C=1
p99 overhead is 0ms. Acknowledged writes rest on the SYNCED raft log
and deterministic replay — the same recovery machinery that already
covers a crash between the raft sync and the engine sync; deferral only
widens the replayed window. The metadata catalog is NEVER deferred.
Log compaction takes a `sync_applied_now` barrier before discarding any
replay source. Torn unsynced tails truncate at recovery (CRC-guarded).

## Predeclared targets, and the accepted result

Fixed in the workdir before any timed run: T1 C=32 write throughput
≥1.3×; T2 data-group engine syncs per op reduced ≥1.5×; T3 C=1 p99
overhead ≤5ms. The accepted median-of-3 benchmark (identical fixtures,
data-group keyspace, per-trial fresh clusters, all raw repeats
retained): **T1 2.13×** (median 299→638 ops/s; every deferred repeat
beat every strict repeat), **T2 10.2×** (0.346 → 0.034 syncs/op),
**T3 0ms**. Debug-profile, single-host, one data group — no scaling
claims; the earlier windows that missed targets or ran on broken
fixtures are all retained and labelled.

## Crash evidence

`scripts/deferred-sync-e2e.py` (accepted): concurrent acked writes
under active deferral, then SIGKILL of EVERY voter mid-load — twice,
with no shutdown sync — and after each restart every one of the 341
acknowledged writes reads back through public routing, with the
cluster taking new writes afterward. Unit tests cover the deferral
window, the sync_now barrier, torn-tail truncation with summary
regression, and strict-mode equivalence.
[proofs/lean/deferred-sync](../proofs/lean/deferred-sync/README.md):
fourteen theorems with five semantic mutation controls (an ack without
the durable raft entry; a crash keeping the unsynced tail; compaction
discarding the replay source; a sync covering an unwritten record; a
crash dropping the acknowledgement) and two proof-policy controls.

## Not claimed / remaining (#20 stays open)

Backpressure budgets and coordinated admission (items 3–4), the C04
dual-WAL decision (item 5), per-tenant fairness (T03),
release-profile and multi-host numbers (the 3/6/9-host contract in
[HORIZONTAL-SCALING-PLAN.md](HORIZONTAL-SCALING-PLAN.md)), chaos
coverage, bounded recovery time. The feature ships DEFAULT OFF.

Validation packet: [docs/deferred-sync-v1](deferred-sync-v1/README.md).
