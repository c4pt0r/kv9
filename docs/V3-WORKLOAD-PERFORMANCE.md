# Point and batch read/write performance against Redis

The new full workload comparison moves the next performance priority to the
write/apply path. At c64, read-credit candidate `57ff6851` reaches **373,909
point GET/s**, but only **117,729 point PUT/s** and **628,102 keys/s with
BatchPut(64)**. Same-recording Redis reaches **512,191 GET/s**, **493,396 SET/s**
and **6,066,081 keys/s with MSET(64)**. The respective throughput gaps are
approximately **1.37x, 4.19x and 9.66x**. There is no Redis-parity claim or new
general server promotion. The general baseline remains `5ee897a`.

The first independent audit accepts all **72 timed cohorts**, **79,909,933
measured calls** and **787,052,110 input keys**, with every measured call
successful and one attempt each. The separate 36-cohort correctness smoke also
completed. This refresh uses the v3 measurement clients already published at
`0be806d9`; it introduces no production server change.

## Results and comparison scope

The fixed matrix covers actual point GET/PUT and BatchGet/BatchPut(64), read
percentages 0/50/100, concurrency 1/64 and three arms: general baseline,
read-credit candidate and standalone Redis 7.0.15. Two ten-second repetitions
run the entire 36-case list forward then in reverse. The workload has 4,096
keys plus sentinel, 128-byte values, seed 71, 128 warmup calls, a ten-million
call cap and a 1,500-ms deadline. All outcomes and configured SDK attempts
remain accounted for. No per-cell performance retry or discarded result occurs.

Clients use CPUs 0-1; all three KV9 voters, or the Redis server, share CPUs 2-5.
Helpers and three owned background containers use 6-15,22-31. The host remains
shared. KV9 retains quorum, commit/apply acknowledgement and normal sync calls
on volatile tmpfs. Redis has save/AOF disabled, no replicas, one I/O thread and
one outstanding command per connection, with no pipelining. These are not
equal-durability systems, nor a real-NIC or sustained-capacity experiment.

Pooled rates divide summed successful calls or keys by summed elapsed time.
Means are weighted by calls, and quantiles merge the original histogram
counts. Batch latency is the complete call latency and is never divided by 64.
The table below uses the read-credit candidate; the [full readout](../scripts/redis-reference/v3-workloads-v1/READOUT.md)
also includes the general baseline, both individual repetitions and CPU usage.

| c64 workload | KV9 calls/s | Redis calls/s | KV9 mean us | Redis mean us | KV9 p99 us | Redis p99 us |
| --- | ---: | ---: | ---: | ---: | --- | --- |
| Point GET | 373,909 | 512,191 | 171.042 | 124.845 | 294.912–299.007 | 231.424–233.471 |
| Point PUT/SET | 117,729 | 493,396 | 543.494 | 129.557 | 876.544–884.735 | 235.520–237.567 |
| Point 50% reads | 167,810 | 501,639 | 381.244 | 127.447 | 647.168–655.359 | 235.520–237.567 |
| Batch64 GET/MGET | 36,240 | 92,743 | 1,760.753 | 688.070 | 2,949.120–2,981.887 | 1,040.384–1,048.575 |
| Batch64 PUT/MSET | 9,814 | 94,783 | 6,519.435 | 670.455 | 12,320.768–12,451.839 | 1,007.616–1,015.807 |
| Batch64 50% reads | 15,352 | 90,182 | 4,165.011 | 705.724 | 8,323.072–8,388.607 | 1,081.344–1,097.727 |

The three batch rows respectively represent KV9 **2,319,345 / 628,102 /
982,521 keys/s**, versus Redis **5,935,553 / 6,066,081 / 5,771,667 keys/s**.
Actual finite mixed populations are retained; they need not be exactly 50/50.

At c1, candidate point GET averages **37.879 us**, versus Redis **5.699 us**;
point PUT averages **61.593 us**, versus Redis **5.696 us**. Their roughly
6.65x and 10.81x turnaround gaps show why high-concurrency throughput alone
does not establish a short individual request path.

The read-credit candidate repeats its c64 point-GET improvement over the
general baseline: **+8.240% / +8.027%**, pooled **+8.133%**, with improved
pooled mean and p99. It gives only **+0.440%** pooled point PUT and **+0.175%**
BatchPut throughput. c64 batch mixed throughput changes **+0.137% / -1.718%**,
pooled **-0.793%**, and pooled p99 worsens. c1 point GET falls **0.115%** with
worse pooled mean and p99. Retain these observations alongside earlier
screens; two repetitions do not establish significance or no regression.

At c64, native PUT uses approximately **3.54 server CPU cores**, and batch PUT
**3.31**, summed over the three voters. Redis uses approximately **1.00 / 0.90**
server cores in those cells. The Redis batch clients use approximately
**1.92–1.99 of their two allowed cores**, so their observed rates must not be
presented as Redis's intrinsic server ceiling. These process CPU estimates
are sampled and are not per-request latency decompositions.

## Write-stage evidence and the next experiment

