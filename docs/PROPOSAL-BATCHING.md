# Proposal batching (#20 slice): mechanism qualified, targets missed

Updated 2026-09-17. Parent [#9](https://github.com/c4pt0r/kv9/issues/9),
issue [#20](https://github.com/c4pt0r/kv9/issues/20). The first #20
slice: bounded proposal aggregation for public raw writes, shipped
correctness-qualified and DEFAULT OFF — because the predeclared
performance targets were NOT met, and predeclared targets are never
revised after timing.

## The mechanism

With `KV9_PROPOSAL_BATCH_OPS > 0`, in-flight SAME-EPOCH fenced raw
writes of one data group coalesce into ONE raft entry
(`WriteAggregator` in `crates/server/src/runtime/range_api.rs`): the
first writer opens a batch and owns its flush after a bounded delay
window (`KV9_PROPOSAL_BATCH_DELAY_MS`, clamped to 50ms) or an early
close (ops/byte caps, epoch mismatch); joiners append in arrival order
and await the shared receipt. The merged command rides the exact
unchanged submission path (`propose_with_async_wait` +
`finish_async_proposal` retry identity); every participant receives the
one applied position — receipts stay exact, and NotLeader/StaleEpoch
refusals stay typed through the fan-out. An epoch change (a split's
seal) closes open batches: cross-epoch writes never merge. Catalog
transactions and every non-raw path are untouched.

## The predeclared targets, and the honest result

Targets were fixed in the increment workdir BEFORE the first timed run:
T1 C=32 write throughput ≥ 1.4× unbatched; T2 raft-log syncs per op
reduced ≥ 2.0×; T3 C=1 p99 overhead ≤ 25ms. The accepted bench
(`scripts/batching-bench.py`, identical single-host fixtures, both
sides same binary and durability, per-trial fresh clusters):

- **T1 FAILED**: 0.82× (an earlier informal run of the same fixture
  measured 1.12× — the delta is within run-to-run variance).
- **T2 FAILED**: 0.92× (unbatched already ran at ~0.44 raft-log syncs
  per operation at C=32).
- **T3 PASSED**: 0ms measured p99 overhead at C=1.

The finding behind the numbers: the Ready-persistence layer ALREADY
group-commits — one raft-log sync covers many entries under load — so
collapsing proposals into fewer entries does not reduce fsyncs on this
fixture, and throughput moves within noise. The mechanism therefore
ships DEFAULT OFF, with no performance claim. Raw reports, configs,
sync counters and the two failed earlier bench attempts (a crashed
compute step and a drain-verification flake) are all retained in the
packet.

## Correctness evidence

- Accepted zero-shortcut e2e (`scripts/proposal-batching-e2e.py`): 32
  concurrent writers, every write landing with a nonzero applied
  receipt under batching; an automatic split fires MID-FILL so the
  epoch fence closes open batches under load; every key reads back;
  full-cluster restart recovers.
- Aggregator unit tests: merge order, fence segregation, ops/byte
  caps, taken-batch exclusion, typed shared errors.
- Fifteen-theorem Lean model
  ([proofs/lean/proposal-batching](../proofs/lean/proposal-batching/README.md))
  with five semantic mutation controls and two proof-policy controls;
  twenty-seven sibling Lean models and the retention TLA/TLAPS model
  re-accepted.

## Not claimed / remaining (#20 stays open)

No throughput or latency improvement claim. Backpressure budgets and
coordinated admission (#20 items 3–4), the C04 dual-WAL decision (item
5), per-tenant fairness (T03), release-profile and multi-host
measurement (the 3/6/9-host contract in
[HORIZONTAL-SCALING-PLAN.md](HORIZONTAL-SCALING-PLAN.md)), and chaos
coverage all remain future work.

Validation packet: [docs/proposal-batching-v1](proposal-batching-v1/README.md).
