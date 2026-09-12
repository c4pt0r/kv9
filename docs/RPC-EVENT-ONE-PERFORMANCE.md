# Event interval one regresses throughput and read latency

Reject `5bed688c3b829770092ee6802078102b894ef6d5` as a performance increment.
The complete forward/reverse screen loses **6.325% c1 GET**, **13.085% c64 GET**
and **5.385% c64 mixed throughput** against selected CRC. Means and p95/p99
worsen in both repetitions, including separate mixed GET latency. Keep CRC
selected; do not expand this setting to the full matrix or candidate Chaos.

The [preceding body-handoff diagnostic](RAFT-BODY-HANDOFF-RESULTS.md) is also
complete. It finds sub-microsecond c1 heartbeat/response offer-to-poll means and
3.244-us mixed leader batches, with larger mixed tails. These local populations
cannot be added into GET latency or assigned to socket/ACK time. Together the
experiments motivate isolating existing outbound Raft execution from public
request work, not further increasing shared-executor polling frequency.

## Exact implementation and comparison

The sole executable delta is event_interval(8) to (1) on the existing two-worker
Tokio runtime. Adaptive global scheduling, allocator, all protocol guards,
bounded peer queues, original coalescing/body stream, deadlines and cancellation
remain unchanged. The observer and earlier held candidates are excluded. The
[conditional safety correspondence](https://github.com/c4pt0r/kv9/blob/5bed688c3b829770092ee6802078102b894ef6d5/docs/RPC-EVENT-ONE.md)
maps unchanged application transitions under changed scheduling; it is not a
proof of Tokio or complete executable refinement.

The 24-cohort protocol retains two ten-second forward/reverse repetitions of
c1/c64 point GET and 50/50 GET/PUT. All roles use 4,096 keys plus sentinel,
128-byte values, seed 71, 128 warmups and closed-loop load. Fixed v3 native and
Redis clients remain unchanged. Native calls have a 1,500-ms deadline and six
maximum SDK attempts. Clients use CPUs 0--1, all three voters/Redis 2--5 and owned
observer/container work 6--15,22--31. No build, test, fault, profile or audit
overlaps timing. Other host services remain unconstrained.

KV9 uses three voters and ordinary quorum/sync behavior on **tmpfs WAL**. Redis
is standalone with persistence and pipelining disabled. This is a shared-host
loopback screen, not equal-durability, disk, cross-host or sustained-capacity
evidence. Two repetitions do not establish statistical significance or a
general optimum. Measured correctness checks do not replace complete histories.

## Pooled results

Rates divide summed completed calls by summed complete cohort elapsed time.
Means use integer duration sums/counts; p99 merges raw histogram buckets and is
reported as an interval. The mixed rows below use separate GET populations.
Both repetitions and PUT/SET statistics remain in the
[complete readout](rpc-event-one-v1/READOUT.md) and
[per-repeat tables](rpc-event-one-v1/PER-REPEAT.md).

| Metric | Selected CRC | Event one | Redis |
| --- | ---: | ---: | ---: |
| c1 GET calls/s | 26,618.801 | 24,935.181 | 173,073.396 |
| c1 GET mean us | 37.449 | 39.987 | 5.701 |
| c1 GET p99 interval us | 49.664--50.175 | 51.200--51.711 | 7.552--7.615 |
| c64 GET calls/s | 345,535.084 | 300,320.897 | 510,848.551 |
| c64 GET mean us | 185.097 | 212.978 | 125.172 |
| c64 GET p99 interval us | 352.256--356.351 | 413.696--417.791 | 231.424--233.471 |
| c64 mixed combined calls/s | 171,793.616 | 162,541.915 | 502,676.242 |
| c64 mixed GET mean us | 384.665 | 408.502 | 127.179 |
| c64 mixed GET p99 interval us | 622.592--630.783 | 671.744--679.935 | 233.472--235.519 |
| c64 mixed PUT/SET mean us | 360.155 | 378.746 | 127.190 |

Candidate throughput changes in repetition 0 / repetition 1 are:

- c1 GET: **-6.937% / -5.708%**; mean latency **+7.481% / +6.077%**.
- c64 GET: **-13.272% / -12.898%**; mean **+15.311% / +14.816%**.
- c64 mixed: **-5.456% / -5.314%**; separate GET means and p99 worsen in both.

C1 mixed throughput also falls 4.313% pooled. Its separate GET mean increases
47.046 to 49.432 us and p99 moves from 62.976--63.487 to 65.536--66.559 us.
No isolated favorable tail or run order offsets the repeated regressions.
More frequent maintenance adds cost, but this screen does not isolate which
executor-internal operation caused the loss. The selected control's same-run
Redis throughput gaps are approximately 6.50x at c1 and 1.48x at c64 GET.

## Correctness, provenance and retained failures

Default and experimental RPC server suites pass 224 and 234 tests/doctests,
respectively, with one pre-existing ignored test in each overlapping suite.
Formatting and all-target experimental Clippy pass. Original release build 2047
exits 0 (`391ae4`); independent source/binary readback passes (`65cab7`), binding
595 source files, 11 freshly compiled first-party units and 20 artifact rows.
All runtime constructor coverage is tied to that retained default binary.

Actual stream/unary leader-loss and original-directory restart histories pass:
**372 calls, 338 OK and 34 unknown**, five server/two client lifetimes and six
fresh drains. Unknown writes are preserved without blind replay. Five process,
seven driver/source-binding and 17 auditor contracts pass. The 12-cell correctness
smoke passes before timing. This is ordinary process recovery, not actual Chaos
or independent-host qualification.

The first full timing session 92656 exits 0 (`1c2b26`). Independent audit 91098
exits 0 (`faf8c2`) and accepts all 24 cohorts, **48,550,288 measured calls**, all
issued, successful and completed in one attempt. Across all phases,
**48,848,344 successful calls / 48,848,360 attempts** preserve 16 initialization
routing attempts. The audit checks 80 exited lifetimes, 48 fresh drains/bindings,
2,347 source checks, 4,679 resource samples and 636 files / 4,447,964,001 bytes.
All three owned container CPU masks and namespace maps are restored exactly.

The first root statistics wrapper stops before invoking the frozen reader:
it expects a `files` envelope while the final inventory is flat. The exact
wrapper failure (`c2d0f7`) is retained; reading the same pinned manifest entries
corrects that wrapper only. The original frozen reader/arithmetic and runtime
inputs remain unchanged; its first invocation passes (`fb4fc8`). Publication
corrects only an inherited display title, preserving the original generated
readout separately. No benchmark or successful test is rerun for these fixes.

Before timing, storage review finds insufficient room above the unchanged 96-GiB
retention floor. Root refreshes the privileged link/reference census and removes
exactly 22,769 reviewed, single-link non-executable Go compiler-cache objects.
Available bytes rise 105,777,442,816 to 110,932,639,744; actual reclaimed space is
5,155,196,928 bytes, leaving 7,853,424,640 bytes above the floor. Original retained
server/client/manifests remain byte-identical. Rust caches, downloads, uv,
evidence and WALs are preserved. The earlier read-only release-cache census's
incorrect historical workload filename and corrected census are also retained;
neither affects runtime acceptance. No guard is lowered or runtime discarded.

## Next structural candidate

The [outbound peer-executor prototype](https://github.com/c4pt0r/kv9/commit/36ae89a774131b368cf1ed28e95df02ba325c8d4) retains public two-worker/event8 behavior
and moves only existing GrpcTransport peer tasks and outbound connections to
one extra node-local worker. Incoming peer RPCs keep their existing listener.
It introduces no new per-message task, queue, network port or cluster service.
Existing route cancellation, progress budgets, ReadIndex and durable writes
remain authoritative. Source, startup/teardown and actual-binary recovery checks
must pass before comparison.

This adds a worker, so a shared-three-worker control is required to attribute
any later gain specifically to isolation. Compare throughput and c1/c64 GET/mixed
means/tails with fixed CPU placement; a promising candidate still needs full
point/batch, applicable proof and actual exact-source Chaos Mesh gates before
promotion. No gain is yet established for the prototype. Dynamic multi-Raft and
automatic splits remain after the read milestone; broader core proof and
industrial availability gates stay open.

The [compact evidence bundle](rpc-event-one-v1/README.md) preserves original
source/build/cache checks, recovery histories, preparation/validators, all
cohort reports, accepted audit and arithmetic. Raw WAL, binaries and bulky host
observations remain local under original inventories. No hosted CI is dispatched.
