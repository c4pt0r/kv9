# Completion notification and outbound coalescing proof

Tracking: #41 and #9. This gate is source-bound to
[`b4a74b2dca72d289c9635e9382a44f2c2282e66a`](https://github.com/c4pt0r/kv9/tree/b4a74b2dca72d289c9635e9382a44f2c2282e66a).
It adds two parameterized protocol proofs for the second and third increments
in that revision's [scheduling contract](https://github.com/c4pt0r/kv9/blob/b4a74b2dca72d289c9635e9382a44f2c2282e66a/docs/RAFT-SCHEDULING.md).
The first owner/wakeup/tick/inbox scheduling proof has a separate source boundary
at `4201d04`; this document does not relabel that proof or later implementations.

The source contract's sentence that future arrivals necessarily belong to a
subsequent outbound batch was too strong. Each `try_recv` observes the queue at
that call; arrivals before a later receive can join this batch. The model follows
the Rust implementation. The documentation clarification in `606b02f` agrees
with this interpretation and does not change the frozen runtime source.

## Protocol boundary and premises

These are deductive proofs of explicit transition systems, with a reviewed
source refinement map. They are not a machine-checked translation of Rust,
CRC proofs, a complete consensus proof, or performance measurements. Neither
model grants persistence, quorum, application, receipt or endpoint authority.
The existing authorities remain premises at the composition boundary.

For completion, one `CompletionSignal` lifetime owns its mutex and condition
variable. The standard mutex/Condvar memory ordering and atomic release/register
semantics, nonpoisoned locks, and nonpanicking bounded local operations are
premises. Waiters observe the generation before checking observable state. The
state's lock is released before waiting, and publishers release observable locks
before notifying. A generation uses checked arithmetic and never resets within
that lifetime. Restart requires a fresh signal and invocation identities.

`RCTokens` represents exact authorized observable predicates, not merely an
index or an address. An external state transition supplies an already validated
predicate through `RCData`; notification cannot supply one. A token includes the
identity and verdict required at its source wait site. `RCEvict` may remove a
retained predicate at any time. Each abstract waiter represents one invocation,
and terminal results are immutable. Other invocations and unrelated state changes
can interleave; they do not transfer authority to the selected invocation.

For outbound, a call begins with one envelope already validated for the captured
session by `receive_for_destination`. The per-peer FIFO queue, immutable captured
`Arc<PeerDestination>`, original client/session binding, and outer route-change
cancellation are premises. Addresses may be reused by distinct identities.
Cancellation can safely abandon a batch; this model does not prove eventual
network delivery, reconnection, first-message selection, or remote acceptance.
Queue operations and encoded-byte sums must be representable in the Rust types.
The proof's arbitrary natural-number weights are not a proof of allocation or
integer-overflow behavior outside those premises.

The [direct peer body candidate](DIRECT-PEER-BODY.md) replaces the historical
`receive_for_destination`/coalescer call boundary with locked prefix inspection
inside `Body::poll_next`. Leading stale entries now also consume the same
128-entry inspection budget. The historical source mappings below remain
versioned evidence; they do not mechanically verify the new per-RPC token,
receiver waker or independent watchdog. The candidate documents those additional
obligations separately while preserving exact route filtering, FIFO output and
the soft byte target.

## Completion safety and progress

`RaftCompletion.tla` separates observable publication, generation notification,
observation, exact-condition checking, atomic registration and wake return. A
publication may follow a condition check or an actual park. All parked waiters
are included in the notification, including waiters sharing the same target.
Several updates may precede one notification; unrelated turns, stop and fatal
paths also produce hints. Spurious wakes inside the predicate loop are stutters.

| Requirement | Predicate or declaration | Source refinement at b4a |
| --- | --- | --- |
| Publication cannot strand a waiter | `RCNoLostCompletion`, `RCRegisteredSignal`, per-action induction | `work.rs:38–64`: observe, publish and predicate/park under one mutex; `driver.rs:357–375`: complete turn before publication |
| Generation cannot wrap or recover from exhaustion | `RCGenerationOrder`, `RCExhaustionResult`, `RCPublication` | `work.rs:47–51`: checked add maps permanently to `None`; `driver.rs:370–374`: otherwise successful turn becomes a fatal persistence error |
| Hints never become exact success | `RCAuthority`, action `RCExactResult`, `RCHintHasNoAuthority` | All five driver wait loops re-read their original authoritative state after observing a generation |
| Evicted evidence is not current evidence | `RCEvict`, `RCExactResult` | Exact write lookup preserves retained-ring and passed-slot outcomes; generation equality/difference does not bypass them |
| All registered waiters receive notification | `RCRegisteredSignal`, arbitrary `RCWaiters` | `CompletionSignal::publish` calls `notify_all`; it does not select one waiter |
| Stop/fatal paths notify without manufacturing success | `RCStopHint`, `RCNoLostFatal`, hint and fatal progress theorems | `driver.rs:567–586`, `1011–1014`: state publication precedes notification; stop alone does not satisfy a receipt |
| A qualifying event eventually causes rechecking | `RCRecheckProgressProof`, `RCHintRecheckProgressProof`, `RCCheckProgressProof`, `RCFatalProgressProof` | Conditional on publication and selected waiter service; wake return re-enters the observation/check loop |

`RCDead = RCMaxGeneration + 1` is a mathematical sentinel for `Option::None`,
not an additional `u64` generation. The proof permits every positive maximum;
the implementation instantiates `u64::MAX`. `rcNotifyOk` records the publication
result and proves exhaustion is never a successful signal publication. The
source's conversion of that error to driver fencing is a separate explicit
refinement step, not an assumption that notification itself persists anything.
A waiter that already observed a valid generation can complete an authoritative
state check concurrently with exhaustion; the signal error never grants that
authority, and subsequent observation or wake return sees the terminal error.

The five condition boundaries are configuration term/index and apply-time
membership (`driver.rs:608`), exact write term/index and typed outcome (`721`),
current-term read admission (`867`), exact read-context incarnation/sequence
confirmation (`891`), and application through the already confirmed read index
(`919`). Admission is not a quorum certificate. The applied watermark alone is
not an exact write receipt or a read-context match. This proof assumes the
existing condition checks implement those authorities correctly; it does not
reprove Ready, Raft, receipt retention, or read-context generation.

Conditional progress uses weak fairness for pending notification and the
selected waiter's return/observe/check steps. `RCChosen` is arbitrary: the
parameterized theorem applies to each waiter under that waiter's fairness,
without a finite waiter bound. It does not promise all waiters finish in one
uniform time bound, eventual success of an unknown write, or successful recovery
of evicted evidence. Fatal state eventually reaches a terminal result under the
same fairness. An already completed successful invocation remains completed.

`RCExpire` abstracts expiration of the original invocation budget; it is not
permission to extend or reset that deadline. The source recomputes remaining
duration from the original start. OS scheduling and lock acquisition can exceed
a wall-clock deadline; this is not a real-time timing proof. A stop hint causes
rechecking, but an unresolved caller may still wait until its original timeout.

## Outbound safety and progress

`RaftOutbound.tla` represents the input FIFO with immutable ordinal IDs, captured
destination identities and weights. Each nonblocking receive advances the
inspected prefix exactly once, including stale work. Accepted entries have an
explicit ordinal-to-output-slot map; a recurrence over prefix totals tracks
exact byte accounting. Concurrent arrivals and endpoint changes can interleave
between receives. The offered client remains the captured session.

| Requirement | Predicate or declaration | Source refinement at b4a |
| --- | --- | --- |
| Only available FIFO prefix is consumed | `ROPrefix`, `ROFIFO`, `ROPopStep` | `grpc.rs:1204–1233`: successive `try_recv` calls and pushes retain order |
| Stale work consumes the count bound | `ROInspectionCounted`, `ROPopStrictVariant` | `for _ in 1..MAX_BATCH_MSGS` advances on every receive, before identity filtering |
| Address equality cannot substitute for identity | independent `ROExactDestination` | `Arc::ptr_eq` compares the captured allocation, including address reuse |
| Byte ledger is exact, with one permitted overshoot | `ROByteLedger` and its inductive recurrence | The loop checks the byte target before accepting another matching envelope |
| A batch cannot rebind to a replacement endpoint | `ROCapturedClient`, `ROTerminalAlways` | `peer_worker`/`peer_session` capture destination and client; no route relookup when offering a batch |
| Coalescing eventually returns under fair local execution | `ROBudgetProgress`, `ROBoundedServiceProgress` | Each pop strictly decreases `ROMaxBatch - roInspected`; empty queue/count/byte limit enables finish |
| A ready batch is eventually offered or cancelled under fair service | `ROOfferProgressProof` | Existing stream execution and route cancellation; no artificial coalescing timer |

`ROMaxBatch` is any positive natural number; production uses 128, including the
first message. The byte target is positive and production uses 1 MiB. One legal
message can cross that target; the proof gives `bytes < target + max_weight`
and exact prefix accounting, not a hard 1-MiB batch limit. The first envelope can
already exceed the target. A zero-weight envelope still consumes a count slot.

The termination argument is natural-number induction over remaining inspection
capacity, not finite-state enumeration and not an assumed loop-termination
axiom. Under weak fairness for local service, every coalescing state reaches a
ready, offered or cancelled state. The queue can receive concurrent arrivals;
it cannot force more than the configured count of inspections. This is a bound
on local iterations, not network latency, stream backpressure, RSS or throughput.
“No artificial delay” means no wait for future envelopes inside coalescing. The
existing RPC/stream budgets and outer cancellation remain independent.

## Local gate and evidence

Run the owned gate with the pinned tools and reserved CPU placement. These
local checks share the host; affinity to logical CPUs 6–31 does not establish
exclusive physical-core isolation:

```sh
taskset -c 6-31 python3 scripts/check-raft-completion-protocol.py \
  --jar /tmp/kv9-p0-tools/tla2tools-v1.7.4.jar \
  --tlapm /tmp/kv9-p0-tools/tlapm-1.6.0-pre-20260731/bin/tlapm \
  --output /tmp/kv9-raft-completion-protocol-new
```

Every proof root has a semantic theorem/assumption/dependency inventory and
pinned standard-module digests. Each execution copies and hashes its exact input,
uses a fresh cache and `--strict --nofp --threads 1`, and requires the exact
nonzero obligation count without warnings or errors. Independent SANY JVMs have
separate temporary namespaces. No shared validator is changed.

Finite configurations exercise two waiters, shared and distinct targets, two
small generation limits, address reuse, stale work, zero-weight envelopes and
byte overshoot. Both fingerprint polynomials must agree on exploration counts.
Temporal runs use TLC `-lncheck final`: full temporal checking after exhaustive
exploration. Positive runs require complete temporal markers and an empty
exploration queue; expected counterexamples require their named violation. The option avoids duplicate interim/final marker pairs; it
does not remove any configured temporal property. Bounded TLC is a model
sanity check and counterexample search, never the parameterized proof.

Protocol controls change one model source file, retain baseline/mutant/restored
triples, require a named finite counterexample and a matching nontrivial failed
proof obligation, and verify byte-identical restoration. Controls cover the
observation/check race, non-atomic registration, absent notification, waking one
waiter, wraparound, successful exhaustion, fabricated and evicted authority,
silent stop/fatal paths, missing fairness, unadmitted queue work, address equality,
unbounded/stale inspections, byte accounting, FIFO overwrite, client rebinding
and waiting for future arrivals. Separate semantic triples reject proof holes
and custom axioms before solver acceptance. Output corruption controls reject
missing completion/temporal evidence and incomplete exploration.

The accepted run is `/tmp/kv9-raft-completion-protocol-second`, terminal exit 0.
The independent audit is `/tmp/kv9-raft-completion-audit.json`.

| Accepted evidence | Result |
| --- | --- |
| Completion proof | 28 declarations, 223 obligations |
| Outbound proof | 18 declarations, 309 obligations |
| Parameterized total | 46 declarations, 532 obligations |
| Finite cases | 84: ten fingerprint runs, three temporal runs, eight witnesses, 63 protocol cases |
| Proof/audit cases | 77: 52 distinct fresh successful proof executions, 21 intended proof failures, four semantic rejections |
| Protocol controls | 21 baseline/mutant/restored triples |
| Semantic controls | Four baseline/mutant/restored triples |
| Output controls | 22 rejected malformed-output variants with baseline/restored checks |
| Independent audit | All 161 case records, copied hashes, source bindings, commands, verdicts, inventories and restorations verified |

The two completion safety configurations explore 55,017 and 153,079 distinct
states, respectively, with both fingerprint choices agreeing exactly. Outbound
FIFO, byte and stale configurations explore 240, 180 and 152 states. These are
finite exploration sizes, not the proof's parameter limits.

The audit compares `work.rs`, `driver.rs`, `rawnode.rs`, `transport.rs`, `grpc.rs`
and the original scheduling contract byte-for-byte with the frozen b4a revision.
The three changed-runtime source hashes relevant to this boundary are:

```text
work.rs   766fe563ceba1f2e398ce14704b9fc232be3b4fa52b0301783036f95f089164a
driver.rs e5cffd675c3c1f9a87753b2e8919492dd0e89d54ce200741110e884cc6bb543d
grpc.rs   9083374c5ac34abac7ed8eb5967b1c5fdb693b397e6f6570131e9691e422ac25
summary   f9dfacfc04583606a8d3bb7bee601a5fc4ebd49ca2eac9943275698140453ad9
```

The local evidence archive retains the original development snapshots/logs,
both finite preflights, both full gate attempts, semantic preflight, accepted
source snapshots, independent audit and a manifest:

```text
/home/dongxu/kv9/target/correctness-evidence/2026-09-09-b4a74b2-raft-completion-proof.tar.gz
bytes: 3,008,013
entries: 2,946
SHA-256: 149c1f445878f47cda520ad95902a714f6f59f8ed32092d48714da529640c6ec
manifest SHA-256: 8c545cc65321e254f2438f083b1325b8eaf8d125483e488eca33423651cf1a5f
```

Every archived entry was read back and compared with its original bytes. Failed
attempts remain failures: early completion proofs lacked the needed observation
ordering invariant or explicit enabledness steps; early outbound proofs needed
FIFO/byte decomposition and temporal induction proof structure. One development
invocation used an unsupported cache flag and exited before proof execution.
The first completion model used a mixed-type terminal sentinel and was rejected.
The first finite preflight rejected duplicate interim/final temporal marker
pairs. The first full gate stopped because its FIFO and fairness matchers named
later formulas than the actual failing slot-update and budget-progress lemmas.
Only those exact matcher expectations changed for the accepted second full gate;
no model/proof source or solver limit changed. Original failed outputs and case
verdicts were not rewritten or relabeled.

This gate runs only local TLA+/TLAPS tools; it adds no runtime test, process,
MinIO, Chaos Mesh or benchmark claim. Later Raw apply grouping, Ready grouped
persistence and receipt lookup changes require their own refinement/composition
gates. No hosted workflow was dispatched.
