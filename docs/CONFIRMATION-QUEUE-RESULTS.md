# Read confirmation: local queue observations

The new diagnostic points toward replication message volume and queue scheduling
under mixed load. Successful admission to the local batch channel is small;
sender and receiver queue residence increases substantially under c64 mixed
traffic. These independent message populations cannot be added to reconstruct
a GET or quorum-round latency. The selected runtime remains CRC `ca0002c7`.

## Completed observations

Each cell is one five-second instrumented recording, with fixed v3 client
`0be806d9`, 4,096 keys plus sentinel, 128-byte values, point GET/PUT, seed 71,
128 warmup calls and a 1,500-ms deadline. Samples span the whole drained client
envelope, including initialization, warmup, measurement and verification.
The three voters use ordinary quorum/sync calls on **tmpfs WAL**, on shared-host
loopback. Clients use CPUs 0-1, voters 2-5 and helpers 6-15,22-31.

Each stage/message-kind selects attempts 1, 65, 129, ... independently. Means
below use integer duration-sum/count deltas. Node 2 is the leader in both
endpoint snapshots of both cells; nodes 1 and 3 are followers. Times are us.

| Node and successful local boundary | c1 GET samples | c1 mean | c64 mixed samples | c64 mean |
| --- | ---: | ---: | ---: | ---: |
| 2: sender heartbeat queue | 4,389 | 1.914 | 4,236 | 30.654 |
| 1: sender heartbeat-response queue | 2,195 | 2.037 | 2,118 | 15.195 |
| 3: sender heartbeat-response queue | 2,195 | 2.034 | 2,118 | 15.012 |
| 1: receiver heartbeat inbox | 2,195 | 1.623 | 2,118 | 21.920 |
| 3: receiver heartbeat inbox | 2,195 | 1.662 | 2,118 | 22.633 |
| 2: receiver heartbeat-response inbox | 4,389 | 1.602 | 4,236 | 20.758 |
| 1: batch-channel admission | 2,322 | 0.317 | 1,779 | 0.467 |
| 2: batch-channel admission | 4,644 | 0.288 | 3,886 | 0.268 |
| 3: batch-channel admission | 2,322 | 0.311 | 1,677 | 0.441 |

The mixed leader heartbeat sender p99 falls in 131.072-262.143 us; its
heartbeat-response inbox p99 is 65.536-131.071 us. The common exponential
histogram intentionally gives coarse bounds. A batch entering channel 16 is
not a wire flush, stream-consumption event or remote ACK. Low admission time
does not rule out waiting inside that channel, HTTP/2, the socket or the network.
Sender observations exclude acquisition of the route lock; admitted inbox
observations exclude producer lock acquisition and stop before Raft processing.

The mixed leader observes 1,954,990 outbound Append attempts over the full
envelope, versus 271,134 heartbeat attempts. This is a reason to investigate
replication work, not a proven per-request attribution or bottleneck ranking.
Neither background heartbeats nor initialization traffic is silently removed.
Public/read/apply drains do not quiesce peer transport. The [full readout](confirmation-queues-v1/READOUT.md) retains
all 90 stage/kind rows and all 630 outcome populations, including empty rows,
sample-selection counters and unknown-drop counts.

## Exact implementation and validation

Diagnostic [6530239](https://github.com/c4pt0r/kv9/commit/65302397880f6e580505cff5241b10941479c161)
adds bounded local observations to the selected runtime without changing queues,
route generations, notifications, select arms or Raft decisions. Its
[schema and erasure argument](https://github.com/c4pt0r/kv9/blob/65302397880f6e580505cff5241b10941479c161/docs/CONFIRMATION-QUEUE-DIAGNOSTIC.md)
define the precise local outcomes and non-atomic snapshot boundaries. Removing
the driver's new snapshot accessor reproduces its original source exactly;
138 other runtime/source inputs are identical. This source check is not a
machine-checked whole-implementation proof or timing equivalence.

Local gates pass 441 Raft/server tests and doctests, one ignored, plus 234
experimental server tests/doctests, one ignored in that overlapping configuration,
formatting and warnings-denied Clippy. Twelve extension-validation contracts,
four fixture contracts and five ordinary recovery contracts pass. The first
source attempts exposed a missing direct serialization dependency/lock entry
and an overbroad export-size stress fixture; the corrected gate passes without
raising the 512-KiB cap or omitting any outcome. The original 26-metric bound
remains, and the extension stresses all source-reachable outcomes. All original
failures are retained.

The clean default-feature release binds all 596 source files, 11 freshly
compiled first-party units and 20 artifact observations. New ordinary
three-voter leader-loss and original-directory restart histories independently
pass: stream 190 operations (169 OK, 21 unknown), unary 179 operations (161 OK,
18 unknown). All 369 operations, including the 39 unknowns, remain in the
histories. Five server/two client lifetimes exit, with six fresh voter drains.
This is process recovery, not a Chaos Mesh or power-loss result.

The new diagnostic recording completes **974,650 measured calls**, all successful
in one attempt: 131,944 c1 GET and 842,706 c64 mixed calls. All client phases
contain 999,488 successful logical calls and 999,490 attempts; the two extra
attempts occur during initialization. The independent
readback accepts 8 exited lifetimes, 6 fresh drain documents, 6 voter/listener
bindings and 12 metric documents. It retains 78 runtime files / 492,189,442 bytes
and verifies exact restoration of all three owned containers and namespace maps.
No builds, tests, faults or audits overlap either timed cell. Instrumented call
rates are accounting, not a new performance comparison or Redis result.

Recording session 34360 exits 0 (`16e5d8`); independent readback exits 0
(`fb8779`). Recovery session 67692 exits 0 (`79605f`), followed by independent
audit exit 0 (`fb11d6`). The [compact original-byte evidence](confirmation-queues-v1/README.md)
retains these inputs and terminal receipts; raw binaries, WAL and bulky host
observations remain local under their original inventories. All work is local;
no hosted CI or new Chaos campaign is dispatched.

## Next experiment: bounded Append entry payloads

Source inspection finds that the selected adapter leaves
`max_size_per_msg` at raft-rs 0.7.0's zero default, which limits each Append to
one entry. The documented
[upstream configuration](https://docs.rs/raft/0.7.0/raft/struct.Config.html#structfield.max_size_per_msg)
permits an encoded-entry byte target. A new independent, uninstrumented
[candidate 74b958a](https://github.com/c4pt0r/kv9/commit/74b958a8bcdf25252ab55ba6149876a1cddc0637)
sets this target to 64 KiB; one legal oversized entry still progresses. It
leaves the separate `batch_append` option disabled and adds no batching timer.

The hypothesis is fewer replication messages and less shared transport/owner
work when a suffix is already available. Its source gates pass 436 Raft/server
tests/doctests and 234 experimental server tests/doctests (overlapping, one
ignored in each), formatting and Clippy. The new lag/rejoin test observes actual
multi-entry messages, contiguous indexes, the byte target and oversized-entry
exception, and identical final values across replicas. Its proof mapping keeps
the original Raft AppendEntries, persistence, quorum and Safe ReadIndex rules.
No candidate performance or fault-qualification result is asserted here.

Next run exact ordinary recovery, then the fixed c1/c64 GET/mixed screen with
both repetitions and separate GET tails. Expand qualification only for a useful
candidate. The wider proof/Chaos/host-failure gates remain open. Redis-class
reads still precede dynamic multi-Raft and automatic splits; DPDK remains
conditional on a measured cross-host NIC bottleneck.
