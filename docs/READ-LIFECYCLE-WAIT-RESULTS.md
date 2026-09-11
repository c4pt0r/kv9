# Sampled read lifecycle: confirmation and receiver scheduling dominate

The completed diagnostic on `2ec6fcbdb339d869e2c042d62b7d7dac771eee6e`,
based on the retained jemalloc control, partitions the successful asynchronous
read barrier into four elapsed intervals. Single GET averages **131.316 us**:
**82.119 us** from successful ReadIndex admission invocation to observed group
confirmation, and **37.779 us** from result send to receiver observation.
Those two stages account for about 91% of this sampled barrier duration.
This is diagnostic evidence, not a new throughput result or a hardware ceiling.

## Measured intervals

All means below use differences between fresh, quiescent endpoint counters.
Each API's five histograms have exactly the same successful sample population.
The four stage sums equal the total sum exactly in integer nanoseconds.

| Stage | Single GET mean us | BatchGet(1) mean us |
|---|---:|---:|
| Driver start to successful ReadIndex admission invocation | 7.456 | 7.393 |
| Admission invocation to first exact-group confirmation observed | 82.119 | 83.775 |
| Confirmation observed to successful-result send | 3.962 | 3.876 |
| Successful-result send to receiver observation | 37.779 | 39.261 |
| Total sampled barrier | 131.316 | 134.305 |
| Successful sampled reads | 23,309 | 22,836 |

Node 2 is the observed leader in both fixtures. The other two nodes have zero
sampled successful barriers. No profile error or other non-success outcome
increment occurred. Snapshot validation retains the exact 31-metric inventory,
valid unsaturated counters, histogram conservation and stable node/process/
exporter identity. Fresh drains establish no pending async reads at the endpoint
observations; exports are coherent per metric, independently between metrics.

The confirmation interval includes local admission, peer/pump processing,
transport, quorum progress and the owner's observation. It is not a pure
network RTT. Confirmation timestamps are recorded separately while walking
group members, so earlier member work is included. The next interval includes
remaining pump work, applied-index coverage and completion selection; it is
not pure state-machine apply CPU. Notification includes channel delivery and
task scheduling. Histograms are recorded after the receiver timestamp, while
the existing enclosing backend timers also include recording overhead.

## CPU evidence and interpretation

The unchanged CPU analyzer selects 2,854 single-GET and 2,797 BatchGet(1)
on-CPU samples within their five-second measurement windows. Both recordings
have zero lost samples and all 32 aggregate time bins covered. The selected
single-GET leaf categories include scheduling/synchronization 17.52%,
RPC/framing/buffers 15.56%, allocation/copy/comparison 13.07%, kernel network
7.60%, and engine/storage 2.70%. BatchGet(1) has a similar distribution.
Unclassified other symbols and unknown symbols remain visible in the raw
report; these categories are not a complete causal attribution.

On-CPU percentages are not fractions of end-to-end latency and cannot be
added to the wait-stage percentages. In particular, the small Raft leaf CPU
share does not imply a cheap read barrier. Neither elapsed nor CPU evidence
establishes a NIC/kernel ceiling or the benefit of DPDK.

Endpoint admission counters show 1,503,954 members in 440,571 groups for GET,
and 1,456,148 members in 447,385 groups for BatchGet(1), approximately 3.41 and
3.25 members per group. These include warmup and verification, and are not
group sizes paired with individual latency samples. The maximum admitted
group is 64. Increasing a limit or delaying reads to fill groups is not shown
to improve latency by these observations.

## Next experiment

Follow-up: the [first scheduler screening](RPC-GLOBAL-QUEUE-SCREENING.md)
is complete. A fixed global queue interval of eight regresses throughput,
mean latency and p99 in both repetitions, so that candidate is rejected.
The separate [I/O event interval experiment](RPC-EVENT-INTERVAL-PERFORMANCE.md)
now improves GET by 8.649% / 8.585% in the five-second matched follow-up, with
full local checks and exact-source eleven-window Chaos acceptance complete.
The short screen's second-repeat p99 regression remains documented. The
original diagnostic below remains evidence for completion and confirmation
work; it does not attribute all waiting to either scheduler mechanism.

