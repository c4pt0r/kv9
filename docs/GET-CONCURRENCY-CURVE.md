# Single-GET concurrency curve: short-path latency and admission pressure

The accepted `5ee897a` source reaches **26,216–26,251 successful GET/s at c1**
with **37.975–38.012 us mean latency**. Redis GET reaches 170,019–170,516/s
and 5.789–5.806 us under the same placement and payload protocol. The
single-request turnaround gap is approximately 6.5x. At c64, KV9 reaches
342,979–344,681/s and 185.549–186.470 us, versus Redis 498,993–508,360/s
and 125.782–128.143 us. Increasing offered concurrency hides some turnaround
cost until the existing admission limit becomes material.

At c128, KV9 refuses **30.183–30.857%** of calls for admission count and
completes 345,907–349,389 successful GET/s. At c256 it refuses
**49.924–50.596%**, while successful throughput falls to 306,181–308,856/s.
Those points describe overload with the existing configuration; they do not
establish intrinsic server capacity or Redis parity. The selected production
source remains unchanged. This is diagnostic evidence, not a new optimization.

## Why Redis is fast, and what KV9 should learn

Redis's ordinary GET has a short, mostly single-threaded command path, backed
by nonblocking I/O multiplexing. Its documented GET complexity is O(1).
Keeping command execution continuous reduces the need to coordinate workers
around each small operation. This is the useful architectural lesson for KV9;
the earlier one-worker KV9 experiment lost about 35% throughput, so lowering
thread count without redesigning the path is not sufficient.
Sources: [GET](https://redis.io/docs/latest/commands/get/) and
[Redis execution and latency](https://redis.io/docs/latest/operate/oss_and_stack/management/optimization/latency/).

RESP uses length-prefixed payloads that can be parsed without scanning or
escaping their contents. Persistent connections and pipelining can amortize
I/O overhead. This particular Redis client uses one preconnected connection
per worker and one outstanding GET per connection; its measured advantage is
not a pipeline-versus-single-command comparison.
Sources: [RESP specification](https://redis.io/docs/latest/develop/reference/protocol-spec/)
and [pipelining](https://redis.io/docs/latest/develop/using-commands/pipelining/).

Standalone Redis GET does not execute KV9's fresh Raft quorum/applied-index
read barrier. Redis replication is asynchronous by default, and WAIT does not
turn it into a strongly consistent CP system. Keep that guarantee difference
explicit without treating all KV9 overhead as unavoidable consensus cost.
Source: [Redis replication](https://redis.io/docs/latest/operate/oss_and_stack/management/replication/).

The [existing c64 lifecycle diagnostic](RPC-PAIR-READ-LIFECYCLE.md) measures
66.634 us from admitted invocation to observed confirmation and 40.770 us from
result send to receiver observation. Those instrumented, load-dependent stages
must not be subtracted from this uninstrumented c1 result. Confirmation includes
local processing, transport, quorum and owner observation; notification includes
delivery and scheduling. The profile's 6.46% kernel-network CPU samples do not
establish a NIC ceiling or justify DPDK as the next change.

## Complete matched results

Throughput counts successful logical calls only. Mean and p99 below also use
successful calls only. p99 is a histogram bucket interval, never an averaged
percentile. All calls succeed through c64 for both targets; Redis succeeds at
every point. Native c128/c256 include refusals, detailed separately below.

| Offered concurrency | Repetition | Target | Successful calls/s | Success mean us | Success p99 bucket us | Success rate |
| --- | --- | --- | ---: | ---: | --- | ---: |
| 1 | 1 | KV9 | 26,215.7 | 38.012 | 60.416–60.927 | 100.000% |
| 1 | 1 | Redis | 170,018.5 | 5.806 | 8.128–8.191 | 100.000% |
| 1 | 2 | KV9 | 26,250.5 | 37.975 | 58.880–59.391 | 100.000% |
| 1 | 2 | Redis | 170,516.4 | 5.789 | 8.192–8.319 | 100.000% |
| 8 | 1 | KV9 | 116,884.6 | 68.323 | 104.448–105.471 | 100.000% |
| 8 | 1 | Redis | 490,215.6 | 16.216 | 24.064–24.319 | 100.000% |
| 8 | 2 | KV9 | 116,497.1 | 68.550 | 104.448–105.471 | 100.000% |
| 8 | 2 | Redis | 490,092.3 | 16.221 | 23.808–24.063 | 100.000% |
| 32 | 1 | KV9 | 270,315.2 | 118.254 | 204.800–206.847 | 100.000% |
| 32 | 1 | Redis | 494,507.8 | 64.597 | 109.568–110.591 | 100.000% |
| 32 | 2 | KV9 | 269,377.3 | 118.666 | 206.848–208.895 | 100.000% |
| 32 | 2 | Redis | 508,081.6 | 62.876 | 109.568–110.591 | 100.000% |
| 64 | 1 | KV9 | 342,978.9 | 186.470 | 356.352–360.447 | 100.000% |
| 64 | 1 | Redis | 508,360.3 | 125.782 | 231.424–233.471 | 100.000% |
| 64 | 2 | KV9 | 344,680.5 | 185.549 | 360.448–364.543 | 100.000% |
| 64 | 2 | Redis | 498,992.9 | 128.143 | 231.424–233.471 | 100.000% |
| 128 | 1 | KV9 | 345,906.9 | 306.710 | 565.248–573.439 | 69.143% |
| 128 | 1 | Redis | 500,715.4 | 255.481 | 479.232–483.327 | 100.000% |
| 128 | 2 | KV9 | 349,389.4 | 304.872 | 557.056–565.247 | 69.817% |
| 128 | 2 | Redis | 492,274.6 | 259.869 | 483.328–487.423 | 100.000% |
| 256 | 1 | KV9 | 306,181.3 | 513.184 | 892.928–901.119 | 49.404% |
| 256 | 1 | Redis | 488,694.0 | 523.607 | 1007.616–1015.807 | 100.000% |
| 256 | 2 | KV9 | 308,855.5 | 512.502 | 884.736–892.927 | 50.076% |
| 256 | 2 | Redis | 496,595.4 | 515.311 | 991.232–999.423 | 100.000% |

All measured native calls make exactly one SDK attempt; all Redis calls make
one command attempt. The native six-attempt configuration remains unchanged,
but no retries occur in this recording. Admission refusals are terminal outcomes.
All other native reasons/outcomes, and all Redis non-success outcomes, are zero.
No calls are dropped or capped. The runner and auditor allow complete non-success
populations; one attempt and success at lower concurrency are observations.

| KV9 concurrency | Repetition | Admission-count refusals | Refusal rate | All-outcome mean us | All-outcome p99 bucket us |
| --- | --- | ---: | ---: | ---: | --- |
| 128 | 1 | 771,897 | 30.857% | 255.716 | 540.672–548.863 |
| 256 | 1 | 1,568,000 | 50.596% | 412.798 | 819.200–827.391 |
| 256 | 2 | 1,539,744 | 49.924% | 414.797 | 819.200–827.391 |
| 128 | 2 | 755,294 | 30.183% | 255.624 | 532.480–540.671 |

The complete matrix contains **45,300,391 terminal calls**:
**40,665,456 successes** and **4,634,935
admission-count refusals**. Successful-only latency at c256 must not be compared
with Redis's all-success population as evidence of equivalent service quality.
The [machine-readable result](../scripts/redis-reference/get-concurrency-v1/results.json)
retains every cohort's outcomes, reasons, attempts and both latency populations.

## Frozen protocol and acceptance

Two repetitions cover c1, c8, c32, c64, c128 and c256: ascending concurrency
with KV9 then Redis, followed by the complete reverse order. Each cohort uses
five seconds, 128 warmup calls, 4,096 keys plus sentinel, 128-byte values,
single GET, closed-loop workers, a ten-million-call cap and a 1,500-ms deadline.
Native SDK max-in-flight equals offered concurrency; public server admission
remains 64 requests / 16 MiB per voter, and the async-read registry remains 128.
These existing fixed bounds explain why high offered concurrency is an overload
experiment. All Redis workers are preconnected before the workload.

The ordinary streaming-gRPC server uses three Raft voters and volatile tmpfs
WAL with normal sync calls. Redis 7.0.15 is standalone with save/AOF disabled
and io-threads=1. Neither equal durability nor cross-host behavior is claimed.
Clients use CPUs 0–1; all voters share 2–5. Three owned background containers
use 6–15,22–31, then regain their original configured/effective 0–31 masks.
Unrelated host services remain unconstrained. No build, test, fault, profile
or audit overlaps timing. This is a short shared-host diagnostic, not sustained
capacity or a broad release gate.

Six driver/client compatibility tests and twelve independent auditor-contract
tests pass before runtime. A separate c1/c256 four-cohort correctness smoke
passes, retaining its high-concurrency refusals. Its data are excluded from
timing. The first complete 24-cohort timing exits 0 (root session 4852), and the
first independent audit exits 0 (67649). It verifies all 72 owned lifetimes
exited, 36 fresh three-replica drain documents, 36 voter/listener bindings,
1,743 source-file checks, 2,344 resource observations, final 4,097-key datasets,
396 retained files / 66,566,882 bytes, container restoration and namespace UID
preservation. No fixture or failed predicate was rerun to obtain acceptance.
The independent auditor's initial construction-anchor failure remains retained;
it occurred before the auditor file or runtime existed.

## Next development step

Prioritize the GET execution/confirmation path with the accepted source as
control. Use the c1 and c64 points together: c1 exposes request turnaround,
while c64 checks useful overlap and throughput. Map registration, owner wake,
peer processing and result delivery at low concurrency before choosing one
handoff to change. Keep the existing c64 lifecycle evidence separately scoped.
Preserve fresh quorum confirmation, applied-index guards, absolute deadlines,
per-frame authentication and cleanup/admission ownership in every candidate.

Separately test the current admission configuration as an explicit controlled
variable before calling the high-concurrency plateau a server limit. Increasing
that bound requires bounded-resource evidence and all outcome populations; it
is not itself an execution-path optimization or a correctness waiver. Accept
performance changes only with useful throughput and mean/p99 behavior, then
complete the relevant proof, local correctness and actual Chaos Mesh gates.
Writes, mixed traffic, sustained operation and Redis parity remain open before
dynamic multi-Raft and automatic range splits.

## Reproduction and evidence

The [executed helpers](../scripts/redis-reference/get-concurrency-v1/README.md)
are retained byte-for-byte with their original absolute-path bindings. Native
client source is `03c1c776a5dd7d1cc67491ab253e02ce51665bf8`, Redis client source
is `b8ec38f660786412705f350d96ab086f0e7f6c60`, and server source is
`5ee897a2f58c57bdf17ea1757c96224adf0f0dbb`. Their original default-feature release
artifacts are reused; no server/client rebuild or implementation change occurs.

Raw data: `/tmp/kv9-get-concurrency-diagnostic-attempt1/cohorts`.
Independent audit: `/tmp/kv9-get-concurrency-preparation/results-first/audit.json`.
The machine-readable result includes their hashes and complete per-cohort report
hashes. Original smoke, logs and preparation failures remain separate.

| Artifact | SHA-256 |
| --- | --- |
| matched-driver.py | `f033816fc41437ed38215f9baffde126174ed4b032d480fd9097a31046b55701` |
| audit.py | `feda317f7bc177ba48e8936ff1a4a8f66221d179ceec24dc94f2efe863394b7f` |
| isolate-and-run.py | `e922939e675d722fe36c3b0890d31d6329be810dd7567b2e369643d10e2137fb` |
| protocol.json | `3c784f5b9b9fd7f28f51abb9d7e2da95b95cb4fecb91ebc7d1627fe142d64d20` |
| Independent audit result | `8348cfb651b5a6a1f1c13871b7b487e6db062bf76722204c8facd4939e6bf768` |
| Published result | `084b1a26701436eb63a2a8de667210bdddf1f1800175aac33148d784e844e20b` |
