# First Raft scheduling proof boundary

Tracking: #41, #13 and #9. This proof is bound to production source
`4201d04d52e5532652a670c7d27df1e8a32106be` and the contract in
[RAFT-SCHEDULING.md](https://github.com/c4pt0r/kv9/blob/4201d04d52e5532652a670c7d27df1e8a32106be/docs/RAFT-SCHEDULING.md). It covers one runtime-assembled
`NodeDriver`/peer/transport lifetime. Later queued-only outbound coalescing,
completion generations, write batching and engine group commit require separate
refinement and acceptance. The 2-ms outbound batching window and 1-ms client
completion polling still exist at this source revision.

## Claims and composition premises

These are parameterized deductive proofs checked by the pinned TLAPS toolchain.
TLC supplies finite coverage, counterexamples and reachability witnesses. It does
not replace any parameterized proof.

The boundary assumes the documented semantics of Rust mutexes, atomics and
condition variables: a signal mutex gives one serialized order; condition-variable
waiting atomically releases that mutex and registers the waiter; a notification
reaches a registered waiter; spurious wakes recheck the predicate. Locks eventually
become available for progress claims. No signal/work lock inversion is modeled:
producers release the data lock before taking the leaf signal lock, and the owner
holds no data lock while waiting. This is an explicit refinement premise checked
against source, not a verified Rust compiler or standard-library implementation.

The single-owner CAS and `pump_gate` are per `NodeDriver` instance. Runtime
assembly must create one such driver for one peer/transport lifetime. The proof
does not exclude two independently constructed drivers sharing the same peer.
The CAS never resets; successful claim does not promise that OS thread creation
will succeed. Manual test iterations use the same pump mutex. Production has no
second tick injector.

Stop can race an already-started turn. The proof prevents a new turn after the
signal observes stop and prevents restart after terminal exit. It permits that
in-flight turn to finish. Existing peer/Ready/application fatal fences are separate
publication authority; a wakeup grants no persistence, quorum or receipt authority.

## Proven boundaries

| Model | Parameterized safety | Conditional progress and limits |
| --- | --- | --- |
| `RaftSchedule` | Publish-before-notify callback ordering; coalesced pending bit; clear before drain; retained-suffix hint; atomic predicate/park; delivered wake for pending/stop at a parked waiter; unique owner claim; monotonic stop and terminal exit. | A queued tracked demand or a pending hint reaches another drain, and a drain reaches its finish, under weak fairness of completed callbacks, owner turns, bounded drains, finish and wake. Stop reaches terminal exit under fair wake/exit. Notification progress does not assume a periodic timeout. |
| `RaftTick` | Monotonic time; no early tick; successive ticks separated by the positive period; due call resets from observed time; traffic leaves tick state unchanged. A long delay produces one coalesced tick, not a catch-up burst. | A due deadline eventually becomes not-due under weak fairness of due-call service. No real-time election bound, clock-scheduling bound or consensus progress is claimed. |
| `RaftInboxBudget` | Guarded message/encoded-byte admission; unchanged admission state on refusal; exact count/byte accounting during a locked prefix drain; per-turn message cap; pre-pop byte target; permitted one-message byte overshoot. Each pop strictly decreases a natural-number variant. | A loop has finitely many pops, including zero-weight messages. Eventual completion additionally requires CPU/lock scheduling and completion of each loop step. No total heap, transport decoder, allocator-capacity or RSS bound is claimed. |

`RSMaxQueue` is an arbitrary positive bound for the tracked inbox work domain;
it is not a claimed bound on every queue or Raft log in the process. `RSHint` models notifications for local proposals, successor Ready, assembly and work outside that tracked inbox without inventing another queued inbox message. Hint progress proves another service turn, not request completion. Producer and
owner sets are nonempty arbitrary sets of positive natural identifiers. The finite
configurations instantiate small domains only. `RaftInboxBudget` abstracts the
counter arithmetic of the actual `VecDeque`: the popped weight is the exact
encoded weight of its front element, and the deque provides FIFO order. It does
not prove the container or protobuf codec. All counters are natural numbers in
the proof; Rust additions/subtractions refine them because admission checks bound
both sums, popped weights belong to the current exact ledger, and the configured
caps fit `usize`.

Tick time uses unbounded integer units for a monotonic clock and positive period.
Representable `Instant + Duration`, positive validated duration and no injected
extra ticks are premises. The finite wrapper restricts the clock horizon only for
TLC. The parameterized theorem has no finite horizon. Passing tick progress does
not prove that a busy OS or blocking synchronous I/O meets a wall-clock deadline.

## Source refinement

| Production source at `4201d04` | Model transition / named proof obligation |
| --- | --- |
| `work.rs`: `WorkSignal::notify`, `begin_turn`, `wait_until` | `RSNotify`/`RSHint`, `RSBegin`, `RSPark`/`RSWake`; `RSNotifyStep`, `RSHintStep`, `RSBeginStep`, `RSParkStep`, `RSWakeStep`; `RSInvariantAlways`. `rsWake` records delivery to an already parked waiter, so a flag written before an invalid non-atomic park cannot invent wake delivery. |
| `work.rs`: `WorkSignal::stop`; `driver.rs`: `stop`, `poison`, `poison_persistence` | `RSStop`, `RSExit`; `RSTerminalStep`, `RSTerminalAlways`, `RSStopProgressProof`. Either outer stop observation or stopped `begin_turn` may clear the pending bit before exit. |
| `driver.rs`: `spawn` claim, `pump_gate`, loop body | `RSSpawn`, `RSBegin`/`RSDrain`/`RSFinish`; `RSSpawnStep`, `RSServiceProgressProof`, `RSFinishProgressProof`. Each whole pump iteration is serialized by the actual mutex. |
| `rawnode.rs`: successful proposal/read/configuration/campaign publication, followed by guard drop and notify | `RSHint` and `RSHintProgressProof` establish notification-to-turn progress for work outside the tracked inbox. Source maps the publication-before-notification leaf-lock contract; refusal does not create a successful proposal/ReadIndex receipt. Application/Ready authority remains in its existing protocols. |
| `work.rs`: `RaftInbox::send` and `drain`; built-in `transport.rs`/`grpc.rs` bindings | `RSPublish`/`RSNotify`, bounded `RSDrain`, retained `RSFinish`; `RBAdmitStep`, `RBPopStep`, `RBBoundedPopAlways`, `RBRefusalPure`, `RBPopStrictVariant`, `RBOvershootBoundary`. The first item crossing the byte target is included, never split. |
| `driver.rs`: `has_pending_ready` after success; transport `set_signal` | `RSHint`/`RSHintStep` and `RSHintProgressProof` cover the extra notifications for successor Ready and initial binding; tracked inbox callbacks still publish before `RSNotify`. Custom transports retaining the trait's default no-op binding are outside notification progress. |
| `work.rs`: `TickDeadline::{new,due,next}`; `driver.rs`: due check inside serialized iteration | `RTAdvance`, `RTTick`, `RTNotDue`, `RTTraffic`; `RTTickSafety`, `RTTrafficIndependent`, `RTTickProgress`. Notifications neither invoke ticks nor change the next deadline. |

Exact production digests:

| File | SHA-256 |
| --- | --- |
| `crates/raft/src/work.rs` | `74311ed391905291774f71145f689e5bc706430ffad6440fb76d1a9cddac572b` |
| `crates/raft/src/driver.rs` | `32f54272d23f1d9d6358930de96f8b02a0f6f5adf8c25edf43451cd3f1bb726c` |
| `crates/raft/src/rawnode.rs` | `3d9de5c7335084f113ff3d095064409dcc788c7183145b9e500ba8b18f9a2042` |
| `crates/raft/src/transport.rs` | `090642e0b487142ef32b3cc9f9d35fcec26fef9d95ba7942664a9e0db9419f58` |
| `crates/raft/src/grpc.rs` | `eafd74455d235be8b28356cecbff7a5a43d606626ac5338118c43433874812a4` |
| `docs/RAFT-SCHEDULING.md` | `4290a10f86ce0460d7d67d131a0c52d8cf1ac0ee42da2f8a753e53029d377819` |

## Reproduction and controls

```sh
taskset -c 6-31 python3 scripts/check-raft-schedule-protocol.py \
  --jar /tmp/kv9-p0-tools/tla2tools-v1.7.4.jar \
  --tlapm /tmp/kv9-p0-tools/tlapm-1.6.0-pre-20260731/bin/tlapm \
  --output /tmp/kv9-raft-schedule-protocol-new
```

The gate verifies pinned tools, the exact semantic dependency/assumption/theorem
inventory, strict fresh-cache proof commands and exact positive obligation counts.
Every protocol fault has a baseline/mutant/restored TLC and TLAPS triple. A fault
must produce its named invariant/action-property/temporal counterexample and a
nontrivial failed proof obligation matching the intended formula. Omitted proofs
and custom axioms are rejected semantically for each root. Output controls reject
empty, incomplete, zero-obligation and unfinished temporal records. Each JVM has a
separate temporary directory for standard-module extraction. A `--model-only`
preflight is labeled as such and never counts as proof acceptance.

The 17 protocol faults remove or invert publication ordering, clear-after-drain,
retained notification, atomic park, stop wake, stopped-turn guard, owner claim,
callback fairness, owner fairness, due guard, coalesced deadline reset, traffic
independence, due-service fairness, inbox count cap, inbox byte cap, turn count cap
and turn byte target. Finite witnesses reach queued work, a coalesced hint,
retained suffix, park, terminal exit, ticks, delayed ticks, zero-weight messages,
byte-target overshoot and retained inbox work.

This proof-only increment runs no Cargo suite, MinIO scenario, process history or
Chaos Mesh campaign, dispatches no hosted workflow, and supplies no throughput
measurement. Those are separate exact-candidate acceptance obligations. Prior
runtime results do not become fresh evidence because these proofs pass.

## Accepted local evidence

The final gate `/tmp/kv9-raft-schedule-protocol-second` exited successfully. Its
independent audit checked all 144 case records, original copied inputs, current
source hashes, semantic dependency/assumption inventories, intended negative
formulas, exact commands and baseline/mutant/restored relationships. The audit
also compared all six source-mapped production/contract files byte-for-byte with
`4201d04`. All tool workloads used logical CPUs 6–31 on the shared host; this is
not exclusive physical-core isolation.

| Evidence | Accepted result |
| --- | --- |
| Parameterized proofs | 55 declarations, 386 obligations: WorkSignal 33/294, ticks 10/47, inbox arithmetic 12/45. |
| Finite full models | Scheduling 1,300 and 1,723 distinct states; tick 29; inbox 714. Each full model passes two independent fingerprints with named action coverage. |
| Fair continuations | Work/extra hints 241 states, terminal stop 3, due ticks 29; completed temporal checks. |
| Protocol controls | 17 TLC and TLAPS baseline/mutant/restored triples; all intended counterexamples and failed obligations observed. |
| Reachability | 10 finite witnesses, including a valid two-state zero-weight message witness. Positive explorations still require at least three states. |
| Proof/audit executions | 72 cases: 49 distinct strict fresh-cache positive executions, 17 intended failed executions and 6 semantic rejections. No prior proof execution is reused or double-counted. |
| Validator controls | 6 semantic baseline/mutant/restored groups and 28 malformed-output controls. |

The evidence archive is
`target/correctness-evidence/2026-09-09-4201d04-raft-scheduling-proof.tar.gz`:
1,881,639 bytes, 2,887 entries, SHA-256
`3d804d264d0b69e7f44e9c6016c953cc635b2ee365f9a32b45352a5b47d01db2`.
Every archived entry was read back and compared with its original bytes. The
manifest SHA-256 is
`986890d13a7512f45228b2b8189d4c3ae977e793541f6aa6c251437ea5911f9a`;
the accepted summary SHA-256 is
`c293d7e0bc2d562ba94326d7096a3939eba7f303c4337eafd7ab8e570052edc7`.
It includes proof/model/configuration/script inputs, frozen production source,
independent audit code/results, logs and failed attempts. This document is kept
in Git rather than inside the archive whose digest it records.

Retained attempts include early syntax/action-specification failures, liveness
witness/decomposition failures, the extra-hint refinement's initially missing
membership bridge, the first non-unique tick mutation anchor, and the first full
gate's overly literal wrapped-formula matcher. That full gate's unfair-callback
mutant correctly failed TLAPS; the harness rejected its unexpected formatting.
Its original result remains failed. The two passing model-only preflights remain
finite preflights. Early ad hoc finite-development logs did not snapshot every
input and are not acceptance evidence; the final gate copies and hashes every
input. `audit/kv9-raft-schedule-attempts.json` inventories these records explicitly.
