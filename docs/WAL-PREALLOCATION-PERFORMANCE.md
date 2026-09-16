# WAL payload preallocation: same-source write results

Completed locally on 2026-09-16 UTC. **Keep CRC selected and WAL payload
preallocation default-off.** Loaded Put improves **0.545%**, and loaded
BatchPut(64) improves **1.474%** pooled. Both loaded pooled p99 values remain
in the same histogram buckets. Batch throughput, mean and p99 change direction
between the two execution orders. The encoder's earlier 27.339% mean-time
reduction does not become a comparable database improvement.

All eight smokes and sixteen ten-second timed cohorts pass independent
acceptance. The timed population contains **7,200,959 successful calls /
63,451,454 input items**, each with one data-command attempt. Failed, refused,
unknown and client-rejected outcomes, hidden data retries and dropped slots
are zero. Complete final datasets, **48 fresh applied drains, 48 voter/writer/
listener bindings and 64 exited timed lifetimes** pass. The eight smokes
separately contain 749,264 calls / 7,790,333 items and 32 exited lifetimes;
their performance values are excluded from this report.

## Throughput and whole-call latency

Both server roles come from the same clean `86aa6fc` source, including the
checkpoint-owner integration. Only the candidate enables
`wal-payload-preallocation`. Both use one qualified measurement client; there
is no pooling with earlier server versions or historical client executables.

| Concurrency / API | Default throughput | Preallocated throughput | Change | Default mean / p99 | Preallocated mean / p99 |
| --- | ---: | ---: | ---: | --- | --- |
| 1 / Put | 19,316.845 calls/s | 19,581.309 calls/s | +1.369% | 51.673 / 68.608–69.631 us | 50.973 / 67.584–68.607 us |
| 64 / Put | 137,873.776 calls/s | 138,625.767 calls/s | +0.545% | 464.072 / 745.472–753.663 us | 461.547 / 745.472–753.663 us |
| 1 / BatchPut(64) | 397,327.528 items/s | 398,749.157 items/s | +0.358% | 160.959 / 208.896–210.943 us | 160.390 / 208.896–210.943 us |
| 64 / BatchPut(64) | 1,022,750.059 items/s | 1,037,829.100 items/s | +1.474% | 4.004 / 9.306–9.437 ms | 3.945 / 9.306–9.437 ms |

Rates divide summed successful calls/items by summed actual elapsed time.
Means use original integer latency sums and counts. Percentiles merge the
original histogram counts and retain inclusive bucket bounds; they are never
averaged. Batch latency covers one complete 64-item atomic call. The
[portable report](wal-preallocation-performance-v1/ANALYSIS.md) also retains
batch calls/s, p50/p95, per-voter CPU and all outcome populations.

| Workload | Default-first throughput change | Candidate-first throughput change |
| --- | ---: | ---: |
| c1 Put | +1.411% | +1.327% |
| c64 Put | +0.437% | +0.654% |
| c1 BatchPut(64) | +1.024% | -0.304% |
| c64 BatchPut(64) | -0.143% | +3.066% |

Loaded batch p99 worsens from **9.568–9.699 to 10.617–10.748 ms** in the
default-first order, then improves from **9.044–9.175 to 7.864–7.930 ms** in
the candidate-first order. Low-concurrency batch p99 also changes direction.
Loaded point p99 improves within each order but falls in the same bucket when
the original populations are merged. Two short orders on a shared host do
not establish statistical significance or the cause of these differences.

Sampled three-voter CPU is **3.552 / 3.559 cores** for default/preallocated
loaded Put and **3.319 / 3.321 cores** for loaded BatchPut(64). Client CPU is
0.430 / 0.433 and 0.468 / 0.471 cores respectively. Sample boundaries differ
between processes; these rates do not measure exclusive per-request CPU.

## Exact inputs and acceptance

| Role | Source revision | Executable SHA-256 |
| --- | --- | --- |
| Default server | `86aa6fc07d82f4d49b43fbfe309e0e5c20276da5` | `04749125c176ab25b1a5cb9537ea55b9df3573201d3dfb7006b066cc8531c144` |
| Preallocated server | `86aa6fc07d82f4d49b43fbfe309e0e5c20276da5` | `1a38b0a0223650f42aa306131787937f55d7cfc9db2963b2d4bdf8f3d2c8566e` |
| Fixed measurement client | `0be806d9671e2c50701a64aa7889c8859b7648ba` | `1b8060eb168610328c10a27480de16bd8b2d6805166d8169638f85ceef0992c4` |

