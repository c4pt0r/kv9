# Inbox vector reuse: a small mixed gain with pure-read tradeoffs

Keep CRC `ca0002c7` selected. The isolated
[inbox vector reuse candidate 1045755](https://github.com/c4pt0r/kv9/commit/1045755eac4292a2ccfdfa3d71741ed60e2f4668)
removes redundant vector construction in `GrpcTransport::drain`. Its complete
24-cohort screen records **+0.420% mixed c64 throughput** and a 1.739-us lower
mixed GET mean, with unchanged mixed GET p99. However, **c1 GET throughput falls
0.494%**, with a lower rate and higher mean in both repeats. Pure c64 throughput
is essentially unchanged and pooled p99 is worse. Hold this experiment without
full-matrix/Chaos expansion. The result does not justify a new runtime selection;
it is not a significance test or a no-regression bound.

## Same-run throughput and latency

| Metric | Selected CRC | Inbox vector reuse | Redis |
| --- | ---: | ---: | ---: |
| c1 GET calls/s | 26,725 | 26,593 | 174,388 |
| c1 GET mean us | 37.305 | 37.489 | 5.659 |
| c1 GET p99 us | 49.664-50.175 | 49.152-49.663 | 7.360-7.423 |
| c64 GET calls/s | 345,439 | 345,339 | 513,870 |
| c64 GET mean us | 185.144 | 185.202 | 124.437 |
| c64 GET p99 us | 352.256-356.351 | 356.352-360.447 | 229.376-231.423 |
| c64 mixed combined calls/s | 171,845 | 172,567 | 505,972 |
| c64 mixed GET mean us | 384.352 | 382.613 | 126.373 |
| c64 mixed GET p99 us | 622.592-630.783 | 622.592-630.783 | 231.424-233.471 |
| c64 mixed PUT/SET mean us | 360.245 | 358.867 | 126.338 |

| Workload | Repeat 0 QPS change | Repeat 1 QPS change | Pooled QPS change |
| --- | ---: | ---: | ---: |
| c1 GET | -0.588% | -0.400% | -0.494% |
| c64 GET | -0.241% | +0.183% | -0.029% |
| c1 mixed | +0.061% | +0.213% | +0.137% |
| c64 mixed | +0.255% | +0.585% | +0.420% |

QPS uses summed calls divided by summed complete cohort time. Means pool
whole-call nanoseconds and counts; p50/p95/p99 merge original histogram buckets.
GET and PUT/SET remain separate in mixed traffic. Both forward/reverse repeats
are retained in the [readout](inbox-vector-reuse-v1/READOUT.md) and
[per-repeat table](inbox-vector-reuse-v1/PER-REPEAT.md).

Pooled c1 GET p99 improves by one bucket even though both per-repeat p99 intervals
are unchanged: equal per-repeat quantile buckets do not imply identical raw
distributions. It does not show a repeat-specific tail improvement. Pure c64 GET p99 worsens in repeat 0 and
is unchanged in repeat 1; p95 worsens in both repeats. Mixed c64 GET mean improves
in both repeats while its p99 is unchanged. These distinctions are why the small
mixed gain is insufficient for the read milestone.

## Source, recovery and complete screen

Only `crates/raft/src/grpc.rs` changes among runtime crate sources relative to
selected CRC: 9 insertions and 8 deletions. The held Append-size change is absent.
The inbox already supplies an owned bounded FIFO vector; default builds now
return it directly. Testing builds retain partition filtering in place. The
[source equivalence argument](https://github.com/c4pt0r/kv9/blob/1045755eac4292a2ccfdfa3d71741ed60e2f4668/docs/INBOX-VECTOR-REUSE.md)
covers message order, one unchanged drain, mask observations and unchanged
admission/wakeup behavior. Rust's documented
[`Vec::retain` contract](https://doc.rust-lang.org/std/vec/struct.Vec.html#method.retain)
supports stable filtering with one predicate visit per element. This representation
argument is not a whole-implementation machine proof. Source-level allocation
removal also does not prove how much optimized binary work disappears.

Local release-profile gates pass **435 Raft/server tests and doctests**, with
one existing ignored case; **two explicit testing-feature partition tests**;
formatting and Clippy over all targets with the testing feature. The populations
overlap. A locked, freshly invalidated build binds all 595 source files to the
original clean default release. The candidate source and all retained binaries
remain unchanged throughout acceptance.

Ordinary stream/unary leader-loss and original-directory restart histories pass
independent checking: **359 calls, 330 OK and 29 unknown**. Stream contributes
175 calls (163 OK, 12 unknown); unary contributes 184 (167 OK, 17 unknown).
All five server and two client lifetimes exit, with six fresh voter drains.
Complete unknown outcomes remain in both histories. This is ordinary process
recovery, not actual Chaos Mesh or power-loss acceptance.

Seven driver/source-binding checks and 17 auditor checks pass before runtime;
12 separate smoke cohorts pass. The first complete timing attempt, independent
audit and statistics each pass. All **49,860,618 measured calls = issued =
attempts = successes**; other measured outcomes and dropped slots are zero.
All client phases retain 50,158,674 successful calls / 50,158,690 attempts;
16 extra attempts occur only during initialization routing.

Independent acceptance checks 80 exited lifetimes, 48 fresh drains and
voter/listener bindings, 2,347 role/source checks, 4,675 resource samples and
exact CPU/namespace restoration. It retains 636 files / 4,573,041,910 bytes.
Before runtime, a read-only reference/link census permits reclaiming only the
rebuildable pip HTTP download cache, releasing 6,665,125,888 available bytes.
No installed package, original evidence, WAL or retained binary is removed.
All existing 96/64-GiB retention and 32/16-GiB tmpfs guards remain unchanged.
There is no failed, resumed or discarded timing cohort for this candidate.

The [compact bundle](inbox-vector-reuse-v1/README.md) preserves original source,
release/cache, recovery, harness, raw timing histogram, audit and statistics
bytes. Its verifier checks integrity only; WALs, binaries and bulky host
observations remain local under the original inventories. No exact-candidate
Chaos or full point/batch matrix runs for this held change.

## Environment and next work

The fixed v3 client `0be806d9` uses c1/c64 point read100/read50, 4,096 keys plus a
sentinel, 128-byte values, seed 71, 128 warmup calls, ten-second cohorts and a
1,500-ms deadline. Clients use CPUs 0-1, three voters or Redis share 2-5, and
helpers use 6-15,22-31. KV9 performs ordinary quorum/sync on volatile **tmpfs WAL**;
Redis is standalone with persistence and pipelining disabled. This is shared-host
loopback, not equal-durability, physical-disk, cross-host or sustained capacity.
No builds, tests, profiling or faults overlap timing. No hosted CI is dispatched.

The [Append experiment](RAFT-APPEND-PAYLOAD-PERFORMANCE.md) and this ownership
simplification do not establish a useful read improvement. Stop expanding these
small candidates or varying their knobs without a new cause. The
[request-body handoff plan](RAFT-REQUEST-BODY-HANDOFF-PLAN.md) targets an unmeasured
part of the confirmation path: offering a batch to the existing bounded channel
through the request stream yielding that batch. It preserves the current transport
while identifying whether an extra task handoff merits removal. Low admission
latency alone does not rule out waiting for the consumer to be polled.

Fresh Safe ReadIndex, sealed groups, complete pump/apply/view fences, durable
acknowledgements, cancellation ownership and bounds remain mandatory. Full
proof composition, actual Chaos coverage and independent host-failure acceptance
remain open. Dynamic multi-Raft and automatic splits follow the read milestone.
No broader issue #9 checklist item closes here.
