# Public backend admission and accounting

Issue [#38](https://github.com/c4pt0r/kv9/issues/38), the first admission increment
of [C03](https://github.com/c4pt0r/kv9/issues/13).

## Contract and units

Every authenticated public RPC must reserve one slot and its protobuf request's
`prost::Message::encoded_len()` before handler conversion or backend submission.
All 25 handlers and all clones of one `Kv9Grpc` share the same node-local ledger.
Reservations cover handler preparation, queued blocking work and running backend
work. There is no admission waiting queue: exhausted capacity is refused at once.
The accounting mutex protects short updates only; backend code runs outside it.

These bytes are an admission weight, not measured allocations. Decoded protobuf
objects, collection overhead, transport buffers, response materialization, engine
state and internal Raft queues are outside this ledger. The existing 16 MiB
per-message transport limit remains separate. This increment does not establish a
bound on process RSS or make response and transport admission unnecessary.

| Environment setting | Default | Meaning |
|---|---:|---|
| `KV9_PUBLIC_MAX_REQUESTS` | 64 | Maximum reserved, queued and running public backend jobs per node |
| `KV9_PUBLIC_MAX_ENCODED_BYTES` | 67108864 | Maximum sum of those requests' encoded lengths |

Both settings must be positive `usize` integers. Invalid, zero and overflowing
values fail startup before runtime/driver threads or storage initialization.
They are local capacity settings, not consensus configuration. Empty protobuf
messages have zero byte weight but still consume a slot. A request larger than
the entire byte budget is refused even when no other work is present.

The wire refusal is `RESOURCE_EXHAUSTED` with exactly one
`kv9-admission-refused` value: `request_count`, `encoded_bytes`, or
`request_too_large`. The server emits no receipt, redirect, partial-write or
read-unconfirmed marker on this response. `admission_refusal` validates the code,
known value, absence of duplicate admission markers and absence of conflicting
protocol markers. Unknown/mixed markers fail closed. The Raw KV CLI prints exactly
`admission_refused=true reason=VALUE` and exits unsuccessfully. A plain transport
failure or `RESOURCE_EXHAUSTED` without this contract does not prove nonexecution.
Clients should use bounded backoff for explicit refusals; this change adds no
implicit retry or deadline extension.

## Ownership, conservation and progress

`Reservation` is private to the public backend boundary, is not `Clone`, and is
required by `BlockingBackend::call`. The handler owns it during validation. On
submission, the actual `spawn_blocking` closure owns it through backend return or
unwind. Dropping the RPC future cannot free capacity for that still-live job,
including a submitted job waiting for a blocking worker. Validation failure or
cancellation before submission drops the unused reservation. A normal return
records completion and an optional backend error; unwinding records an abort.
All releases update gauges and outcome counters atomically under the ledger lock.

Let `L` be a list of live reservation weights, `C` the count limit and `B` the
byte limit. The invariant is `length(L) <= C` and `sum(L) <= B`.

1. The empty list satisfies both bounds.
2. Reservation adds one entry only when its weight fits, the old count is below
   `C`, and `weight <= B - sum(L)`. The pre-state bound makes subtraction safe;
   the checks bound both machine additions by representable configured limits.
3. Release removes exactly one owned list position. Count falls by one and weight
   by exactly that entry's weight. Both remain nonnegative and bounded.
4. Starting queued work, cancelling an RPC whose task still owns its reservation,
   and refusals leave the live list unchanged.
5. Induction over these transitions bounds every arbitrary finite execution.

`proofs/lean/Admission.lean` checks seven declarations: empty-state bounds,
reservation bounds and conservation, release bounds and conservation, preservation
by each step, and reachability induction. Independent invalid controls remove the
count or aggregate-byte premise and must fail the intended arithmetic proof.
This is a local resource theorem; TLA+ remains the primary distributed protocol
specification. It does not replace any Raft, receipt or metadata proof.

| Abstract operation/premise | Rust refinement boundary | Executable evidence |
|---|---|---|
| One live entry per owner | Non-cloneable `Reservation`, private fields, required `BlockingBackend::call` argument | All-handler saturation test; cloned services share refusal capacity |
| Guarded insertion | `PublicAdmission::reserve`, one mutex update | Exact-fill, aggregate-byte, empty-message, oversized and machine-maximum cases; isolated missing-count/byte controls |
| Exact owned removal | `Reservation::drop` | Validation, normal return, backend error and panic cleanup |
| Submitted cancellation is a stutter | Guard moved into the actual blocking closure | One-worker test explicitly polls a queued call once before cancellation; running and queued calls retain both weights; early-release source control fails |
| Consensus/control work has independent admission | Raft service is outside this public ledger | Authenticated public overload and Raft discovery on the same listener and single async worker; actual partitioned-quorum history under pressure |

The source mapping is reviewed and tested, not a machine verification of Rust,
Tokio, allocator behavior or the OS. Eventual capacity reuse assumes queued jobs
are scheduled and backend jobs terminate or unwind. A backend that never returns
retains its reservation indefinitely, deliberately. Admission has no FIFO or
per-tenant fairness guarantee. It bounds retained work but does not prove service
under arbitrarily hostile CPU, network, decode or memory pressure. Those remain
C03/P1 scheduler and resource obligations.

Each node owns its own ledger; no shared coordinator or telemetry process is
required for database service. Consensus availability still depends on a surviving
communicating quorum and its storage assumptions. The one-host Kind acceptance
is not evidence of host-failure independence.

## Measurements and acceptance

The existing atomic status file includes fixed keys for configured limits,
`public_rpc_in_flight`, `public_rpc_queued`, `public_rpc_running`,
`public_rpc_encoded_bytes`, and request/byte peaks. Queued includes reservations
in handler preparation as well as submitted jobs awaiting execution. Each status
render uses one ledger snapshot. Gauges and peaks are exact within this accounting
scope. Per-node lifetime counters saturate at `u64::MAX`, reset on restart, and
never control admission or capacity release.

Five fixed classes (`raw_read`, `raw_write`, `metadata_read`, `metadata_write`,
`transaction`) each export admitted, completed, backend-error, pre-execution
release, backend-abort, count-refusal, byte-refusal and oversized-request counters.
There are no key, tenant, caller, request ID or endpoint metric labels.

Reproduce the local checks with:

```sh
cargo test --locked -p kv9-server --lib admission
python3 scripts/check-admission-controls.py --output /tmp/admission-controls
python3 scripts/check-proofs.py --lean /path/to/pinned/lean --self-test
```

The source controls preserve baseline/mutant/restored sources, hashes and all nine
selected-test transcripts. The complete proof checker also audits the transitive
axioms and refuses proof holes or custom axioms.

The standard `scripts/chaos-mesh-e2e.sh` matrix includes a
`public-admission-overload` window while a real two-way NetworkChaos partition
keeps the old leader isolated. A separate test-image executable uses a persistent
channel and bounded waves of 64 reads against the surviving leader, with an
8-job/2-MiB test budget. It records every request, explicit count refusals,
successful missing-key reads, and an oversized-request refusal. It performs no
mutations. Read keys are a fixed 32-byte repetition of `0xad`, outside the
independent history workload's key range; the oversized probe repeats that byte.

Acceptance retains the exact selected fault and its UID, blocked TCP observations
before/after history progress, pressure start/ready/end times, all RPC outcomes,
before/during/after ledger snapshots and final drain. Four isolated evidence
controls erase refusals, exceed the count bound, remove fault injection, or move
history outside the armed pressure interval; each must fail its intended check. Actual refusals and successful
reads must be observed before starting the independent put/read progress window;
refusals must also occur after that observation while the partition is still
installed. The independent full history must pass its existing witness checker
across all fault windows, including this one. Only exclusive admission refusals become `refused` operations with precommit
evidence in the independent history. Timeouts, mixed output and ordinary RPC
failures remain unknown. The checker cannot invent effects for a proven refusal;
an isolated workload-parser control must reject a timeout carrying refusal text.
This is overload/correctness evidence, not a throughput benchmark.
