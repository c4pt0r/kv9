# Peer executor CPU and scheduling diagnostic

The outbound-executor prototype remains rejected and CRC `ca0002c7` remains the
selected runtime. This diagnostic narrows the next implementation to redundant
Raft-owner notifications. It does not establish a new throughput result or a
causal explanation for the entire executor regression.

## CPU findings

Two original release executables run separate five-second pure-GET c64 fixtures
with the fixed v3 native client. CPU recording uses 199 Hz `cpu-clock`, a
20-second envelope and 16-KiB DWARF stacks. Both selected windows pass full
measurement containment, 1-ms edge exclusion, 32-bin and sampled-edge coverage,
uniform sampling periods, zero recorded loss and original fixture validation.
No executable is rebuilt for this diagnostic.

| Observation | Selected CRC | Outbound isolation |
| --- | ---: | ---: |
| Selected CPU samples | 2,967 | 3,153 |
| Sampled CPU core equivalents | 2.9831 | 3.1701 |
| Public/inbound/shared executor samples | 2,293 | 1,910 |
| Separately identified outbound executor samples | Not separate | 641 |
| Raft-owner samples | 667 | 595 |
| Main-thread samples | 6 | 6 |
| Unbound transient-thread samples | 1 | 1 |
| RPC/framing/serialization exclusive leaf share | 16.38% | 16.21% |
| Scheduler/synchronization exclusive leaf share | 16.35% | 15.60% |
| Allocation/copy/comparison exclusive leaf share | 13.08% | 11.16% |
| Kernel-network exclusive leaf share | 7.11% | 8.72% |
| Kernel generic-spinlock exclusive leaf share | 3.07% | 4.82% |
| Kernel scheduler/futex exclusive leaf share | 1.85% | 2.32% |

The source-supported thread interpretation is retained separately from the
initial reader's conservative automatic labels. Owner threads can synchronously
call `GrpcTransport::send` and remotely schedule Tokio work; those frames do not
make them outbound or async executor threads. Tokio starts its persistent async
workers through `spawn_blocking`, so a `BlockingTask` frame alone does not identify
a blocking-only worker. Generic `NodeDriver<S,E>::step_inner`, `WorkSignal::wait_until`,
stable task rosters and the original source establish the six owner identities.
No selected `NodeDriver::spawn` frame was recovered. Candidate outbound workers
have both the `kv9-raft-tx` name and the executor/peer-worker stack evidence.
Default thread names alone establish no role. Two transient TIDs remain unbound.

Categories above are partial exclusive leaf populations, not request phases.
Unknown and other symbols remain in the denominator; the unchanged categorizer
does not recognize every allocator symbol. Inclusive stacks overlap. Instrumented
CPU core equivalents describe the sampled population, not exact per-request
service time. The candidate adds a worker per node, and the shared host remains
a confounder. The result supports investigating additional protocol/scheduling
work; it does not justify another thread-count or event-interval sweep.

## Scheduler findings

Two separate fresh c64 GET fixtures capture filtered `sched_switch`,
`sched_waking` and `sched_wakeup` events for a frozen owned-TID roster. Initially
disabled events are enabled and disabled through acknowledged FIFO controls.
The actual acknowledgement brackets fit inside each five-second measurement;
controller deadlines, monotonic conversion uncertainty, original source/fixture
checks and positive zero-loss evidence pass. No CPU-cell request is correlated
with a scheduler-cell event.

| Observation | Selected CRC | Outbound isolation |
| --- | ---: | ---: |
| Actual scheduler slice ms | 250.189 | 249.945 |
| Frozen roster threads | 13 | 16 |
| Retained events | 160,078 | 220,076 |
| Switch events | 73,715 | 94,590 |
| Waking events | 43,181 | 62,743 |
| Wakeup events | 43,182 | 62,743 |
| Complete wakeup-to-run episodes | 43,113 | 62,688 |
| Pooled wakeup-to-run mean us | 7.121 | 5.278 |
| Complete runnable-after-switchout episodes | 6,800 | 11,321 |
| Pooled runnable-after-switchout mean us | 4.961 | 8.055 |
| Nonmain `kv9`-named threads: wakeup-to-run mean us | 1.554 | 4.055 |

The added executor produces more observed switching/wakeup activity. Its pooled
wakeup-to-run mean decreases while the runnable-after-switchout mean increases;
it is incorrect to claim all scheduling waits worsened. The last row explicitly
groups thread names and nonmain identities; the separate CPU/source review
supports the owner construction but this trace has no call stacks to identify
logical tasks. Candidate `kv9-raft-tx` workers have a pooled wakeup-to-run mean of
4.237 us. None of these OS-thread durations is a request latency decomposition.

All 380,154 retained trace events are parsed and accounted for. Equal-time
involvement, unmatched sleep/wakeup endpoints and recording boundaries remain
ambiguous or unclassified. Per-thread distributions and wake execution-context
matrices are retained. A wake execution context is not necessarily the logical
async producer; a frozen roster excludes later-created threads. The raw slices
cannot count no-op futex wake calls, so notification-suppression eligibility
remains a separate implementation measurement.