The matching releases retain the qualified Rust/Cargo 1.94 toolchain and
ThinLTO configuration. Both server source trees have the same 1,529 file
identities. The feature is present only in the candidate's root/engine graph;
the Raft/server feature lists remain empty. The separately
[qualified client](WRITE-CLIENT-REQUALIFICATION.md) is unchanged. Explicit
feature acceptance/rejection controls preserve source and build binding.

Each cohort has 4,096 mutable keys plus a sentinel, 128-byte values, seed 71,
128 warmup calls and a ten-million-call cap. Point writes use singular Put;
batches use atomic BatchPut(64). Closed-loop concurrency is 1 or 64. Client
CPUs are 0–1; all three voters share CPUs 2–5. Helpers and the three owned
background containers use CPUs 6–15 and 22–31 during timing. Original container
CPU settings and namespace identities are restored exactly. Unrelated host
services remain outside this isolation, so this is a shared-host diagnostic.

Raft quorum, WAL synchronization, applied-position and response fences remain
enabled. Active WAL storage is **volatile tmpfs**. This does not establish
physical-disk throughput, power-loss durability, cross-host availability,
sustained capacity or read/mixed regression performance. No fault is injected
in this timing screen. Separate [source-bound proofs](WAL-PAYLOAD-PREALLOCATION.md),
[ordinary recovery](WAL-PREALLOCATION-RUNTIME.md) and
[actual full21 Chaos Mesh acceptance](WAL-PREALLOCATION-CHAOS.md) remain accepted.
Those gates were not repeated. Redis was not rerun; its
[earlier WAIT references](WRITE-REDIS3-BASELINE.md) retain different confirmation
and durability semantics. Redis parity is not achieved.

Actual terminals are smoke **1999/4dc234/0**, smoke readback **df9caa/0**,
timing/restoration **3737/e455eb/0**, independent audit **34892/1b65a0/0** and
report derivation **85511/405668/0**. Audit SHA-256:
`71ce123e31591eb908c96fca51716607a945097df1b04eca156beb34495f97f9`.
The eight pure histogram/rate/CPU/comparison functions are byte-identical to
the accepted reporting predecessor; current source, role and protocol bindings
are explicit.

All **72,178,414,308 original logical bytes** across smoke and timing decode
and hash-check successfully. Timed retention contains 2,399 original files /
64,260,204,961 logical bytes; combined physical retention is 50,898,837,504
allocated bytes. Output uses `/mnt/data/kv9-work`; root/data/tmpfs capacity
guards and restoration reserves are unchanged. Compression runs after writer
reaping and before the next cohort, never during measurement.

The [portable metadata packet](wal-preallocation-performance-v1/README.md)
contains all 78 reporter inputs, every timed and smoke report, exact original
audit records and preparation failures. Independent archive readback passes
**e08d1c/0** after packaging **15888f/0**. The initial metadata selection exceeded
its inherited aggregate cap; the documented 80-to-96-MiB publication-only
increase retains every input. Runtime/audit gates are unchanged. Large WAL
objects and executables remain local, so this packet is not standalone replay.
The separate I/O diagnosis attributes the slow post-timing audit to physical
reads on the data volume; it changes neither timing nor acceptance.

## Development decision

Keep the selected runtime and retain this candidate as default-off. The small
point gain and inconsistent batch result do not justify promotion or another
unchanged full screen. Preserve the completed proofs, recovery, Chaos and
original measurements. Do not add these gains to earlier held experiments.

Next quantify Raw-command lowering separately from resident-index updates,
using retained input and exact command/group boundaries before proposing a
runtime change. Engine WAL batch boundaries alone do not reconstruct original
Raft commands or fence outcomes; obtain those from matching retained Raft
records when needed. Measure allocation/copy and fence-evaluation costs, keeping
validation, mutation order, snapshot semantics, atomic publication and durable
acknowledgments intact. Advance a changed candidate only after a material,
repeatable gain across inserts, overwrites and retained snapshots. Do not
repeat rejected clone-removal, sorting/coalescing or borrowed-upsert variants.

Read optimization remains held. Dynamic multi-Raft and automatic splits retain
their write-phase and industrial-storage dependencies. Typed negative checkpoint
history, reference fencing/drainage and destination installation remain open;
the pending pre-upload crash gate is not claimed complete. This checkpoint
closes no original industrial issue checkbox. CI remains local.
