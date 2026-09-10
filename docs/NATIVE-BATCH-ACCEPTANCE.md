# Native atomic batch acceptance

The SDK contract is [RAW-BATCH-CLIENT.md](RAW-BATCH-CLIENT.md). This increment
adds a separate bounded correctness workload and atomic history model for that
contract. It does not change the Raft or engine algorithm or establish a batch
performance baseline. Tracking issue: #50; parent roadmap: #9.

## History model

History version 2 adds `batch_get` and `batch_put`; version 1 keeps its original
operation vocabulary. One BatchGet observes an entire positional vector from
one state. One BatchPut folds every ordered pair into one atomic transition.
Duplicate pairs are last-wins; duplicate reads retain their positions. Missing
values and present empty values differ. An unknown write has zero or one whole
effect after invocation, potentially after its unknown return. No partial
effect or second application is allowed. Positive witnesses are replayed.

The 21 batch checker tests include 256 histories compared against a separate
raw-record permutation oracle, plus counterexamples for fractured reads,
partial writes, ordering, duplicates, missing/empty values, late unknown
writes, repeated unknown effects, and point/batch overlap. Seven isolated
single-defect source mutations are rejected by the targeted tests; baseline
and restored sources pass. The unchanged 44 legacy cases and 21 legacy source
mutations pass too. These checks validate the abstract history checker, not
the authenticity of a server receipt or absence of omitted recorder events.

## Recorder and validation

`kv9-batch-workload` uses the normal persistent SDK. It first verifies an absent
dataset, initializes it using BatchPut, runs concurrent batches and overlapping
point operations, drains every issued call, and reads every key again. A
separate immutable empty-value sentinel stays outside generated traffic.
Each traffic nonce determines the operation and complete payload; a separate
key projection avoids the former operation-kind/key-residue correlation.

Every logical call has one invocation and one terminal event. Unknown writes,
all transport attempts, refusal reasons, whole-call latency and attempt latency
are retained. Reservations cover pending terminal records; failed appends do
not publish a successful terminal or release the outstanding reservation.
Worker failures stop new work and drain existing calls. A run with no traffic
cannot report completion. Nine recorder/generator unit tests exercise these
conditions, including an actual `/dev/full` append failure.

The native report validator checks the complete retained history, configuration,
nonce/payload coverage, initialization/final read coverage, per-worker occupancy,
attempt/retry/receipt structure, and independently reconstructed counts and
latency histograms. It then checks and replays the complete atomic history.
Each configured operation must have successful traffic; setup cannot satisfy
this gate. Duplicate keys count as input items while a batch remains one RPC
and one logical latency sample. Latency is never divided by item count.

This is `closed_loop_correctness`. Its measurement-and-drain stage is explicitly
combined; it must not be used to claim a fixed-window QPS or offered-load curve.
Server artifact/lifetime, fault effects and fresh drain observations belong to
the enclosing fixture, not to the client-only report validator.

The validator's 12 tests passed with 61 corruptions rejected, including strict
integer/boolean types, copied identities, receipt/retry structure, missing
events and altered whole-batch item/latency populations. Synthetic positive
controls cover safe redirects, unknown whole effects and supported stop modes.
These fixtures do not execute a server or authenticate a build. Evidence is
`/tmp/kv9-native-batch-report-controls/attempt2`; attempt 1 retains one test-only
expected-error-substring mismatch, corrected without changing the validator.

## Refinement obligations at the API source boundary

The production mapping below names exact `e52e72b4d1020eb078a818f71854d8c179f3dd27`,
also the runtime used by the later documentation-only `f0eaf23` checkpoint.
The new workload/checker does not change these handlers.

| Obligation | Actual source path | Required premise |
| --- | --- | --- |
| Preserve the whole ordered vector and context in one operation | `client.rs` BatchGet/BatchPut arms; `point_wire.rs` dispatch; `grpc.rs::raw_batch_get/raw_batch_put` | Codec, correlation and cardinality validation preserve the request/result relationship |
| Read every key and its routing authorization from one view | `runtime.rs::raw_batch_get -> established_read -> read_view_after_barrier -> check_read_view`; `txn/src/raw.rs::batch_get` | The quorum barrier and established read view supply an immutable state at or after the confirmed barrier; the context check uses that same view |
| Write the whole ordered sequence at one application point | `runtime.rs::prepare_raw_write` BatchPut arm; `txn/src/raw.rs::plan_batch_put`; `Command::fenced_write_from_batch` | Raft commits the command; the apply-time epoch fence and atomic positioned engine application remain valid |
| A successful result identifies this command's complete effect | `runtime.rs::finish_async_proposal_loop -> settle_proposal`; `grpc.rs::applied_response`; client receipt checks | The exact waiter outcome/receipt, not a later applied watermark, binds the command and its term/index |
| Retry cannot create a second effect | SDK strict NotLeader predecessor; server typed Replaced predecessor in `finish_async_proposal_loop` | Each predecessor proves no effect; uncertainty is terminal at both respective retry boundaries |
| Cancellation does not release running write ownership early | `grpc.rs` prepared-write reservation and completion owner; stream pending generation | Detached completion retains its reservation until its terminal outcome; replacing a stream does not replay pending calls |

Conditioned on these premises, a successful BatchGet linearizes at its single
established view: each positional result is the projection of that state, so
duplicate keys agree and intervening point writes cannot fracture the vector.
For BatchPut, induction over the finite ordered pair sequence gives its
last-wins fold; atomic engine application exposes only the before/after states.
The exact receipt places the entire effect before a successful return.
For uncertainty, retaining zero or one whole effect and removing the unknown
return as an effect upper bound preserves executions that settle late. A safe
retry extends a logical invocation with a proved-zero-effect predecessor;
induction over those predecessors leaves at most one effectful attempt.