## Concrete next implementation

`WorkSignal::notify` currently coalesces the pending bit but calls
`Condvar::notify_one` on every notification while not stopped. The installed Rust
futex implementation increments its condition-variable sequence and calls
`futex_wake` for each `notify_one`. Recovered `WorkSignal::notify` stacks account
for 124/2,967 CRC samples (4.179%) and 127/3,153 isolation samples (4.028%), including
futex and kernel synchronization frames. This does not tell us how many calls are
redundant or removable, and is not a projected speedup.

The next candidate should notify only when `pending` changes from false to true
under the existing signal mutex. Keep the selected shared executor, existing
queues, publication-before-notification order, consume-before-drain order,
atomic predicate-to-park, stop wakeups and independent tick deadlines.

The exact proof obligation is small but mandatory. Under `RSParkedSignal`,
`pending /\ phase = park` implies that wake delivery already exists. Therefore
adding `~pending` to the wake disjunction in `RSNotify` and `RSHint` preserves the
abstract next state. Mechanize that correspondence against the existing
[Raft scheduling model and proof](RAFT-SCHEDULING-PROOF.md), including publication
during a drain, multiple producers, bounded retained work, stop and the park
boundary. An informal argument does not complete the implementation proof.

After proof and concrete race/recovery checks, measure suppressed notifications
and syscall/CPU cost, then run the unchanged uninstrumented c1 GET, c64 GET and
c64 mixed screen with operation-specific means and tail histograms. Promote only
if throughput and read latency jointly pass. Fresh Safe ReadIndex, sealed read
groups, complete pump/apply/view fences, admission/deadlines and durable write
acknowledgements stay unchanged. No lease-read shortcut is proposed.

## Validation, retained failures and scope

Four completed fixtures record 6,703,621 measured successful reads. The CPU
fixtures contribute 3,330,063; the scheduler fixtures contribute 3,373,558. The
original native validators, full outcomes, default release/source bindings,
fresh drains, retained tmpfs/WAL identities and owned-process cleanup pass.
The failed scheduler setup adds three exited voter lifetimes, so all attempts
retain 19 exited fixture lifetimes in total. Both isolation wrappers restore the
original container CPU settings and historical namespace identities. No Cargo,
server rebuild or hosted CI runs in this diagnostic.

The original four-cell campaign remains **failed** because the first scheduler
setup rejected the backend's five-byte `ack\n\0` response before launching its
client. An owned helper-only probe confirms that framing, consistent with
[Linux perf's acknowledgement implementation](https://raw.githubusercontent.com/torvalds/linux/v6.17/tools/perf/util/evlist.c).
A derived two-cell scheduler campaign repairs only acknowledgement framing;
it does not replay the two completed CPU fixtures. Four acknowledgement
contracts pass. Fourteen CPU-reader contracts and nineteen scheduler-reader
contracts pass; those totals overlap earlier revisions and are not additive.

Offline analysis retains its original failures: fixture WAL/reports were first
misclassified under the 8-MiB decoder-metadata allowance; the first raw dump
omitted perf's ownership override; the initial parser did not support the
installed libtraceevent pretty switch/wakeup format; and the first complete CRC
parse encountered a final inventory filename typo. Each correction is explicit.
Successful original CRC decodes are reused by verified read-only hardlinks.
No successful raw recording is repeated, no events are selectively discarded,
and no failed campaign is relabeled complete.

The shared 384-MiB allocation cap and 96-GiB retained-filesystem floor remain
unchanged. Decoder gzip outputs have a 40-MiB subcap; decoder metadata has an
8-MiB subcap. Original fixture reports and WAL count toward fixture/total
allocation, not decoder metadata. Failure artifacts count toward total storage.
Raw perf, original executables and WAL remain at their recorded local paths.
The [compact original-byte evidence](peer-scheduling-diagnostic-v1/inventory.json)
and [integrity verifier](peer-scheduling-diagnostic-v1/verify.py) publish the
readers, protocols, decoded populations, source-supported role review, receipts
and complete analysis, including failures. Integrity verification does not rerun
semantic acceptance.

This is shared-host loopback with client CPUs 0-1, server CPUs 2-5 and helpers on
6-15/22-31. KV9 uses three voters with ordinary quorum/sync calls on volatile
tmpfs WAL. It establishes no real-disk durability, independent host failure,
statistical significance, sustained capacity or new Redis comparison. The
[latest uninstrumented screen](https://github.com/c4pt0r/kv9/blob/3ed8661de5e2f5a3a1f5cc1cdf31ac8cd76a655c/docs/PEER-EXECUTOR-ISOLATION-PERFORMANCE.md)
remains authoritative for throughput and request latency. Complete implementation
proofs, the actual Chaos Mesh matrix, storage/availability gates, dynamic
multi-Raft and automatic splits remain open. No original issue #9 checkbox closes.
