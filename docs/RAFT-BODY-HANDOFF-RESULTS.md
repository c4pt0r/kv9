# Raft request-body handoff: completed diagnostic

The new measurement does not support replacing the 16-slot batch channel as the
primary read-latency optimization. At c1, heartbeat and response batches average
**0.444--0.475 us** from offer to the original request-body receiver yielding.
At c64 mixed load, the leader records **0.987 us** for heartbeat batches and
**3.244 us** for mixed batches. Mixed-batch tails are larger and remain visible.
This is a diagnostic of selected CRC behavior, not a faster runtime or a new
Redis comparison. Selected production behavior remains `ca0002c7`.

The next isolated experiment tests executor maintenance frequency on the existing
two RPC workers: event interval eight to one, preserving adaptive global-queue
scheduling. Its source is [5bed688](https://github.com/c4pt0r/kv9/commit/5bed688c3b829770092ee6802078102b894ef6d5).
More frequent maintenance may reduce busy-executor I/O waits, but also adds work.
It needs a separate uninstrumented throughput/GET-mean/GET-tail comparison.
The rejected fixed-global-queue and direct-peer-stream candidates remain held.

## Exact scope

Source [3c3bd9b](https://github.com/c4pt0r/kv9/commit/3c3bd9b981e00a66ef5b18dbbabfc973da65b878)
adds bounded observation, with its [source correspondence and clock contract](https://github.com/c4pt0r/kv9/blob/3c3bd9b981e00a66ef5b18dbbabfc973da65b878/docs/RAFT-BODY-HANDOFF-DIAGNOSTIC.md).
The clock starts immediately before the original batch-send select and stops
when the original receiver returns the batch, before observer recording. It
includes admission waiting; it is neither pure queue residence nor HTTP/2 flush,
socket delivery or replica acknowledgement. Payload/FIFO, route generation,
queue capacities, coalescing limits, cancellation and progress watchdog remain.

Sampling selects offers 1,65,129,... independently across eight batch classes.
Seven-kind composition arrays and eight independent session-event counters are
retained. Dropped selected samples have unknown duration. Only local yielding
records a Success latency; all six other outcomes remain present and empty.
Sequential counter/histogram snapshots are not an atomic completion ledger.
The unchanged original 26 metrics, schema 2 and 512-KiB export cap remain checked.

Both fixed cells use three voters, native point GET/PUT, 4,096 keys plus sentinel,
128-byte values, seed 71, 128 warmups and five seconds of closed-loop measurement.
The fixed v3 client uses six maximum attempts and a 1,500-ms request deadline.
Clients use CPUs 0--1, voters 2--5 and owned observer/container work 6--15,22--31.
KV9 uses tmpfs WAL with ordinary quorum/sync behavior. These are shared-host
loopback observations, not disk, sustained-load or independent-host evidence.
No build, fault, test or audit overlaps recording.

Snapshots cover the complete drained client envelope, including initialization,
warmup, measurement, verification and external readback. Public drains do not
stop peer heartbeats. Node 2 is leader at both endpoints in both cells; this is
not a continuous-leadership claim. Classes, nodes and earlier diagnostic stages
have different populations and cannot be added into a per-request latency sum.
Observer overhead and deterministic sampling can affect the observations.

## Observed local boundary

Values below are integer sum/count-derived means and p99 bucket intervals.
All 48 class rows, including empty rows, are in [classes.csv](raft-body-handoff-v1/classes.csv).
All 336 outcome rows, compositions, endpoint offsets and events are retained in
the original accepted analysis inside the [evidence bundle](raft-body-handoff-v1/README.md).

| Cell | Node / endpoint role | Batch class | Samples | Mean us | p99 interval us |
| --- | --- | --- | ---: | ---: | --- |
| c1 GET | 2 / leader | heartbeat | 4,408 | 0.444 | 0.512--1.023 |
| c1 GET | 1 / follower | heartbeat_response | 2,204 | 0.475 | 1.024--2.047 |
| c1 GET | 3 / follower | heartbeat_response | 2,204 | 0.463 | 0.512--1.023 |
| c64 mixed | 2 / leader | heartbeat | 480 | 0.987 | 8.192--16.383 |
| c64 mixed | 2 / leader | append | 1,812 | 2.279 | 16.384--32.767 |
| c64 mixed | 2 / leader | mixed | 1,808 | 3.244 | 32.768--65.535 |
| c64 mixed | 1 / follower | heartbeat_response | 198 | 0.787 | 4.096--8.191 |
| c64 mixed | 1 / follower | append_response | 816 | 1.415 | 8.192--16.383 |
| c64 mixed | 1 / follower | mixed | 745 | 1.277 | 4.096--8.191 |
| c64 mixed | 3 / follower | heartbeat_response | 193 | 1.062 | 8.192--16.383 |
| c64 mixed | 3 / follower | append_response | 769 | 1.680 | 8.192--16.383 |
| c64 mixed | 3 / follower | mixed | 726 | 1.195 | 8.192--16.383 |

There are 9,331 completed body samples at c1 and 7,547 at c64 mixed. No selected
unknown-duration drops or session-event increments occur within these endpoint
deltas; teardown after the snapshots is outside that statement. The mixed
tails above remain a limitation of a mean-only reading. They do not establish
a GET-tail causal relationship without request correlation.

## Validation and evidence

Both complete cells pass the independent reader: **984,862 measured calls**, all
successful in one attempt (132,596 c1; 852,266 mixed). Across all client phases,
**1,009,700 calls succeed in 1,009,702 attempts**; two initialization routing
attempts are retained. The reader checks eight exited lifetimes, six fresh
drains, six voter/listener bindings, 12 metric documents, exact source/build
identity and restoration of all three owned container masks and namespaces.

Local source gates pass 442 default Raft/server tests and doctests, 234
experimental server tests and doctests, and two explicit testing-feature
partition regressions. The first two suites overlap and each retains one
pre-existing ignored test. Clippy initially rejects an unused test-only
forwarding helper; removing only that unused method makes the second formatting
and full all-target Clippy gate pass. The original successful runtime tests and
the exact non-behavioral delta are preserved without unnecessary repetition.
The original release freshly binds 596 source files and 11 first-party units.

Ordinary stream/unary leader-loss and original-directory restart histories pass
independent checking: **368 calls, 341 OK and 27 unknown**, all five server/two
client lifetimes exited, with six drains. Unknown writes are retained without
blind replay. Five process contracts, 14 observer-validator contracts and four
fixture contracts pass. This is ordinary process recovery, not actual Chaos Mesh
or host-failure qualification. Whole-implementation proof composition remains open.

Original recording session 52206 exits zero (`d65278`); independent readback
exits zero (`77ef84`). Display derivation exits zero (`95b3af`). The original
retained runtime inventory contains 78 files / 497,469,741 bytes. Compact
publication has a different scope and leaves WALs, executables and bulky host
observations local under their original hashes.

| Artifact | SHA-256 |
| --- | --- |
| Original server | `9c45576e018cb3c9bf2d3cd99fa6eb7bb0c3477613798420007085edd5f75343` |
| Original build manifest | `225b8f7e051a88751faf5f956db8681e9801f9f9840a50ee470ee4dbd4f67316` |
| Retained cache receipt | `dd8cf08dcf6811b6822106e582e2c9a7ef41933dc4598b686950f35dfe02bdf1` |
| Accepted independent analysis | `1018c96678dcd6cf0856582a3fd55afe5820d5c91ee79626eebdd269fd28fac9` |
| Frozen fixture preparation | `c6c14e1a74d89d1b2ec215b8a07a6a9181eee7da8eab9bb86e0f1ba1fdc099df` |

All work is local. No hosted CI was dispatched. No original roadmap completion
checkbox is closed by this diagnostic.
