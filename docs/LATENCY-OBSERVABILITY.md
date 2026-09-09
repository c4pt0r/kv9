# Local latency observations

Tracking: [#39](https://github.com/c4pt0r/kv9/issues/39) and
[#41](https://github.com/c4pt0r/kv9/issues/41), within
[C03](https://github.com/c4pt0r/kv9/issues/13). This increment measures existing
boundaries before changing persistence scheduling or publishing throughput claims.
It adds no batching, asynchronous durability acknowledgement, new consensus
participant, or required telemetry service.

## Representation and resource limits

`crates/common/src/metrics.rs` owns a fixed histogram, measured with Rust
`Instant` in integer nanoseconds. Each latency has seven fixed outcomes, each
with 65 buckets: bucket 0 contains exactly zero; bucket `i > 0` contains
`[2^(i-1), 2^i-1]`. The final upper bound is `u64::MAX`.

The seven outcome names are `success`, `error`, `aborted`, `released`,
`replaced`, `rejected`, and `unconfirmed`. Unsupported outcomes remain empty;
there is no per-key, tenant, request, endpoint, term, index, or context label.
No individual samples or dynamic label registry are retained. A production
node assembles exactly 26 latency objects: ten public, eight driver, four Raft
WAL and four engine WAL observations. Shared `Arc`s do not create additional
histograms. WAL reader/rewrite/reclamation handles retain the engine's original
observer, so replacing the active file does not reset its counters.

Counts and sums saturate independently at `u64::MAX`, with sticky overflow
flags. A duration wider than `u64` nanoseconds is clamped and flagged. The
reported minimum and maximum include clamped values. A poisoned observer mutex
is recovered without returning an error to database code; its snapshot is
marked invalid. Quantiles are unavailable when the observer is invalid, the
count overflowed, the duration clamped, or there are no samples. Sum overflow
alone does not invalidate bucket counts or quantiles.

For percentile `p`, find rank `ceil(count * p / 100)` using `u128`, then return
the interval of the first bucket whose cumulative count reaches that rank.
`p50`, `p95`, and `p99` are intervals, not exact measured percentiles. A zero
sample is valid and has interval `[0,0]`; an empty histogram has null quantiles.

Each metric's arrays and related counters are copied under one leaf mutex.
Allocation, quantile calculation, JSON serialization, and filesystem work occur
after releasing that mutex. The persistent state is constant; export snapshots
contain exactly 26 by 7 by 65 bucket counters. Export is capped at 512 KiB, with
a test filling every numeric histogram field to its maximum width. The release
experiment reports the actual `size_of::<Latency>()` for its compiler/target.
This bound covers observer state and this document, not whole-process memory,
request response buffers, transport buffering, or object-store uploads.

Schema version 2 adds the three pump observations. The current independent
validator requires the exact 26-metric version-2 inventory, including for new
Chaos and benchmark evidence. Historical version-1 reports keep their original
23-metric inventory and pinned checker sources; they are not silently upgraded
or accepted as current evidence. The export byte cap and histogram representation
are unchanged. This is a diagnostic schema change, not a persistent data format.

## Boundary inventory

All counts reset when their owning node component is constructed, normally on
process restart. There is no runtime reset endpoint. `success` means completion
of the named boundary, with the distinctions below; these durations must not be
summed as independent latency components because several boundaries nest.

| Fixed metric name | Included operation and outcome |
| --- | --- |
| `public_{raw_read,raw_write,metadata_read,metadata_write,transaction}_prepare_queue` | From granted reservation through preparation and Tokio blocking-pool queueing to the actual blocking closure's start (`success`). A reservation dropped without execution records `released` at drop, with no backend sample. Transport, authentication, protobuf decoding and refused admissions precede this interval. |
| `public_{raw_read,raw_write,metadata_read,metadata_write,transaction}_backend` | From entry into the actual blocking closure through the backend result, including admission-start bookkeeping, the queue observation, backend locks, consensus waits and storage I/O. `success`/`error` follow the returned result; unwind records `aborted`. The running or queued blocking job owns the reservation even after its RPC future is cancelled. Serialization/transport of the returned response is excluded. |
| `raft_pump_service` | One actual `NodeDriver::step` invocation: fatal-state check, inbound drain/step, Ready persistence, outbound enqueue, read-state capture and committed-entry application. Its return determines `success`/`error`; unwind is `aborted`. The preceding tick call and following idle sleep are excluded. Manual driver steps in tests are included. |
| `raft_pump_idle_wait` | The actual background pump's unconditional `thread::sleep`, including OS overshoot. A returned sleep is `success`; it does not imply work was processed. Stop still waits for this sleep to return. |
| `raft_pump_iteration_spacing` | Time between two entries into the background pump loop before its tick call. The first iteration has no sample. It includes the previous tick/step, observation overhead, sleep and scheduling delay. It is not an actual RawNode tick-delivery timestamp or a configured election-time guarantee. |
| `raft_proposal_submission` | `NodeDriver::propose` and `propose_in_term`: command encoding and the peer proposal call, including peer-lock wait. Accepted submission is `success`, never an applied receipt. NotLeader is `rejected`; other submission errors are `error`. Direct proposal paths outside these driver entry points are excluded. |
| `runtime_logical_proposal_wait` | The runtime's `propose_and_wait` and `commit_catalog` loops, including all submission and exact-receipt waits across safely replaced proposals. One sample per logical loop; earlier replacements do not create extra logical samples. Terminal replacement-budget exhaustion is `replaced`; stale fence or NotLeader is `rejected`; unresolved receipt is `unconfirmed`; machinery or unexpected manifest receipt is `error`. This is not the entire public write, transaction, registration protocol or metadata transaction. |
| `raft_application_wait` | One `wait_applied` invocation, including polling and receipt-lock wait. Exact `(term,index)` receipt matching is unchanged. Applied writes and newly applied manifest changes are `success`; replaced entries are `replaced`; rejected fences and non-new manifest verdicts, including AlreadyApplied, are `rejected`; evicted or unresolved receipts are `unconfirmed`; poisoned drivers are `error`. Bootstrap's short polling calls and manifest callers are included. It is not proposal-to-commit latency. |
| `raft_read_establishment` | One `read_barrier` call: leadership check, unique context, ReadIndex quorum confirmation and local contiguous apply catch-up. NotLeader is `rejected`, either timeout phase is `unconfirmed`, fatal driver is `error`. The later engine snapshot, epoch validation, lookup and response are excluded. |
| `raft_command_apply` | The actual state-machine `apply_at` call, after decoding and after acquiring the applied-ring and state-machine locks. Includes downstream engine/WAL operations. `success` means the command was processed, including a logical rejection that writes nothing; only the separate receipt metric classifies caller acceptance. No-op/ConfChange entries, decoding, lock acquisition and later ring/watermark publication are excluded. |
| `{raft,engine}_wal_record_write` | One framed-record `write_all`, possibly involving multiple short writes. Framing, checksum and serialization, opening and writer-lock acquisition are excluded. A partial-write error is one failed record operation. |
| `{raft,engine}_wal_record_sync` | The actual record `sync_data` (Raft) or `sync_all` (engine) call after a successful `write_all`. Success and error are measured separately. A failed record write must not invoke or invent a sample for this sync. |
| `{raft,engine}_wal_recovery_sync` | The actual repair/open sync following log replay/tail repair. Replay, reads, truncation and opening are excluded. Failed assembly can terminate before a metrics exporter exists. |
| `raft_wal_namespace_publish` | The existing ancestor-publication helper, potentially syncing several directories. One sample measures the whole helper, not one fsync. |
| `engine_wal_namespace_publish` | Existing parent-open/directory-sync operations at WAL opening and after replacing the WAL. Includes directory opening; checkpoint-manifest file write/sync/publication and other storage outside the WAL are excluded. |

Timer boundaries include the small adapter/result-classification overhead between their clock reads. All result observers return the original result or continue the original unwind.
Public admission refusals remain in the separate status ledger; they do not
fabricate reservation, queue or backend durations. A preparation failure released
before execution is deliberately grouped with other unsubmitted releases.

## Local export and snapshot consistency

`<data-dir>/metrics.json` is schema version 1. The normal status pass exports at
most once per second; a fatal driver forces a final attempt before runtime exit.
The exporter builds the document outside its scheduling mutex, writes a temporary
file, then renames it. Diagnostic export is not fsynced and is not crash-durable.
An exporter has no network listener, dedicated authority or external collector.
Its disappearance cannot prevent a quorum from serving requests.

`status` additionally reports `metrics_schema_version`,
`metrics_export_successes`, `metrics_export_failures`,
`metrics_export_failures_saturated`, and `metrics_export_last`. Export successes
saturate; failure overflow has a sticky flag. Last outcome has a fixed inventory:
not attempted, success, inventory/encoding/size/write/rename error. A failure
returns no business error and does not stop the pump or change a receipt. A stale
or absent metrics file must not be read as a fresh health certificate. Wall-clock
capture timestamps and process/node identity support external correlation;
monotonic duration measurements do not depend on wall-clock adjustments.
`exporter_created_unix_ns` and `exporter_uptime_ns` describe exporter construction,
not OS process start or the first sample: WAL initialization can precede exporter
construction. Together with node and process identity they distinguish successive
exports; they are diagnostic fields, not uniqueness or creation authority.

Snapshots are coherent **per latency object**, including all seven outcomes.
Different metrics, the admission ledger, status, and the export envelope are
sampled independently. Do not require conservation across their capture
instants. In particular, capacity can already be released while its final timing
sample is being recorded. A stopped process's final, successfully exported
snapshot is a distinct boundary used by the I/O fault checks.

Apply lag uses a separate observation, not the command-only `applied_index`:

1. Read a peer snapshot containing current term and committed index.
2. Read the driver's atomic `(term,index)` contiguous applied watermark.
3. Read another peer snapshot, without nesting peer and driver locks.
4. Export `committed - driver_applied.index` only if both peer observations have
   the same term and committed index, an applied watermark exists, its term is
   no newer than the peer term, and its index does not exceed commit.

Otherwise `lag_entries` is null, with the raw bracket and paired watermark
retained. Under the existing monotonic term/commit and contiguous-apply
invariants, equal endpoint observations imply that commit and current term were
constant through the intermediate watermark observation. The subtraction is
then a local committed-prefix lag at that observation. It is not a global
snapshot or an exact read-barrier wait prediction. A stable older applied term
is permitted: the watermark can legitimately lag the current leader term.

## Proof boundary

This change does not introduce a new consensus or durability algorithm. Project
away the observer fields and timing/export steps: the authority-bearing
operations remain the existing submission, exact receipt checks, original retry
deadline, ReadIndex barrier, state-machine application, write-then-sync,
persistence-before-publication, and fail-stop transitions. No database branch
reads a histogram or an export result. Observer locks are leaves; collectors
never call database operations or the filesystem while holding them. Delays from
observation remain real execution overhead, not a new bounded-latency theorem.

The existing TLA+ models/TLAPS deductions and Lean admission/resource lemmas
therefore remain the relevant algorithm contracts. This is a source-level
preservation argument plus behavioral/source controls, not a machine-checked
refinement of all Rust code or a proof that observation has zero performance
cost. The proof/model inventories are unchanged. Exact receipt, failed Ready,
read-barrier, poisoned writer, and cancellation tests still execute the real
boundaries. Six isolated Rust mutations test queue time accidentally included
in backend duration, replaced receipts reported as successful applies,
missing observation of the actual fsync call, premature pump completion, failed
pumps reported successful, and idle timing started after the real sleep.
Compilation failures are not accepted as successful mutation detection.

## Reproducible validation

Run ordinary tests and the release experiment separately from throughput work:

```sh
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
python3 scripts/check-latency-controls.py --output /tmp/kv9-latency-controls
python3 scripts/observer-overhead.py --output /tmp/kv9-observer-overhead
```

The experiment records the exact commit, dirty state, source hashes, compiler,
release profile, CPU, affinity and raw trials. It alternates paired baseline
and recording order, with seven trials each at one and eight threads sharing one
histogram. The baseline performs the same clock reads without recording. It
also measures seven sparse and seven dense 26-metric snapshot/JSON trials.
Thread creation is outside the timed interval; barrier release and joins are
inside. Wall time per operation across eight threads is amortized wall time,
not individual call latency. Snapshot/JSON excludes filesystem publication and
the export envelope. Results report variation without a machine-independent
threshold. Concurrent machine activity can affect results and must be reported.

The actual Chaos Mesh runner retains pressure before/during/after snapshots and
six Raft EIO/ENOSPC failure/recovery pairs, alongside fault objects, process
status/exit logs, real connectivity probes and the independently checked full
concurrent history. `check-latency-metrics.py` checks histogram mathematics,
identities, observed pressure completions, real failed writes, absence of an
invented fsync after those writes, and reset after recovery. Invalid histogram,
quantile and missing-boundary evidence controls must fail. This does not cover
Chaos Mesh injection into the engine WAL's fsync; a real failing fsync unit test
and the deterministic Raft filesystem fault matrix cover that operation here.

Accepted exact-revision runs and retained artifacts are recorded on #39 and #9.
C03 remains open for a persistent-connection end-to-end workload matrix,
throughput characterization and the remaining internal/transport/response
resource bounds. Group commit requires its own durability proof and fault gates.

## Scheduling investigation scope

The first #41 increment separates service, actual idle wait and loop spacing.
Each pump call or returned sleep records one fixed histogram sample; the only
new loop-local state is one optional monotonic timestamp. No individual samples,
request identities or dynamic labels are retained. Empty, failed and stopped
populations remain distinct, and snapshots taken while a step is running do not
include its unfinished service sample. A poisoned pump records its failed step
and exits without a new idle sample. Existing histogram saturation/invalidity
rules still reject unsupported quantitative conclusions.

These observations do not change tick frequency, wake the pump, bound its inbound
queue or identify individual request queue residence. Loop spacing includes
service drift and must not be equated with delivered election time. The remaining
local-submission/inbound/outbound observations, synchronized wakeup and independent
tick contract, checked scheduling proofs and paired release experiment remain
part of #41. No performance improvement is claimed by this instrumentation alone.

The [first pump observation report](PUMP-OBSERVATION-VALIDATION.md) retains the
release measurements and exact per-voter/trial populations for this increment.
