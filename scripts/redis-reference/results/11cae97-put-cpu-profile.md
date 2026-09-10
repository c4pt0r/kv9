# Resident runtime: bounded PUT CPU profile

The exact `11cae97` PUT profile shows substantial visible allocation/copy and
RPC work, with specific recovered command-encoding growth and completion-wakeup
stacks. It supports investigating allocation and dispatch costs; it does not
establish that any one change will improve throughput. Most allocation/copy
stacks do not recover a KV9 caller and remain unattributed.

This is **one instrumented, volatile tmpfs diagnostic with concurrent proof
and background load**, not a throughput comparison, A/B result, or durability
acceptance. On-CPU samples omit blocked time and do not measure end-to-end
latency fractions.

## Exact recording and validation

| Artifact | Identity |
|---|---|
| Runtime revision | `11cae977f15df0912c2a35561480d45447bae660` |
| Release executable | `413cfdb20bd947d56a51c792c1d8ab69a676af98a50721649c7b0eceb54f9f33` |
| Unchanged Ready client | `b47d5f408cd9a36ce7a8117917eb0fba530b0616125b4fff38980a255ddd12dc` |
| Actual perf command executable | `/usr/lib/linux-tools/6.17.0-19-generic/perf` |
| Actual perf executable SHA-256 | `a9dde21212f2b4bfa54d66f9c40b4121556367940f0f7c6f7f64c7885e07ff45` |
| Original perf.data SHA-256 | `2f18636ff02a80bbae561501d2049a8de0e7c70b4f473cb64476ae83645b8c92` |

The existing clean release build has features `[]`; all 412 build-manifest
source inputs were rechecked and copied into evidence. The client retains its
own exact Ready `892b2a1` build/source manifest. This task rebuilt or modified
neither runtime nor client, and changed no shared validator.

One PUT100 cohort used 64 persistent workers, 64 hot keys, 23-byte keys,
128-byte values, 32 warmup operations and an eight-second measurement window.
Deadlines remain 1,500 ms, with six maximum attempts for explicit NotLeader
routing and no retry of uncertain writes. The existing workload, complete
outcome and resource-snapshot validators were reused unchanged. Normal Raft
quorum and sync calls remained enabled on three owned tmpfs-backed voters;
tmpfs provides no disk or power-loss guarantee.

The explicit affinity guard observed outer CPUs 0–31, clients 0–1 and all
three voters 2–5 before the measured interval. The recorder ran on CPUs 6–31
and attached only the three owned voter PIDs. It used user-and-kernel
`cpu-clock` samples at 199 Hz, DWARF call graphs with a 16 KiB stack window,
monotonic timestamps, a bounded 20-second recording and a 128 MiB output cap.
Only the client's measured interval is analyzed, excluding one millisecond
at both edges; the two wall/monotonic anchor offsets differed by 666 ns.

The proof scheduler remained active. Four running TLAPM workers were observed
before recording, alongside existing Kubernetes/background processes; the
original process/load snapshots are retained. This is one shared physical
host, and logical CPU masks do not remove every shared resource or profiler
cost. No throughput inference is made from the instrumented operation count.

All **427,101 measured PUTs succeeded**; zero final refusals, unknown outcomes,
transport failures or client rejections occurred. Explicit NotLeader routing
attempts remain in the full client report. The 93,579,196-byte recording
contains 5,320 samples, with **5,203 selected measurement samples**, zero lost
samples and no recording-cap truncation. Each voter has usable samples in all
32 quarter-second measurement bins. Independent decoding of the raw recording
is byte-for-byte identical to the first successful rendering.

## Observed CPU populations

These categories use the sampled leaf symbol. The exact exclusive rules and
complete populations remain in the analyzer; generic allocation, copy and
spinlocks are not silently attributed to a caller.

| Sampled leaf population | Samples | Share of selected samples |
|---|---:|---:|
| Generic allocation, copying and comparison | 1,244 | 23.91% |
| RPC framing, serialization and buffers | 968 | 18.60% |
| Kernel generic spinlocks | 487 | 9.36% |
| Generic scheduler and synchronization | 327 | 6.28% |
| Kernel network symbols | 258 | 4.96% |
| Engine symbols | 246 | 4.73% |
| Kernel futex/scheduler symbols | 188 | 3.61% |
| Raft driver/consensus symbols | 185 | 3.56% |
| Metadata/context symbols | 78 | 1.50% |
| Raft storage symbols | 53 | 1.02% |