Prioritize the completion delivery and receiver scheduling path, with a narrow
change on a clean, uninstrumented retained control. Preserve bounded stream
ownership, original cancellation and deadlines, and exact read-group identity.
Also inspect peer-message dispatch and owner wakeups within the confirmation
interval; a further timing split may be needed
before attributing that interval to a specific mechanism. Do not weaken quorum
confirmation, the successful pump requirement or the applied-index fence.

Run focused correctness and process checks before a short matched candidate/
control screen. Compare both GET throughput and mean/p99 latency, retaining
all outcomes and memory observations. Broader correctness and actual Chaos
Mesh acceptance follow only for a useful candidate. Then check writes and
mixed workloads. Redis parity and automatic range splits remain separate,
unfinished gates. The diagnostic branch is not a production optimization.

## Scope and retained evidence

Two owned fixtures ran ordinary three-voter streaming RPC, 64 closed-loop
workers, 128-byte values and five-second read intervals on the shared host.
WAL sync calls remain on volatile tmpfs. Client CPUs were 0–1, voters 2–5,
and profiler/helpers 6–15,22–31. No builds, tests, fault campaigns or audits
overlapped recording. CPU sampling used 199 Hz with a bounded 20-second
recording per fixture. Approximately one in 64 async registrations carries
the lifecycle trace. Instrumentation and profiling can perturb scheduling.
Whole-cohort histogram deltas include warmup and verification; they are not
an exact decomposition of the benchmark's measurement-only end-to-end latency.
Success-only observations do not establish failure-path latency.

Runtime session 44178 exited 0. The unchanged retained CPU analyzer and
fixture readback both exited 0 in session 16177. All 2,943,458 measured calls
succeeded with exactly one SDK attempt per call. Both full nonce-zero datasets,
all six fresh two-export drains, 195 in-window resource observations and all
eight owned fixture lifetimes passed readback. All four owned profiler/helper
lifetimes exited and raw recordings were preserved. No Redis process was used here.

The diagnostic source passed 214 Raft tests, four observability tests,
targeted all-target Clippy and the actual stream/unary leader-kill and
original-directory restart E2E before recording. Existing algorithmic gates
remain in force. No hosted CI was dispatched.

Raw artifacts: `/tmp/kv9-read-wait-profile-run-first`, including protocol,
build bindings, per-API endpoint snapshots, raw perf recordings, selected
samples, CPU analysis and fixture readback. The independent phase review is
retained at `/tmp/kv9-read-wait-profile-independent-first`.
Its first execution passed all 12 endpoint documents and six stable node
lifetimes. All after snapshots follow client exit and the post-readback drain.
The independent analysis binds 31 unchanged inputs; no new runtime was needed.

| Retained artifact | SHA-256 |
|---|---|
| checkpoint-input-hashes.json | `0593847a1c6f3923720b037ac4170ebe4b6b7e329f97826726443d4fbab63761` |
| analysis-summary.json | `5c136e955f102dfe46d5ed5b7dadb324693cd02b38478b2e3558f2785935b7f7` |
| readback-summary.json | `062b1cff9c779886e35f85238cbb2dbd0bcffc8edd6a836586a9ca4d4d4a9550` |
| independent analysis.json | `2df952e7109c3c9cafcab71f877db8969314939ec7e8ad488959d19eea0bd3e2` |
| independent REPORT.md | `48c92970994086a54f3d496e230c963e77e0b593b3fc80e4ba48a74afc2eb30a` |
| independent inventory.json | `4905911a4548cb31b8cc3add8593c12639b313714472d8a5c4235745e5ecc85b` |

Server SHA-256:
`842fad08a7aa6e96a5c325354588111c907ca30a20278e52c50b317c2fd2028e`.
Build manifest SHA-256:
`7542fbd7dd33e93283ecf3c75897d504108213b351e3e23906f8ca5087d6696d`.

The best retained uninstrumented comparison remains
[304,863–305,903 GET/s versus Redis 507,769–508,136 GET/s](JEMALLOC-SERVER-PERFORMANCE.md),
with approximately a 1.66x gap. This diagnostic does not replace that result,
combine the separately validated authorization-metadata optimization, or
promote either candidate to master.
