# Outbound executor isolation: read latency improves at c1, regresses at c64

**Keep selected CRC behavior. Do not promote this candidate.** Moving outbound
Raft work onto an additional per-node executor improves c1 GET throughput by
4.159%, but loses 4.415% c64 GET throughput and 2.066% c64 mixed throughput.
Both repetitions have the same direction. C64 GET means and tails worsen,
including the separate GET population under mixed load. This workload tradeoff
does not satisfy the default read-performance gate.

Candidate source is [`36ae89a`](https://github.com/c4pt0r/kv9/commit/36ae89a774131b368cf1ed28e95df02ba325c8d4).
Its two public workers retain inbound Raft and public RPC handling; existing
outbound peer tasks/connections move to one additional worker. Event interval
eight, adaptive scheduling, process CPU affinity and all transport/consensus
guards stay unchanged. This is three async workers versus two per replica,
not complete Raft networking or CPU isolation. No new queue, per-message task,
port or cluster-wide service is introduced.

## Results

The first complete screen contains 24 cohorts: c1/c64, point GET and 50/50
GET/PUT, selected CRC/candidate/Redis, and two forward/reverse ten-second
repetitions. The fixed v3 clients use 4,096 keys, 128-byte values and closed-loop
load. Clients use CPUs 0--1; the three voters or Redis use CPUs 2--5; helpers
and the three owned background containers use CPUs 6--15,22--31 during timing.
All original container affinity and namespace state is restored.

KV9 uses ordinary Raft quorum and sync calls on **tmpfs WAL**. Redis is standalone
with persistence and pipelining disabled. This is a shared-host loopback memory
diagnostic, not equal durability, real-disk throughput, cross-host latency or
sustained capacity. Quantiles are intervals from pooled original histogram
buckets; percentiles are never averaged.

| Metric | Selected CRC | Outbound isolation | Redis |
| --- | ---: | ---: | ---: |
| c1 GET calls/s | 26,750.692 | 27,863.230 | 173,282.670 |
| c1 GET mean us | 37.265 | 35.774 | 5.693 |
| c1 GET p99 interval us | 49.664--50.175 | 47.616--48.127 | 7.680--7.743 |
| c64 GET calls/s | 345,507.734 | 330,252.289 | 507,232.724 |
| c64 GET mean us | 185.108 | 193.665 | 126.062 |
| c64 GET p99 interval us | 352.256--356.351 | 364.544--368.639 | 229.376--231.423 |
| c64 mixed combined calls/s | 171,768.495 | 168,220.418 | 500,947.685 |
| c64 mixed GET mean us | 384.481 | 395.922 | 127.628 |
| c64 mixed GET p99 interval us | 622.592--630.783 | 679.936--688.127 | 233.472--235.519 |
| c64 mixed PUT/SET mean us | 360.448 | 364.725 | 127.615 |
| c64 mixed PUT/SET p99 interval us | 589.824--598.015 | 638.976--647.167 | 233.472--235.519 |

| Candidate versus CRC | First repeat | Reverse repeat | Pooled |
| --- | ---: | ---: | ---: |
| c1 GET throughput | +3.544% | +4.779% | +4.159% |
| c64 GET throughput | -4.106% | -4.723% | -4.415% |
| c64 mixed throughput | -2.534% | -1.594% | -2.066% |
| c64 mixed GET mean | +3.433% | +2.520% | +2.976% |

C1 mixed combined throughput rises 1.516%; its GET mean falls from 47.226 to
45.214 us, while PUT mean rises from 58.266 to 58.703 us. Complete combined and
separate-operation means/p50/p95/p99, all repeats and outcome populations are
in [READOUT.md](peer-executor-isolation-performance-v1/READOUT.md),
[PER-REPEAT.md](peer-executor-isolation-performance-v1/PER-REPEAT.md) and the
bundled original `statistics/summary.json`. Two repetitions are a screen,
not statistical significance or a universal regression bound.

## What the observation supports

The selected CRC remains about 6.48x below same-run Redis c1 throughput and
1.47x below same-run Redis c64 throughput. The candidate narrows the c1 gap but
widens the c64 gap. Selecting its favorable c1 row would hide the target
high-concurrency and mixed-read regressions.

For c64 GET, aggregate server CPU estimates rise from 3.018 to 3.347 cores,
despite lower throughput. The ratio of observed CPU to measured QPS rises
16.024%. These CPU estimates come from retained resource-coverage intervals;
the ratio is not an exact per-request service time or a causal attribution.
At c1 GET the estimates instead fall from 1.642 to 1.481 cores. The evidence
motivates a bounded comparison of task execution and scheduler/wakeup costs,
distinguishing public workers, outbound peer work and the Raft owner.

The extra worker remains a confounder. A shared three-worker control would be
required to attribute an improvement specifically to placement. Because this
candidate already regresses the c64 acceptance workloads, stop its default
promotion, full-matrix and candidate Chaos expansion. A future scheduling study
must establish a concrete mechanism before another runtime change; do not
repeat the completed frequency sweeps or coarse ReadIndex/queue/body profiles.

## Correctness and provenance

The already published [source and ordinary recovery gate](PEER-EXECUTOR-ISOLATION-VALIDATION.md)
passes 224 default / 234 experimental server tests and doctests (overlapping,
one existing ignored test in each), formatting, all-target Clippy, and complete
stream/unary histories with 356 calls: 328 OK and 28 retained unknowns. Five
server and two client lifetimes exit with six fresh drains. The original clean
release binds 595 source files and 11 freshly compiled first-party units.
These successful gates are reused without test, build or recovery reruns.

This screen passes seven driver/source-binding contracts and 17 independent
auditor contracts, then all 12 short smoke cells. Formal timing completes in
root session **11845**, exit 0 (`129d15`). Independent terminal audit session
**42209** exits 0 (`914027`); the frozen statistics reader exits 0 (`d2854b`).

All **49,236,254 measured calls = issued slots = attempts = successes**.
Across initialization, warmup, measurement and verification, 49,534,310 calls
succeed in 49,534,326 attempts; the 16 extra attempts are retained initialization
routing. No failed, refused, unknown or dropped measured call is hidden.
The audit accepts 80 exited process lifetimes, 48 fresh drains, 48 voter
writer/listener bindings, 2,347 role-source checks and 4,675 resource samples.
Its original retention inventory covers 636 files / 4,525,699,688 bytes.

The exact-delta source argument preserves fresh Safe ReadIndex, sealed groups,
successful pump/apply/view fences, durable quorum commit/apply acknowledgement,
route generations, cancellation, deadlines and reservation ownership. It is
conditional scheduling correspondence, not a full machine-checked Rust/Tokio
or database proof. This performance screen does not add actual Chaos Mesh or
independent host-failure coverage. Those industrial gates remain open.

## Retention preparation and original evidence

Before timing, root reclaims only the reviewed release compiler cache and
1,318 archive-backed extracted third-party Cargo packages. Every selected
package's source bytes match its retained `.crate` archive, whose checksum
matches the retained registry index. Original release files, sources, histories,
WALs, all `.crate` archives and named build/package locks remain intact.
The five retained executables remain distinct and hash-identical.

The first capacity qualification exits 1 because its file-only projection
does not cover its separate artifact-writing reserve. That original result is
retained. A derived plan credits 51,470,336 bytes from 12,453 selected source
directories that root also removes after their files; no archive check is
rerun and no storage guard is reduced. Fresh verification runs while root holds
the named build/package locks. Cleanup session **86949** exits 0 (`57ad22`),
removing 83,382 exact cache files and reclaiming **2,897,895,424 available bytes**.
The 5-GiB combined smoke/timing budget counts the already retained 650,121,216-byte
smoke once; original 96/64-GiB retention and 32/16-GiB tmpfs guards are unchanged.
One excluded registry package remains untouched.

[The reporting bundle](peer-executor-isolation-performance-v1/inventory.json)
contains original-byte screen preparation, retained failed preparation outcomes,
contracts, smoke/cleanup/timing/audit/statistics receipts, raw per-cohort reports,
resource coverage, histogram populations, source/release bindings and cache
restoration metadata. Its [verifier](peer-executor-isolation-performance-v1/verify.py)
checks archive membership and hashes only. Large original runtime observations,
binaries and WALs remain under their original local inventories; the separately
published source/recovery bundle retains its original scope. No failed or
partial timing attempt is pooled. The screen uses one complete timing attempt.

All checks run locally. Hosted workflows remain manual and none is dispatched.
Original issue #9 checkboxes remain unchanged; this experiment closes no broader
proof, Chaos, storage, scale-out or production acceptance item.
