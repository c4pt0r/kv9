# Direct lowering with real MemEngine updates

Recorded: 2026-09-16 UTC. **Hold the standalone prototype.** Composing direct
lowering with unchanged `MemEngine::write_applied` reduces prepopulated overwrite
mean only **1.134% without a snapshot / 0.452% with a snapshot**. All twelve
opposite-order mean comparisons improve, but initial-fill unpinned p99 worsens
**3.309%**. The predeclared advancement gate fails. Production remains unchanged;
this is not a database-QPS result or a Redis comparison.

The preceding [lowering-only experiment](RAW-APPLY-ATTRIBUTION.md) saved
1.548 us/group. This follow-up measures that change together with the real index,
write lock, applied-position/data-revision publication and destruction of the
consumed WriteBatch. It excludes decoding, fence adjudication, full state-machine
checks, WAL I/O, Raft, RPC and queues.

## Workloads and results

All cases retain the original 106 group boundaries / 1,564 command boundaries
and 100,096 mutations. Prepopulated overwrite uses the original keys with
same-length initial values differing in their first byte. Initial fill starts
empty and follows the original workload, so it transitions from insertion to
overwrite; it is not a pure-insert test. Unique insert appends a globally unique
eight-byte ordinal to each original Raw key, preserving its mode/keyspace prefix,
values and command/group structure. That case is explicitly synthetic.

When enabled, a fresh owned snapshot is held from immediately before each group
until its write completes. Snapshot creation/destruction, initial population,
final-state checks and engine teardown are outside timing. Each pass starts a
fresh engine. There is no long-lived reader thread or concurrent writer here.

| Workload | Snapshot held | Baseline mean/group | Direct mean/group | Mean change | Baseline p99 | Direct p99 |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| Prepopulated overwrite | No | 126.660 us | 125.224 us | -1.134% | 351.601 us | 346.522 us |
| Prepopulated overwrite | Yes | 140.154 us | 139.521 us | -0.452% | 380.856 us | 378.281 us |
| Initial fill | No | 129.935 us | 129.671 us | -0.203% | 345.409 us | 356.840 us |
| Initial fill | Yes | 145.753 us | 142.118 us | -2.494% | 389.192 us | 377.179 us |
| Unique insert | No | 289.118 us | 280.487 us | -2.985% | 849.596 us | 819.140 us |
| Unique insert | Yes | 378.385 us | 370.135 us | -2.180% | 1,174.157 us | 1,092.594 us |

Each case runs baseline/direct/direct/baseline, with 12 passes per row after
one warmup pass per arm. Each pooled arm contains 2,544 group samples. Percentiles
are recomputed from raw samples. Across 24 timed rows this is **30,528 group
samples / 28,827,648 mutations**, with 288 validated final states. CPU 4,
jemalloc 0.6.1, Rust/Cargo 1.94 and release ThinLTO are fixed on the shared host.
These short, repeated-corpus CPU measurements do not establish statistical
confidence for an endpoint workload or an exclusive index CPU fraction.

## Correctness and independent checks

The same original Raft/engine join is revalidated before measurement: every
original command and engine payload matches, and the original catalog accepts
all selected fences. Synthetic unique keys are an engine workload, not a claim
that those modified commands were committed by the original Raft group.

Dedicated baseline/direct checks compare all three column families with a
separate ordered-map model after every group: **1,272 live-state prefixes and
636 retained old snapshots**. Applied positions and data revisions are checked
as well. Two optimized standalone tests cover mutation order, empty/binary data,
deletions, all column families, snapshot immutability and refusal of a repeated
position before mutation. All pass.

An independent Python checker decodes every retained group, reconstructs all
three workload payloads and final states, verifies their hashes against the Rust
outputs, and recomputes every row and pooled statistic. It also binds the source,
build, executable, original command joins and successful process terminal. The
build retains 165 dependency source identities and 269 registry identities.

The first build failed on a slice-versus-array comparison in the new oracle.
Its exact source/logs are retained. The correction adds an explicit slice;
the subsequent build and both tests pass. No failed measurement was discarded
and no successful matrix was rerun. There is no new mechanized proof, recovery
or Chaos acceptance because this prototype has not entered production.

## Decision and next work

Before measurement, advancement required at least a 3% pooled mean reduction in
both prepopulated-overwrite snapshot cases, improving means in both orders for
all six cases, and no pooled p99 regression. Only the opposite-order mean
condition passes. Keep the prototype outside production and do not repeat this
unchanged screen or advance it to an expensive runtime campaign.

The next [resident-index layout experiment](RESIDENT-INDEX-LAYOUT-NEXT.md) targets
the ordered map's node and allocation structure while preserving snapshots and
ordered access. Earlier coalescing, borrowed-upsert and clone-removal variants
remain stopped. This is a new hypothesis, not an established gain.

The [evidence packet](raw-lowering-composed-v1/README.md) contains source, plans,
raw samples, opposite-order results, independent analysis and both build logs.
Full archive readback passes. CI remains local. Latest database write numbers
remain in the [same-source WAL screen](WAL-PREALLOCATION-PERFORMANCE.md).