These are explicit refinement arguments and outstanding machine-checking
boundaries, not a newly discharged TLAPS proof. The existing
[Raw group composition proof](RAW-GROUP-PROOF.md) is bound to `fe650ed` and proves
its stated ordered-fold/composition/receipt properties under its engine and
consensus premises. It does not prove the later Rust adapter, waiter, read view,
transport or whole runtime. The history oracle and real fault tests remain
separate evidence and cannot discharge those source correspondence premises.

## Reproduction and remaining gates

Run locally from a clean candidate checkout. Build/evidence directories must
be new and outside the source tree. No hosted CI dispatch is needed.

```sh
env CARGO_TARGET_DIR=/home/dongxu/kv9/target taskset -c 6-31 \
  python3 -B scripts/build-native-batch.py --output /tmp/kv9-native-build-NEW
taskset -c 6-31 python3 -B scripts/native-batch-e2e.py \
  --build /tmp/kv9-native-build-NEW --output /tmp/kv9-native-e2e-NEW
python3 -B scripts/history/check-controls.py --batch --output /tmp/kv9-batch-controls-NEW
```

The process fixture requires default-feature standalone binaries, ordinary WAL,
three voters, the normal public endpoint, and both omitted/default streaming
and explicit unary arms. Four concurrent workers mix eight-item batches over
four shared keys with point operations. Successful complete BatchGet/BatchPut
calls must fall inside leader-loss and original-directory-restart intervals.
The full histories, exact executable lifetimes, source-bound helpers, healthy
Serving state and fresh zero-admission/read/apply ledgers are mandatory.
All attempts and original errors remain retained if cleanup also fails.

The first retained standalone fixture and independent audit passed on exact
`e246eae6ff48ef1f27dd8f0527b20ae48dc26280`, with 558 inventoried source files
and default features. All five server and two client lifetimes exited; original
source/build/run artifacts remained unchanged. Every voter was healthy Serving
with empty fatal status, drained public/read/apply ledgers, and two fresh status
export advances after each complete client history.

| Full history | Calls | Successful | Unknown | Refused | Traffic batch calls / input items |
| --- | ---: | ---: | ---: | ---: | ---: |
| Default streaming | 188 | 173 | 15 | 0 | 130 / 1,040 |
| Explicit unary | 190 | 173 | 17 | 0 | 132 / 1,056 |

The item counts include duplicate keys. Setup/final reads are included in full
history counts but excluded from traffic batch counts. In the fully contained
leader-loss window, streaming had 9 successful BatchGets and 10 BatchPuts;
unary had 12 and 12. After original-directory restart those counts were 10/12
and 10/11. Every configured point operation also had successful overlapping
traffic. All 32 unknown outcomes are preserved rather than reclassified as
refusals or discarded. Both complete atomic histories and witnesses were
independently checked again.

| Evidence | Location / SHA-256 |
| --- | --- |
| Clean retained build | `/tmp/kv9-native-batch-build-first` |
| First process attempt | `/tmp/kv9-native-batch-e2e-first` |
| Process summary | `974ab33a1b0280807505399c5bd565ad805a7981b1109cdb5065be9ab349cf01` |
| Independent audit | `/tmp/kv9-native-batch-independent-first/audit.json`; `1479aee7fcfa1a8edbd3c459be497cfa16782565de017ac2ba056cd6e92ccac5` |
| Exact counts and fault windows | `/tmp/kv9-native-batch-independent-first/counts-and-windows.json`; `47211dc3f50c7541332a4df273668beff90ec9a2cd455e5a999bdb5a68338dbe` |
| Server executable | `f7e88b6c2f2514748c13fbaf85c3ee0540284138b3f3cc6bd774bdfd17e7eb4e` |
| Native workload executable | `6fb5633bd42cd34c9121d9bd8c04103b146beb79fcad52cb1fc8df41529db6d8` |

The new workload's 9 unit tests and all-target workspace Clippy with the
RPC-experiment feature and warnings denied passed locally. The 65 checker
tests, 28 source mutations and 12 report-validator tests are described above.
No production consensus or storage algorithm changed in this increment.

The first genuine-artifact controls rejected 24/24 mutations, twelve for each
real transport arm. They cover batch result cardinality, successful/unknown
receipts, omitted verification or traffic calls, nonce identity, uncertain
retries, duplicate item counts, unknown whole-batch item counts, whole-call
latency and the immutable empty sentinel. History hashes/byte counts were
repaired in the corrupted copies so semantic checks, not stale digests, reject
them; omitted whole calls also had their ledgers reconstructed. Both original
complete histories passed before and after, and all 29 protected original,
source and build paths remained byte-identical.

Controls: `/tmp/kv9-native-batch-genuine-controls-attempt1/summary.json`, SHA-256
`52663b093ac97329c2495171b1e204b94704e4e594d36162e28b7393879a187c`.
The read-back inventory covers 176 files and 15,447,399 bytes, SHA-256
`81ced920075b67f2d7fb415e24b4462dc639d36b373af3a8e9d2789223bea723`.
These are copied-artifact corruption controls; they do not authenticate a
fabricated receipt cryptographically or add a new process/fault execution.

Actual native-batch Chaos Mesh and dedicated client-link fault acceptance
remain separate gates. The accepted
[normal point Chaos matrix](STREAMING-RPC-CHAOS-ACCEPTANCE.md) is point-only.
Batch-size/load curves, paired Redis MGET/MSET measurements, adapter proof
composition and main promotion remain open in #50.
