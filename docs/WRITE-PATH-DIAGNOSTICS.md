# Bounded write-path observations

The next write experiment needs measured queue and grouping distributions to
explain the [receipt candidate's batch tradeoffs](WRITE-RECEIPT-TAIL-PERFORMANCE.md).
`write-path-diagnostics` adds observations to the selected CRC implementation.
It is disabled by default and does not select the held receipt-tail candidate.
This is an observation increment, not a performance promotion.

## Build and capture

Build an instrumented server with:

```sh
cargo build --offline --locked --release --bin kv9 --features write-path-diagnostics
```

Use the existing bounded build supervisor and a fresh source/space check for a
retained release. This command alone is not a release qualification. The server
exports a compact JSON value on the `write_path_diagnostics=` line of its normal
local `status` file. The value has schema version 1. The existing process-start,
boot, node and exporter identity fields bind the containing status record.
Default builds contain neither the observer nor that status line. There is no
new endpoint, background task, per-request log or diagnostic WAL.

The implementation records all events in an instrumented build. It uses fixed
arrays, with no recording-time allocation. Snapshots allocate only the fixed
inventory: 17 driver distributions, three Ready distributions and a 9 by 9
joint table. Keys, values, request IDs and arbitrary labels are never retained.
The maximum-width JSON inventory is tested to fit below 40 KiB. Recording takes
an additional leaf mutex for driver observations; Ready observations use the
existing peer lock. Consequently, observer overhead must be measured before
using an instrumented run to explain uninstrumented performance.

## What the observations mean

| Observation | Population and boundary |
| --- | --- |
| Committed entries taken per pump | The exact vector drained for application, including no-op and configuration entries. Zero is included for an empty or apply-paused turn. |
| Applied commands per pump | Commands whose apply call returned success, including successful fence-rejection processing. A later failure does not erase earlier successful groups. This is not a count of successful client writes. |
| Successful apply-group commands / encoded bytes | One sample after each successful application group, including singleton non-Raw commands. Encoded bytes count the Raft command payloads. They are not physical WAL bytes or syscall counts. |
| Requests extracted per service | The actual asynchronous queue extracted by the owner, including canceled requests. This differs from the admission reservation count and the retained receipt history. Empty owner turns are included. |
| Inspected / resolved / canceled / expired / requeued / closed | The actual service branches. Resolved means receipt inspection returned a terminal result; it includes replacement, rejection and errors. Canceled includes a receiver already closed by client timeout. Expired means the owner's deadline branch ran after inspection returned pending. Closed counts extracted pending requests dropped when stop races the service pass. |
| Inspection / resolved age | Time since registration after proposal, sampled immediately before inspection. A request inspected repeatedly contributes repeatedly; resolved age covers only the terminal inspection. It excludes proposal submission, client RPC time and completion delivery. |
| Receipt ring length / linear probes | Retained receipts and predicates visited by the actual first-match linear lookup, including misses. These are logical comparisons, not CPU instructions or memory loads. |
| Hit slots / index distance behind tail | Two different measures of a found receipt's position. Log-index gaps need not equal vector-slot distance. A tail index below the queried index contributes no index-distance sample. |
| Ready entries to persist | Entries in one actual Raft Ready; this is distinct from entries committed in that Ready. |
| Ready / LightReady committed entries | Separate counts from the same successfully persisted Ready cycle, before publishing outputs. Failed persistence contributes no successful Ready sample. |
| Applied versus resolved table | Per successful owner turn, rows bucket applied commands and columns bucket resolved asynchronous requests. A zero-apply turn can resolve an earlier receipt; a follower can apply commands with no local waiter. The table measures co-occurrence, not a one-to-one causal mapping. |

Distribution bucket zero is exactly zero; bucket `i > 0` is
`[2^(i-1), 2^i-1]`. Units accompany every distribution. The joint table uses the
same rule through bucket 7 and places all values at least 128 in bucket 8.
Counts, sums and buckets saturate with explicit flags. A poisoned diagnostic
lock marks the driver snapshot invalid and does not fail database work.

Each component snapshot is coherent under its own lock. Driver and Ready
snapshots are independent, and lookup recording can precede the current turn's
aggregate publication. Use fresh drained snapshots at comparison boundaries;
do not assert cross-counter identities on arbitrary live snapshots. Validate
process continuity, schema, units, inventory, validity, every saturation flag,
monotonic deltas and bucket/count conservation before interpreting a capture.
No per-call history or linearizability conclusion follows from these counters.

## Safety argument and implementation mapping

Let `S` contain the original protocol, storage, receipt, queue and deadline
state, and let `O` contain the diagnostic counters. Erase `O` when projecting
an instrumented execution onto database events. Counter updates and snapshot
serialization project to stuttering steps: no diagnostic value is read by an
admission, persistence, quorum, apply, receipt, eviction or deadline predicate.
The extra fields and calls are removed by conditional compilation in a default
build. Observer locks are leaves and acquire no database lock or perform I/O.
Ready counters are copied under the existing peer lock; snapshot allocation is
outside that lock.

The only alternate expression on a receipt-returning path is the observed
linear scan. For every finite receipt vector `R` and requested index `q`, define
`P(e)` as `e.index == q`. Both `R.iter().find(P)` and
`R.iter().position(P).map(|i| &R[i])` return `None` when no predicate is true.
Otherwise, let `j` be the least matching position; both visit the same prefix
`0..=j` and return the same stored entry `R[j]`. This proves equivalence for
empty, gapped, duplicate, reordered and extreme-index vectors without a Raft
ordering assumption. The existing term and exclusive-outcome checks consume
that unchanged entry. A miss continues through the original eviction,
watermark and unknown-result logic. The observation never manufactures a hint
hit, quorum certificate or applied receipt.

In `async_apply::service`, the extracted queue, callback order, reservation
release, sender completion and cancellation/timeout precedence remain the
same. The already-required elapsed-time sample is also recorded; it is not
replaced by a diagnostic clock. In `driver::step_inner`, recording follows a
successful apply and does not move receipt or watermark publication ahead of
that apply. In `rawnode::persist_locked`, recording follows both required
persistence operations and does not move messages or committed entries before
durability.

This is a safety projection, not identical wall-clock behavior: instrumentation
can delay progress, change batching, affect elections or cause existing
deadlines to expire. It does not prove negligible overhead, implementation-wide
liveness, power-loss recovery or cross-host availability. The existing strict
Raft/proof, recovery and actual Chaos requirements remain in force for a
subsequent promoted write optimization.

## Validation and next capture

The new tests exercise histogram overflow and extreme values, poisoned observer
locks, empty and nonempty joint populations, actual cancellation/expiry/stop
outcomes, committed-but-unapplied waits, and real apply groups split at the
128-entry and encoded-byte bounds. The group observations are compared with
the test store's actual writes, and Ready counts include the election no-op.
The server test serializes the real fixed inventory at maximum integer width.

The first instrumented development run passes all 255 Raft and 207 server
library tests, with one existing server test ignored. After an empty-histogram
merge shortcut and a maximum-width JSON test, all five diagnostic tests pass.
The final default run passes 250 Raft and 207 server library tests, with that
same existing ignore. Warnings-denied Clippy passes for the default workspace
and all targets, and separately for the diagnostic Raft/server library and
test targets. Root binary feature wiring and formatting checks pass.
[Exact commands, source identities and original logs](write-path-diagnostics-v1/README.md)
are retained. These are development checks, not a retained release, Chaos run
or matched performance result.

Next, qualify an instrumented release and collect bounded point/batch samples
at c1 and c64. Compare a diagnostic build with its matching default build to
measure throughput and latency overhead separately. Add actual checked-hint,
ordered-fallback and first-match-fallback observations to a separately bound
receipt-candidate build before comparing lookup paths. This baseline schema
does not contain those candidate counts and must not be presented as if it did.
Preserve the original complete eight-smoke/sixteen-timed performance screen
for a subsequent candidate decision. Select a causal change only after these
observations support it. CRC remains selected; no new QPS or batch-regression
explanation is established by this instrumentation increment.
