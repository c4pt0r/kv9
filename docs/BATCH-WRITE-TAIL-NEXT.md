# Next batch-write tail investigation

This protocol has now run once. See the [accepted fixed-rate result](BATCH-WRITE-FIXED-RATE-RESULTS.md)
for all eight cohorts, higher ThinLTO p99, client drops and the preserved reader
failure/repair. The original planned rates, duration and scope below remain
visible. Actual frozen commands, capacity releases and terminal receipts are in
the linked evidence; this planning document is not a runtime receipt.

The [complete72 result](RELEASE-THIN-LTO-FULL72.md) improves loaded BatchPut(64)
throughput by 3.076% and mean latency, but worsens pooled p99. Its closed-loop
clients let the faster server complete more writes during the same duration.
The next question is whether the tail difference persists at the same offered
request rate and comparable completed work. This does not erase the observed
closed-loop tradeoff or establish its cause in advance.

## Bounded comparison

Keep the original CRC server `b33b5d30…` and the integrated ThinLTO server
`dd028cb2…`, whose full hashes and manifests are retained in the linked reports.
Use the original `0be806d9` native benchmark executable `8da9af46…`; do not
replace or recompile the timing client. Its existing `FixedRate` mode already
records offered/dropped slots, schedule lateness and scheduled-to-completion
latency. No server or client implementation change is needed for this question.

Proposed cells: pure BatchPut(64), 64 workers, 4,096 keys, 128-byte values,
8,000 and 12,000 batch calls/s, ten seconds per cell, both CRC/ThinLTO and
ThinLTO/CRC orders. That is eight native timed cohorts and 800,000 offered
batch calls in total. Both rates are below the previously measured CRC
closed-loop rate; this is a selection rationale, not a promise of zero drops.
Redis is unnecessary for this version-specific diagnosis.

Reuse the original three-voter topology, CPU placement, transport, preparation,
ordinary quorum/sync calls and tmpfs WAL scope. Keep fixed seeds and the same
slot-to-operation sequence for each paired rate. Use fresh data directories;
retain preparation traffic separately. Preserve existing storage floors,
per-file limits and complete writer-exit/retention/cleanup predicates. Calculate
capacity for smoke, all timed writes, retained evidence and restoration before
launching; the old closed-loop reservation is not automatically transferable.
Builds, archival and fault injection must not overlap measurement.

## Accounting and interpretation

The client assigns strided slots to workers and sheds overdue slots. Therefore
equal configured rates alone do not prove equal admitted or completed work.
For every cohort retain offered, issued, successful, refused, unknown, retry,
dropped and post-cutoff counts, plus actual duration and full histogram counts.
Check `offered = issued + dropped` and preserve each outcome population.
If losses or unresolved calls prevent a matched-work comparison, report that
limitation; do not drop the cohort or silently lower the rate and rerun it.

Report whole-call and **scheduled-to-completion** mean/p50/p95/p99 separately,
together with schedule lateness. A good service-time tail with poor scheduling
or dropped slots cannot establish good offered-load latency. Timer granularity
and 64-worker scheduling can affect arrival shape; inspect those recorded
populations before attributing a difference to the server. Do not subtract
means from different populations to manufacture a queue-time estimate.

Compare each run order before pooling. Retain successful calls/s and items/s,
actual write counts, per-voter CPU/RSS, fresh final drains and lifetime identity.
If the tail difference disappears only with lower load, that supports a
load-dependent explanation; it does not prove that higher load caused every
earlier tail sample. If it persists at matched successful work, use a bounded
diagnostic to locate the changed cost before altering batching or persistence.
Two repetitions are not a significance or sustained-capacity claim.

No Raft, fsync or acknowledgement boundary may be weakened. This short tmpfs
comparison cannot establish real-disk write latency, equivalent Redis write
durability or independent-host availability. The separate
[single-GET quorum investigation](QUORUM-LATENCY-NEXT.md) remains the main read
optimization path; scheduler/transport rewrites require new causal evidence.
