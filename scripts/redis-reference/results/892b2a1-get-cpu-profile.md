# Ready GET CPU diagnostic

Exact Ready `892b2a178450309859113c942f1738c070130eb5` has several visible
read-path costs. RPC framing/serialization, allocation/copying and synchronization
all contribute. The receipt vector search is material, but it does not account
for most recorded samples. Actual kernel stacks also show completion publication
and blocking-pool dispatch waking threads through futexes. Metadata/context
validation is a small directly identifiable population in these recordings.
There is no established single bottleneck or optimization gain from this
instrumented diagnostic.

## Frozen runtime and scope

The release executable is the original Ready artifact at
`/tmp/kv9-redis-ready-build/kv9`, SHA-256
`e8232f8c612d92dad68b9209aa55d2d043d5a6fe3b22baac93c6735ee4c25b5d`.
Its source inventory is
`a49402a2fd5e12b97e778b5d4d5186ee58a88a54fef8d24abc42c6072079c31f`.
The unchanged persistent workload executable has SHA-256
`b47d5f408cd9a36ce7a8117917eb0fba530b0616125b4fff38980a255ddd12dc`.
Every tracked source hash and both retained executable hashes were independently
rechecked. All voter execution identities were captured through `/proc/PID/exe`
and bracketed with PID start ticks and the same executable hash before cleanup.
The indexed-read candidate was not used.

Each recording uses one eight-second GET-only cohort: 64 persistent workers,
64 hot keys, 23-byte keys, 128-byte values, 32 warmup operations, unchanged
1,500-ms deadlines and public admission limits. The original fixture provisions
three local WAL-backed voters on owned ephemeral loopback ports. Clients use
CPUs 0–1, servers 2–5 and profiler/harness 6–31. This is one shared physical
host; CPU masks do not establish independent physical resources.

`perf record` attached only to the three owned voter PIDs, at 199 Hz with
`--call-graph dwarf,16384 --clockid mono`, for a bounded 20-second window.
The first recording uses `cpu-clock:u`. Its `/proc` CPU deltas revealed a large
kernel-time population, so the second recording uses `cpu-clock` to capture
user and kernel stacks. Both retain the original complete workload reports,
latency histograms, CPU samples and before/after admission snapshots. Neither
recording is an additional throughput acceptance matrix or a Redis comparison.

## Recorded populations

Only samples inside the client's measurement stage are analyzed. Wall/monotonic
clock anchors were captured before and after each recording; the analyzer
excludes another one millisecond at both edges. Initialization, warmup, drain
and final verification samples are retained in the original recording but
excluded from these tables.

| Recording | Recorded / selected samples | Successful measured GETs | Unsuccessful | Lost samples |
|---|---:|---:|---:|---:|
| User CPU only | 3,522 / 3,496 | 662,851 | 0 | 0 |
| User and kernel CPU | 5,205 / 5,158 | 656,730 | 0 | 0 |

Both clients stopped by duration below their one-million-operation cap. After
drain, all voters reported zero queued/running/in-flight requests and encoded
bytes, with no fatal state. All eight observed server/client process lifetimes
exited. Their measured rates are diagnostic context only: about 82.9k and
82.1k GET/s with profiling overhead present.

The following are exclusive **leaf-sample** categories in the second recording.
The analyzer retains every symbol-matching rule and does not assign generic
allocation or locking samples to an assumed caller.

| Identifiable leaf population | Samples | Share of selected samples |
|---|---:|---:|
| RPC, framing, serialization and buffers | 1,113 | 21.58% |
| Generic allocation, copying and comparison | 767 | 14.87% |
| Kernel generic spinlock functions | 742 | 14.39% |
| User scheduler and generic synchronization | 575 | 11.15% |
| Receipt linear-search instructions | 362 | 7.02% |
| Kernel network functions | 351 | 6.80% |
| Kernel futex/scheduler functions | 274 | 5.31% |
| Metadata/context-related functions | 52 | 1.01% |
| Explicit Raft notification functions | 33 | 0.64% |

Other functions and unresolved symbols complete the population. The small
explicit-notification leaf fraction does **not** bound notification cost:
kernel and mutex descendants are separate leaves.

## Useful actual stacks and instruction evidence

These stacks are shown caller to sampled leaf. Their inclusive populations
can overlap other categories and must not be added to the table above.