The omitted table rows remain in the complete summary: other symbols, other
kernel functions, file/memory I/O, explicit notification leaves and unknown
symbols. `WalSegment::append` itself accounts for 208 sampled leaves (4.00%),
`DiskRaftStorage::write_record_unsynced` for 53 (1.02%), and
`NodeDriver::wait_applied_inner` for 69 (1.33%). These sampled symbol locations
are not a decomposition of all WAL, sync or waiting cost.

Useful actual partial stacks, shown caller toward sampled leaf:

- `CompletionSignal::publish -> futex_wake -> try_to_wake_up -> spin unlock`:
  175 samples (3.36%).
- `blocking::pool::Spawner::spawn_task -> futex_wake -> try_to_wake_up -> spin unlock`:
  71 samples (1.36%).
- `WorkSignal::notify -> futex_wake -> try_to_wake_up -> spin unlock`:
  39 samples (0.75%).
- `driver::push_ring -> memcpy`: 40 samples (0.77%) in one exact recovered
  partial chain, with the next outer caller unknown.

Across all recovered partial stacks, notification symbols occur in 342
samples (6.57%) and blocking-pool symbols in 171 (3.29%). These inclusive
populations overlap the exclusive leaf categories and each other; they must
not be added to the table as independent costs.

## Recovered allocation callers and limits

Among 1,564 samples with allocation/copy/free/grow symbols anywhere in the
partial stack, 382 recover any KV9 symbol. The remaining **1,182 samples have
no recovered KV9 caller**. Known caller populations overlap:

| Recovered KV9 frame in those stacks | Samples |
|---|---:|
| `driver::push_ring` | 58 |
| `MetaTxn::get` | 38 |
| `Tables::region_for_key_in` | 37 |
| `Command::encode` | 23 |
| `WalSegment::append` | 6 |
| `RuntimeBackend::commit_batch` | 6 |
| `wal::encode_batch` | 4 |

The `Command::encode` population includes eight exact
`encode -> grow_one -> finish_grow -> realloc` chains and seven analogous
chains ending in allocator work under realloc. A recovered `wal::encode_batch`
chain also includes `grow_one -> finish_grow -> realloc -> malloc`; another
contains `reserve::do_reserve_and_handle`. These are concrete growth sites in
the sampled executable. They do not establish that all generic allocation
cost belongs to these sites, or that the `commit_batch` samples specifically
represent key/value cloning.

Optimized release unwinding recovers only one symbolized frame in 1,889 of
5,203 samples (36.31%). Async boundaries lose additional causal context.
Missing symbols do not imply zero cost. No instruction offsets from older
executables were reused. `/proc` accounting gives about 3.624 aggregate voter
CPU cores, including 1.395 system cores (38.5%); kernel sampled leaves comprise
25.54% of selected samples. These distinct measurements are not rescaled into
one another or treated as a complete causal accounting ledger.

## Evidence, observer history and cleanup

Evidence is `/tmp/kv9-resident-put-profile-attempt1`. The unprivileged,
PID-scoped permission probe was denied at `perf_event_paranoid=4` and retained;
existing noninteractive sudo access allowed the owned-voter probe and capture
without changing sysctls or sampling unrelated processes. The actual recording,
workload, both successful renders and independent audit passed first attempt.
The first analyzer taxonomy used an overbroad Raft symbol prefix; its exact
source/summary/log remain. Only that category rule was narrowed, with identical
raw recording and selected samples. No second cohort was collected.

`independent-audit.json` records exact source/executable/PID identities,
unchanged workload/resource checks, independent raw decoding, sample coverage
and cleanup. All three voters, the workload client and recorder exited.
Before/after public and async/group occupancy is zero; no reservations,
queued/running backend work, active groups or encoded bytes remained. The
owned tmpfs data was copied and hash-verified before scratch cleanup: 45 files,
491,640,523 bytes. Copies do not change runtime volatility.

The complete inventory is `/tmp/kv9-resident-put-profile-inventory.json`:
**636 files / 643,942,557 bytes**, SHA-256
`53074e32dc2a5010cfcc07f86f510123e20b772aa5b386af59c634f84d7efaf8`.
Every original was reopened and verified against its size/hash; the second
pass is recorded in `/tmp/kv9-resident-put-profile-inventory-verification.json`.
Original perf data, renders, selected samples, caller observations, full client
reports/CPU/histograms, source/build inputs and every observer version remain.
The report itself is excluded to avoid a circular hash. No runtime, shared
validator, GitHub or hosted-CI mutation was performed by the profiling task.
