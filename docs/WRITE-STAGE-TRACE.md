# Bounded write-stage timestamps

The [retained loaded-batch review](write-batch-tail-review-v1/README.md) identifies
an unresolved interval between committed-group preparation and terminal receipt
inspection. Existing per-command apply timers start after decoding/grouping and
overlap within a group. Their sums cannot locate an individual batch's delay.

The new **default-off** `write-stage-tracing` feature records exact-position
timestamps in the existing driver and exports `write_stage_trace` in local
status. It does not select a write optimization. CRC, Safe ReadIndex and every
Raft persistence/quorum/application/response fence remain selected.

## Recorded boundaries

One driver-local monotonic clock timestamps these events:

| Field | Actual boundary |
| --- | --- |
| `prepare_started_ns` | Before decoding the first committed command or acquiring its applied/state-machine locks. |
| `locks_acquired_ns` | After both existing locks return; subsequent prefix decoding and group formation follow. |
| `apply_started_ns` | After grouping and construction of the existing per-command metric timers, immediately before the original apply call. |
| `apply_finished_ns` | Immediately after that call returns, before finishing those timers. |
| `receipts_inserted_ns` | After inserting the group's actual result receipts into the existing ring, while its locks are still held; absent when apply returns an error. |
| `inspect_started_ns` / `inspect_finished_ns` | Around the original asynchronous receipt inspection when it returns a terminal result. |
| `registration_age_ns` | The waiter's existing elapsed-time sample taken before inspection; it is not a new deadline clock. |

Receipt insertion is not lock release, waiter notification, sender delivery or
client acknowledgment. Observation work and those later events can occur after
that timestamp. The original outcomes remain distinct: Applied, Manifest,
FenceRejected, Replaced, Failed and Unconfirmed. A terminal inspection does not
necessarily mean a successful client write.

Each selected command gets its **actual term/index**, group sequence, command
count and encoded byte count. Selection is nonzero log index divisible by 16.
The collector visits the actual group members; it never invents an index or
term between group endpoints. Multiple selected members of one group share its
timestamps. Do not sum those repeated intervals as exclusive work. A command
may itself be BatchPut(64); group size counts commands, not input items.

Both rings hold at most 512 records per driver. Each has a monotonic record
sequence, total recorded count and exact overwritten count. This is bounded
tail retention with deterministic index sampling, not a random sample or a
complete operation history. Group sequence and counters describe formed apply
calls observed by the collector, including unsuccessful calls. A first-command
decode failure before group formation supplies no group record.

The inspection hook records only `inspect_applied` returning `Some`. A canceled
receiver, a pending inspection followed by deadline expiry, close/drop and a
client-side deadline can terminate outside this hook. No trace count certifies
complete terminal-outcome coverage. Keep independent request accounting.

## Loss, clocks and interpretation

Recording uses a fixed array and a leaf `try_lock`, performs no I/O and allocates
nothing. A busy or poisoned observer lock drops that recording call explicitly;
it never waits while holding a database lock. Counter/time overflow or poisoning
invalidates the observation. A busy snapshot has `rows_available=false` and
cannot be interpreted as an empty successful capture. Serialization and vector
allocation happen after copying the fixed arrays and releasing the observer
lock. Maximum-width serialization is tested against 384 KiB.

The first actual capture exposed five lost recording calls on its instrumented
leader. It was correctly refused. Status export now first tries the driver's
existing pump gate: all production group/inspection recording paths already
hold that gate. A busy gate returns an unavailable snapshot; an idle snapshot
holds the gate only while copying fixed arrays, then releases both locks before
allocating vectors or serializing. An exporter therefore cannot own the trace
lock while a driver recording call needs it. Snapshot refusal does not discard
a recording or permit quality-based resampling. See the
[original failure and correction](WRITE-STAGE-CAPTURE.md).

Snapshot begin/end timestamps bracket the fixed-array copy. The row set is
coherent, while the loss/invalid indicators are independent atomic observations.
Bind all records to the node, process incarnation, nonzero `trace_instance` and
unchanged schema. Driver recreation within a process gets a distinct checked
instance counter; exhaustion invalidates tracing without stopping database work.
Clock origins are independent between drivers/processes; do not subtract their
timestamps, even on the same host. No timestamp authorizes a lease or write.

An offline reader must validate fields, sequence continuity within each retained
ring, overwrite accounting, sampling, time ordering, no saturation/poison, and
process continuity before joining by **both term and index**. Report unmatched
records and ambiguous repeated identities rather than choosing a convenient
match. A late registration can be inspected after the corresponding group has
already left the ring. Loss-free retained rows do not imply complete coverage
outside that retained interval.

For an unambiguous same-process join with an inserted receipt, the intervals are
prepare-to-locks, locks-to-apply, apply-to-return, return-to-receipt-insertion,
and insertion-to-terminal-inspection. The last interval also includes actual
lock release, remaining owner work and observation overhead. These are elapsed
intervals, not per-thread CPU or an exclusive decomposition across concurrent
requests. No current native-client histogram contains the matching proposal
identity; a join alone cannot identify its exact p99 call.