- 280 samples (5.43%): `CompletionSignal::publish → syscall → futex_wake →
  wake_up_q → try_to_wake_up → _raw_spin_unlock_irqrestore`.
- 121 samples (2.35%): `blocking::pool::Spawner::spawn_task → syscall →
  futex_wake → wake_up_q → try_to_wake_up → _raw_spin_unlock_irqrestore`.
- 59 samples (1.14%): `read_barrier_inner → syscall → futex_wake → wake_up_q →
  try_to_wake_up → _raw_spin_unlock_irqrestore`.
- 77 samples (1.49%): `read_barrier_inner → Mutex::lock_contended`.
- 48 samples (0.93%) follow the TCP write/loopback receive stack into
  `tcp_data_queue → tcp_data_ready → sock_def_readable → __wake_up_sync_key →
  _raw_spin_unlock_irqrestore`.

Across all recovered partial stacks, 299 samples (5.80%) include
`CompletionSignal::publish`, 414 (8.03%) include a `CompletionSignal` method,
267 (5.18%) include the Tokio blocking pool, and 1,205 (23.36%) include a futex
function. These are observed inclusive counts, not a complete causal partition.
RPC/network functions occur in 2,265 partial stacks (43.91%); metadata-related
functions occur in 139 (2.69%).

The exact release disassembly locates the receipt vector comparison at
`read_barrier_inner+0x470..0x4bc` (exclusive end), virtual addresses
`0x5d1a40..0x5d1a8c`. It advances 32-byte entries, checks the 24-byte context
length, compares 16+8 bytes, then loops on inequality. This corresponds to
`crates/raft/src/driver.rs`'s exact-context `receipts.iter().find`.
In the user-only recording, this region receives 420 of the function's 434
leaf samples: 12.01% of selected user samples. In the kernel-inclusive
recording it receives 362 of 377 function samples: 7.02% of selected total
samples. Timer interrupts can skid; the region identifies aggregate search
work rather than proving an individual instruction's latency.

This supports treating receipt lookup and wakeup/dispatch overhead as real
optimization targets alongside RPC/allocation costs. It does not overturn the
separate indexed-read benchmark's lack of an established end-to-end gain.

## Limits and retained evidence

The optimized binary has symbols and unwind tables but no line-level debug
information. In the user-only recording, 2,079 of 3,496 samples (59.5%) have
only one symbolized frame. Kernel stacks improve the second recording, but
2,086 of 5,158 samples (40.4%) still have one symbolized frame. Async boundaries
also prevent recovering every causal caller. An absent symbol is not zero cost.

`/proc` accounting gives about 3.63 server CPU cores during each cohort, of
which about 1.59 cores are system time (44%). Kernel leaves are 1,609 of 5,158
samples (31.2%) in the second profile. Software timer samples are an approximate
population, not a complete CPU accounting ledger; these two ratios are not
interchangeable. The report does not rescale one into the other or infer that
the sampled ranking uniquely determines total CPU cost. Off-CPU blocked time
is absent. The short run, shared host, optimized unwinding and profiler overhead
also limit extrapolation.

Raw roots are `/tmp/kv9-ready-get-profile-attempt1` and
`/tmp/kv9-ready-get-profile-attempt2-kernel`. Their original `perf.data` SHA-256
values are, respectively:

- `8f62b5ea883dbd5970cd34cb32e908d014ceaf0a0a69b0f53f7241acad45876b`
- `104e68b66cf7d4fca1b3e4902b4bdb75fccca8b4eb3017e5748353233a565ced`

Each root retains its requested protocol, driver copy, build manifests,
executing identities, exact profiler command, original recording, complete
sample rendering, selected samples, summary, original workload evidence and
fresh independent checks. `/tmp/kv9-ready-get-profile-final-audit.json` records
both independent workload checks, exact source/executable verification,
byte-identical independent decoding of the raw recordings, sample counts and
owned-process cleanup. The external drivers/analyzers/auditor are retained as
`/tmp/kv9-ready-get-profile*.py`.

The unprivileged profiler preflight was denied by `perf_event_paranoid=4`;
existing noninteractive sudo access permitted PID-scoped capture without any
sysctl change. One unsupported rendering flag and two root-perf ownership
checks failed before corrected rendering commands succeeded; their original
errors are retained. Both actual workload/recording attempts succeeded. No
runtime, shared validator, unrelated service or Kubernetes resource was changed,
and no hosted CI or GitHub action was invoked by this diagnostic.
