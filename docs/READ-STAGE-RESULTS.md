# Asynchronous read latency diagnosis — 2026-09-11

The successful asynchronous read's largest observed interval is quorum
confirmation. Under c64 load, completion-to-receiver resumption is also large.
The next experiment explicitly polls the RPC executor's global task queue every
eight task selections. This report establishes diagnosis, not a new QPS result
or a selected runtime change.

## Matched stage measurements

The opt-in [observer source and boundary contract](https://github.com/c4pt0r/kv9/blob/40f014f9df1ef4e64be188318e3a8d4c1bc8701c/docs/READ-STAGE-TIMING.md)
uses the selected CRC runtime and does not include notification coalescing.
Five adjacent intervals and their total come from each successful request's
same monotonic timestamp chain. Counts and exact sums agree on fresh drained
endpoints; percentile intervals are calculated from merged integer bucket counts.

| Successful lifecycle stage | c1 mean us | Share | c64 mean us | Share |
| --- | ---: | ---: | ---: | ---: |
| Registration | 0.140 | 0.58% | 0.353 | 0.31% |
| Queue publication to owner admission | 1.389 | 5.77% | 2.399 | 2.12% |
| ReadIndex call to exact quorum confirmation | 20.636 | 85.72% | 65.425 | 57.74% |
| Confirmation to successful reply send | 0.257 | 1.07% | 2.677 | 2.36% |
| Send boundary to receiver resumption | 1.653 | 6.86% | 42.449 | 37.47% |
| Total observed asynchronous read wait | 24.075 | 100% | 113.304 | 100% |

| Stage p99 bucket interval, us | c1 | c64 |
| --- | ---: | ---: |
| Quorum confirmation | 32.768–65.535 | 131.072–262.143 |
| Receiver resumption | 2.048–4.095 | 65.536–131.071 |
| Total | 32.768–65.535 | 131.072–262.143 |

These are two **instrumented** fixtures: c1/c64 closed-loop point GET, 4,096
data keys plus sentinel, 128-byte values, 128 warmup calls and five measured
seconds each. Before/after metrics bracket the entire client lifecycle and
therefore include batch GET initialization/verification. The stage populations
are **140,116** and **1,687,961** successful reads; the measured point GET
populations are **131,794** and **1,679,639**. Each fixture also performs 17
external scan pages through the synchronous read path, outside the stage
population. The reader retains all phases and their outcomes/attempts.

Quorum includes local owner processing, transport, remote peer work and the
returning confirmation. It is not network RTT alone. Resume includes sending,
wakeup and executor scheduling, not pure scheduling time. Stage totals exclude
observer recording and subsequent memory view/lookup/response encoding. Do not
add stage percentiles or subtract these means from measurement-only client
latency. Histogram resolution is intentionally coarse; no artificial precise
tail estimate is derived from a bucket.

## Source and runtime acceptance

Default Raft/Server checks pass **435 tests/doctests**, diagnostic checks pass
**438**, with one existing ignored test in each invocation. Formatting and
both all-target Clippy variants pass with warnings denied. New tests cover
stage arithmetic, malformed timestamps, and actual sealed-group confirmation,
apply coverage, cancellation and successful delivery. The observer is absent
from default builds and does not participate in any correctness decision.
No core algorithm or proof was changed; full Rust implementation proofs remain
open under the existing roadmap.

The original retained release matches all **626** checked source files. Its
serialized build receipt records first-party invalidation, **13** first rebuilt
units and **20** artifact observations; the source gate records 42 observations.
Rust is 1.94.0 (`4a4ef493e3a1488c6e321570238084b38948f6db`, LLVM 21.1.8).

| Artifact | SHA256 |
| --- | --- |
| Diagnostic server | `af1d61634219eb8094aca1c6590b540b2f10e00a2a5c4495686f05f35ad45b14` |
| Build manifest | `ed7d897ac49ebc1b18a6d3c29f5b7425011b518b8a0d6805d57afff4ac275bfc` |
| Cache receipt | `3d56c57f7afa80d2a220c02cbb81002930b689140e2c0ecc28944cd8b21a5299` |
| Fixed v3 client | `8da9af469f962a938027d1970141bbe4622f7d42b2b795f720e288fb3f8d5957` |
| Frozen fixture inventory | `d554bb58ea3d085613f54314401a8bf6b2a9584b98a05ee77ffc9bbf53b20029` |

Eight metric/build contracts and four inherited fixture contracts pass before
runtime. The independent readback accepts both original fixtures, all
**1,811,433 measured calls** succeeding in one attempt, eight exited process
lifetimes, six fresh drains, six voter/listener bindings and 12 metric documents.
It rejects zero/truncated stage coverage, observer errors, changed exporter or
leader identity, non-monotonic histograms and unmatched stage sums. Initialization
includes one extra routing attempt per fixture; it remains in full accounting.
No fixture was repeated, omitted or pooled with an earlier runtime.

The host is shared: clients use CPUs 0–1, the three voters use 2–5, and helpers
and owned background containers use 6–15/22–31. The normal three-voter quorum
and synchronous WAL path run on tmpfs. Exact CPU/namespace restoration and
unchanged storage guards pass. This does not establish real-disk, cross-host,
sustained-capacity, candidate Chaos Mesh or independent host-failure acceptance.

Original-byte reporting evidence is in [read-stage-diagnostic-v1](read-stage-diagnostic-v1/README.md).
Original binaries, WALs and full local output remain under the retained paths.
Integrity checks do not replace the independent runtime acceptance predicates.

## Next performance work

1. Test `.global_queue_interval(8)` on the existing two-worker RPC executor,
   preserving socket event interval eight. This targets remote Raft completion
   wakeups under sustained local task traffic. It is a hypothesis; the observed
   resume interval alone does not identify the particular Tokio queue.
2. Use default-feature, uninstrumented builds for the existing complete c1/c64
   GET/mixed comparison, retaining both run orders, means, p99 and complete
   outcomes. Run ordinary leader-loss/restart histories before timing; require
   applicable actual Chaos acceptance before selecting an optimized default.
3. Continue c1 localization inside the quorum round trip. Existing owner-queue
   and receiver-resume measurements do not explain most of the c1 delay, and the
   earlier body-handoff diagnosis did not justify replacing that channel.
4. Keep the original source-bound notification candidate frozen for its broader
   API and actual Chaos gates. The test-environment agent has prepared distinct
   network, Pod/WRITE-errno and follower-FSYNC scopes; preparation is not a run.

Selected runtime remains CRC `ca0002c7`. The previous uninstrumented
[notification comparison](COALESCED-OWNER-PERFORMANCE.md) remains the latest
accepted performance comparison until the next complete screen finishes.