The [write execution map](RAW-WRITE-EXECUTION-PATH.md) records the exact existing
reservation, metadata-fence, proposal, commit/apply and persistence ordering.
The independent audit binds all 288 before/after metrics/status/resource
documents. A separate root reader validates source-matched metric schemas,
PID/start/boot identity, fresh drains, capture brackets, histogram conservation
and nonnegative deltas for all 48 native cohorts and 533 input files.

The following successful-outcome means pool both repetitions of `57ff6851`
and all voters. **Their envelope includes initialization, warmup, measurement,
drain and final readback. Metrics overlap and have different populations.**
They cannot be added or subtracted to explain the client mean, and command
timers in an apply group share the entire group interval.

| Endpoint metric | Point PUT c1 us | Point PUT c64 us | Batch64 PUT c1 us | Batch64 PUT c64 us |
| --- | ---: | ---: | ---: | ---: |
| Public preparation queue | 2.555 | 12.079 | 2.351 | 98.135 |
| Public write backend | 41.309 | 414.334 | 176.249 | 5,688.651 |
| Proposal submission | 1.357 | 12.117 | 5.124 | 89.535 |
| Application wait | 36.787 | 398.037 | 127.228 | 5,509.094 |
| Command apply interval | 4.327 | 51.944 | 65.065 | 2,173.226 |
| Engine WAL record write | 0.695 | 1.412 | 2.216 | 31.574 |
| Engine WAL sync | 0.095 | 0.132 | 0.103 | 0.136 |
| Raft WAL sync | 0.093 | 0.126 | 0.104 | 0.133 |

The batch apply interval grows substantially, while observed sync calls on
tmpfs remain small. This supports investigating apply computation and its
queueing before attributing the batch-write gap to disk latency or kernel
networking. It does not identify the exclusive bottleneck or measure a
recoverable-disk write budget.

The next bounded diagnostic should profile **point PUT and BatchPut(64)** on
the general baseline, separately from performance selection. Attribute CPU
and bytes to command decode/conversion, metadata-fence checks, log handling,
ordered-index updates and scheduling. Existing Ready persistence and Raw apply
already batch work; increasing group sizes blindly can worsen tail latency.
Then implement the dominant avoidable path with focused safety tests and
repeat the same read/write/mixed matrix. A per-shard owner and owned mutation
buffers are concrete hypotheses; neither is selected on source intuition.

Redis's useful architectural lesson is local state ownership and very little
work per ordinary operation. KV9's persistent ordered map provides cheap
snapshot capture and ordered scans, but shared-path updates still have costs.
Owned `WriteBatch` data is also cloned on insertion today. Preserve scan and
snapshot semantics while testing representation improvements. The [Redis
analysis](REDIS-EXECUTION-PATH.md) separates these implementation choices from
the required distributed protocol. Keep fresh read confirmation, applied-index
checks, quorum replication, recovery ordering and bounded backpressure.

## Retained deviations and acceptance limits

Root's ancillary prelaunch summary failed an undeclared assertion that every
smoke phase must be single-attempt, then root incorrectly launched timing
without checking that command's exit. Each of 24 native smoke initialization
reads had one additional attempt; all measured smoke calls were successful and
single-attempt. The already reviewed driver, protocol and auditor bytes and
declared acceptance predicates remained unchanged. The final root inventory
was produced **after launch**, and is not claimed to be a prelaunch inventory.
The original failure, invocation and postlaunch readback are retained. The
independent audit accepting the recording does not erase this process error.

Timed initialization likewise records 48 extra attempts, with all logical
calls successful; warmup, measurement and verification are single-attempt.
No build, test, fault, profile or full audit overlapped timing. Light root
operational reads and metadata writes to retain the deviation did overlap.

The first statistics-only reader rejected the legitimate empty histogram of
an inactive operation before emitting results. Its original source/log remain
retained. A separately named corrected reader accepts only the validated
zero-count/zero-sum representation; the frozen audit and recording were not
changed or rerun. The endpoint reader separately passed its first full-matrix
execution after its pre-execution input-binding review correction.

The audit verifies **240 exited lifetimes, 144 fresh drains, 144 writer/listener
bindings, 13,990 resource samples, 2,340 source-file checks and 3,736 retained
files / 74,627,673,540 bytes**. Storage guards and exact owned-container CPU
restoration pass; historical namespaces and fault identities are preserved.
Minimum observed free tmpfs/retention space is **60,019,949,568 /
526,960,189,440 bytes**. These observations do not prove future space bounds.

Final deterministic values, sentinel, configured nonce limits and write-key
membership pass. This benchmark does not record exact issued nonces or full
concurrent histories and establishes no new full-history linearizability,
Chaos Mesh acceptance, proof composition or production readiness. Existing
source-specific correctness evidence remains separate. Dynamic multi-Raft
and automatic splits remain subsequent product work after the performance
gate. Tests and recording are local; no GitHub CI is dispatched.

The [evidence index](../scripts/redis-reference/v3-workloads-v1/README.md) links
the exact protocol, helpers, audit, complete statistics, endpoint derivation
and original failure chronology. Full raw data remain at their recorded local
paths; binaries and large WALs are not copied into Git.