Proposal submission, Ready/quorum/scheduling before group preparation and
inspection-to-response delivery remain outside these boundaries. If a future
capture cannot explain the observed delay within this scope, extend the missing
boundary with a separate bounded measurement; do not label the remainder as
network, disk or scheduler cost by subtraction.

## Safety and boundedness argument

Let `S` contain the original driver, Raft, engine, receipt, queue and deadline
state; let `O` contain trace arrays/counters and its clock origin. Project an
instrumented state `(S, O)` to `S`.

1. Every new observer call either updates only `O`, copies it, or drops a record.
   None reads an observation to decide a transition of `S`. These steps project
   to stuttering. In particular, a failed/contended snapshot cannot acknowledge,
   refuse, cancel or retry a database operation.
2. The original apply and inspection calls still execute exactly once, with the
   same arguments and result. Their error branches, receipt insertion order,
   watermark publication and request completion/deadline precedence are retained.
   Erasing the observer calls therefore gives the original ordered transitions.
3. A recording attempt takes no additional blocking lock, acquires no database
   lock and performs no I/O. An exporter tries the existing pump gate before
   the observer lock; writers already take that order. There is no reverse edge
   from either lock to an exporter lock. Snapshot allocation/serialization is
   outside both locks. A successful fixed-array copy can delay the next pump;
   nonblocking acquisition does not establish negligible scheduling overhead.
4. Each ring advances only through checked addition. Before overflow, record
   `n` occupies `(n-1) mod 512`; exactly the last `min(n,512)` records remain.
   The overwritten count is `n-min(n,512)`. Overflow is explicit invalidity and
   never wraps into a false fresh identity. Group iteration is bounded by the
   existing command group, and each row contains fixed-width metadata only.

Thus the instrumentation preserves the database safety projection and bounds
retained trace memory. Conditional compilation removes its clock, arrays,
recording and status field from default builds. This argument does **not** prove
identical scheduling, negligible overhead, liveness or power-loss behavior:
extra work can change grouping, elections or when existing deadlines expire.
The core algorithms and their existing proofs are unchanged; subsequent promoted
optimizations still need their own strict proof and actual Chaos acceptance.

## Local validation and next experiment

Development validation covers actual committed-but-unapplied waits, exact
successful receipt joins, failed group application without receipt publication,
gapped/mixed-term sample identities, both ring overwrites, lock contention,
poisoning, integer/time overflow and the worst-case JSON bound. This is not an
executed performance capture or an observer-overhead result.

The initial `8c00085` qualification has 731 passing default workspace library
tests and 744 with both `write-stage-tracing` and `write-path-diagnostics`. Each leaves four
existing fixture tests ignored. Both configurations pass all-target,
warnings-denied Clippy, and formatting passes. An initial Clippy rejection of
an unused unit binding is preserved with its subsequent correction.

The status export changes the source digest pinned by five existing proof
compositions. Their models and theorems are unchanged; all five pass again with
the reviewed source mapping: 37 theorems / 187 strict obligations, 15 finite
positive models, 29 expected counterexamples, 16 proof rejection controls and
one ordered reachability witness. These are existing checkpoint/retention proof
compositions, not a new mechanized proof of the tracing code. The structural
argument above states the instrumentation's safety scope. See the
[original validation evidence](write-stage-trace-v1/README.md).

The independent reader has 17 passing synthetic controls, including replacement
terms, ring wrap, missing/ambiguous matches, lost records, changed process/trace
identity and malformed input. It accepts two retained raw status files:

```sh
python3 -B scripts/check-write-stage-trace.py \
  --before /absolute/before-status.txt \
  --after /absolute/after-status.txt \
  --output /mnt/data/kv9-work/fresh-trace-result.json
```

The output path must be absent. Each input is limited to 2 MiB, with at most
384 KiB of trace JSON. Only approved identity/trace fields are exported, along
with input and reader hashes. `unique_compatible_pair` permits interval
arithmetic and preserves the original outcome, including `fence_rejected`;
it never certifies client success. Missing/ambiguous/time-incompatible matches
have no interval arithmetic. Every result states that it is not a complete
history or a causal proof. The first actual attempt and exporter-contention fix
are tracked in [the capture report](WRITE-STAGE-CAPTURE.md), including its more
recent tests. The separate [corrected-source capture](WRITE-STAGE-CAPTURE-RESULTS.md)
now accepts all four diagnostic rows with zero recording loss and measured
combined-feature overhead. Earlier overwritten samples remain unavailable.

The corrected pair uses the same source/compiler and requalified client, both
execution orders and separate outcome/lifetime, coverage/loss and overhead checks.
Next attribute the larger state-machine apply region before selecting another
writer or scheduling change. Outputs belong under
`/mnt/data/kv9-work`; active latency-sensitive data keeps its explicitly selected
storage class. Do not repeat an unchanged candidate-selection matrix or promote
the held upper-bound candidate from these diagnostic observations.
