# Raw engine group composition proof

This proof refines the Raw grouping increment at commit
`fe650ed814757e8172fb03eced102894e35d4a4d`. It concerns one bounded group selected
from an already delivered committed-entry vector, followed by the driver's
remaining-entry completion boundary. A later group can be instantiated with
the current store/SM state and the same prior Ready watermark, under the outer
loop premise that its preceding entries completed. It does not cover the later combined
Ready persistence increment, indexed read receipts, or a change to consensus,
read authority, or storage durability.

The fresh local gate and independent source/log audit passed for this exact
boundary. The parameterized proof has 69 declarations and 1,705 obligations;
TLC supplies separate finite witnesses and counterexamples. This is formal
acceptance of the stated composition contract, not whole-runtime acceptance
of later increments.

## State and refinement boundary

`RawMutation` interprets each mutation as a physical key and either a value or
the absence marker. Positive mathematical key/value identifiers stand for
arbitrary encoded keys/byte values; zero is absence, so an empty byte value is
still an ordinary value. Key identity includes column family, decoded mode,
and keyspace. The encoding/classification correspondence is a source premise,
not a theorem about the Rust decoder.

`RawMutationProof` establishes the finite mutation fold by natural induction,
including its type, empty identity, append recurrence, and concatenation law:

```text
Apply(initial, left ++ right) = Apply(Apply(initial, left), right)
```

The proof also establishes that a batch whose physical keys are all Raw leaves
every System key unchanged. Repeated puts and deletes have their usual ordered
last-mutation effect. No commutativity or disjointness of different Raw commands
is assumed. The sequence induction rule used by the proof is itself derived
from natural induction; the gate does not import `SequenceTheorems`.

`RawGroup` keeps distinct selection, planning, engine effect, engine success,
group publication, later-entry completion, and whole-Ready report states.
Its actual composed batch is compared with an independently defined sequential
reference trace. Fenced commands are adjudicated against the initial store
view during deferred composition; their reference verdicts are calculated
against each preceding sequential state. The System-preservation lemma and
explicit adjudicator footprint premise prove that these verdicts agree.

The model carries each original `(term, index)` and each rejected region ID.
Every prepared member must advance its predecessor's index, have a nonzero
term, and not regress its predecessor's term. Index gaps are allowed. The
engine effect binds the exact selected tail pair to the complete composed
batch, even when every member is rejected and that batch is empty. Each
published member's outcome, position, and rejected region remain its own.

`RGBasePosition` is the **validation floor**: its index represents
`MemStateMachine.applied`, while its term represents the positioned engine's
reported durable term or zero when that authority is absent/volatile. These
components are not claimed to be a previously durable same-entry pair.
`RGBaseDriver` is a separate prior unified driver publication, which may lag
or lead the command-only validation floor. A report replaces it only when the
candidate index increases, preserving the exact candidate pair.

`rgImage` records this group's atomic engine effect; it is not a claim that
later commands cannot overwrite the database. Durable recovery interpretation
requires the durable positioned-store premise below. Before this group has an
effect, `rgImagePosition` is a ghost sentinel containing the validation floor;
it does not assert the engine's previous durable position. `MemEngine` supplies
an atomic volatile effect, not a durable recovery guarantee. The model contains
no physical crash action.

`rgSmPublication` and `rgReceipts` record this group's publication event.
The Rust SM field stores only the index; the model's SM pair is a ghost witness
of the exact local `at` value used by that publication. The positioned engine
and receipt rows carry their own actual position identities.
They are not a persistent cache specification: subsequent entries can advance
the actual state machine and evict actual receipt-ring rows. At publication,
the production 128-member cap fits the 1,024-row command ring. No theorem here
authorizes an evicted receipt or substitutes a watermark for an exact receipt.

## Explicit premises

1. Committed entries and their term/index identities come from the established
   consensus contract. This proof does not establish their Raft commitment.
   The driver owns a fixed, finite input vector and serializes application;
   the state-machine and receipt locks exclude competing publication within
   the modeled call. No new batching timer or future-arrival wait is assumed.
2. The physical Raw/System sets are disjoint. `raw_key` accepts only a complete
   successful decode, Default column family, Raw mode, and a non-System
   keyspace. Every operation in a grouped command passes that predicate.
3. An adjudicator's opt-in truthfully asserts independence from Raw writes.
   The production `CatalogFenceAdjudicator` reads an authoritative System
   region row and applies a pure epoch predicate. The model represents this
   as one System-key read and a fixed predicate per command. Arbitrary
   adjudicators do not receive this premise merely by implementing the trait.
   Correct region-row lookup and the epoch predicate itself belong to the
   existing fence contract; this theorem proves that composition preserves
   their inputs and per-command results.
   Epoch read/decode errors terminate planning; they are not stale/success
   verdicts. Absent rows may produce the specified stale verdict.
4. The `ApplyStore` implementation honors atomic positioned application:
   successful `write_applied(batch, at)` applies the entire ordered batch and
   its one exact position before returning success. A reported error can
   precede the effect or follow an already complete effect; it grants no new
   successful result. For WAL recovery, complete positioned-frame validation
   and durable persistence are separate storage premises. Partial live
   publication by a violating store is outside this composition theorem.
5. Each later entry's application has its own contract. `RGTailSuccess`
   represents that external return, and `RGTailPass` records only ordered
   completion. It deliberately does not interpret metadata, membership,
   singleton, or later-group data/receipt effects. An undecodable later entry
   cannot pass. Entries behind a barrier are not imported into this group.
6. Finite progress requires finite legal input and mutation sequences,
   representable checked positions/bounds, sufficient allocation capacity,
   termination of engine/adjudicator/delegated calls, and weak fairness of
   the enabled owner service. Storage may return an error; fairness does not
   imply eventual storage success or client success. Thread/process crashes,
   permanent storage hangs, allocator failure, and cross-host availability
   are not progress premises supplied by this proof.

The constants remain parameterized under these premises. Instantiation for
this runtime uses 128 entries and 1 MiB of encoded input. The byte ledger
counts each selected command's `entry.data.len()`; total allocation capacity
remains a separate premise. A legal
first command larger than the grouping limit is delegated to the existing
singleton path; the grouping limit is not a request rejection rule. An empty
input to the internal helper is guarded by runtime validation and is outside
the driver's nonempty-group model; an empty **composed mutation batch** is
included.

## Source mapping

All source references in this table mean the exact `fe650ed` tree, even when
this document is later integrated into a different tree.

| Obligation | Protocol / proof | Exact source |
| --- | --- | --- |
| Ordered overwrite/delete/empty composition | `RMApplyConcatenation`, `RMApplyEmpty`, `RGComposition` | [crates/engine/src/write_batch.rs:57](https://github.com/c4pt0r/kv9/blob/fe650ed814757e8172fb03eced102894e35d4a4d/crates/engine/src/write_batch.rs#L57); [crates/raft/src/state_machine/raw_group.rs:73](https://github.com/c4pt0r/kv9/blob/fe650ed814757e8172fb03eced102894e35d4a4d/crates/raft/src/state_machine/raw_group.rs#L73) |
| Physical namespace and opt-in barriers | `RGEligible`, independent `RGSelection`, `RMApplySystem` | [raw_group.rs:11](https://github.com/c4pt0r/kv9/blob/fe650ed814757e8172fb03eced102894e35d4a4d/crates/raft/src/state_machine/raw_group.rs#L11), [raw_group.rs:24](https://github.com/c4pt0r/kv9/blob/fe650ed814757e8172fb03eced102894e35d4a4d/crates/raft/src/state_machine/raw_group.rs#L24); [state_machine.rs:388](https://github.com/c4pt0r/kv9/blob/fe650ed814757e8172fb03eced102894e35d4a4d/crates/raft/src/state_machine.rs#L388); [crates/server/src/fence.rs:39](https://github.com/c4pt0r/kv9/blob/fe650ed814757e8172fb03eced102894e35d4a4d/crates/server/src/fence.rs#L39) |
| Same per-member fence verdict under deferred writes | `RGVerdictSameSystem`, `RGSequence`, `RGPlanIdentity` | [raw_group.rs:74](https://github.com/c4pt0r/kv9/blob/fe650ed814757e8172fb03eced102894e35d4a4d/crates/raft/src/state_machine/raw_group.rs#L74); [crates/server/src/fence.rs:67](https://github.com/c4pt0r/kv9/blob/fe650ed814757e8172fb03eced102894e35d4a4d/crates/server/src/fence.rs#L67) |
| Every position and epoch-read error checked before effect | `RGCanPlan`, `RGPlanIdentity`, `RGFail` | [raw_group.rs:52](https://github.com/c4pt0r/kv9/blob/fe650ed814757e8172fb03eced102894e35d4a4d/crates/raft/src/state_machine/raw_group.rs#L52), [raw_group.rs:59](https://github.com/c4pt0r/kv9/blob/fe650ed814757e8172fb03eced102894e35d4a4d/crates/raft/src/state_machine/raw_group.rs#L59), [raw_group.rs:78](https://github.com/c4pt0r/kv9/blob/fe650ed814757e8172fb03eced102894e35d4a4d/crates/raft/src/state_machine/raw_group.rs#L78) |
| Bounded prefix, every barrier, no future wait | `RGCanGrow`, `RGSelection`, `RGChooseDone`, `RGSingletonFallback` | [driver.rs:425](https://github.com/c4pt0r/kv9/blob/fe650ed814757e8172fb03eced102894e35d4a4d/crates/raft/src/driver.rs#L425), [driver.rs:455](https://github.com/c4pt0r/kv9/blob/fe650ed814757e8172fb03eced102894e35d4a4d/crates/raft/src/driver.rs#L455); [raw_group.rs:8](https://github.com/c4pt0r/kv9/blob/fe650ed814757e8172fb03eced102894e35d4a4d/crates/raft/src/state_machine/raw_group.rs#L8) |
| Full batch plus exact final position before success | `RGEffect`, `RGEngineBinding`, mutation fold | [raw_group.rs:97](https://github.com/c4pt0r/kv9/blob/fe650ed814757e8172fb03eced102894e35d4a4d/crates/raft/src/state_machine/raw_group.rs#L97); [crates/engine/src/mem.rs:221](https://github.com/c4pt0r/kv9/blob/fe650ed814757e8172fb03eced102894e35d4a4d/crates/engine/src/mem.rs#L221); [crates/engine/src/persist.rs:578](https://github.com/c4pt0r/kv9/blob/fe650ed814757e8172fb03eced102894e35d4a4d/crates/engine/src/persist.rs#L578) |
| No new group SM/receipt publication on error | `RGPublication`, `RGReceiptsExact`, `RGFenced` | [raw_group.rs:98](https://github.com/c4pt0r/kv9/blob/fe650ed814757e8172fb03eced102894e35d4a4d/crates/raft/src/state_machine/raw_group.rs#L98); [driver.rs:507](https://github.com/c4pt0r/kv9/blob/fe650ed814757e8172fb03eced102894e35d4a4d/crates/raft/src/driver.rs#L507), [driver.rs:519](https://github.com/c4pt0r/kv9/blob/fe650ed814757e8172fb03eced102894e35d4a4d/crates/raft/src/driver.rs#L519), [driver.rs:528](https://github.com/c4pt0r/kv9/blob/fe650ed814757e8172fb03eced102894e35d4a4d/crates/raft/src/driver.rs#L528) |
| Earlier group success survives later failure without early driver report | `RGContiguity`, `RGDriverAuthority`, later-failure witness | [driver.rs:437](https://github.com/c4pt0r/kv9/blob/fe650ed814757e8172fb03eced102894e35d4a4d/crates/raft/src/driver.rs#L437), [driver.rs:525](https://github.com/c4pt0r/kv9/blob/fe650ed814757e8172fb03eced102894e35d4a4d/crates/raft/src/driver.rs#L525), [driver.rs:562](https://github.com/c4pt0r/kv9/blob/fe650ed814757e8172fb03eced102894e35d4a4d/crates/raft/src/driver.rs#L562), [driver.rs:615](https://github.com/c4pt0r/kv9/blob/fe650ed814757e8172fb03eced102894e35d4a4d/crates/raft/src/driver.rs#L615) |
| Terminal failure does not resume the modeled call | `RGTerminalAlways` | [driver.rs](https://github.com/c4pt0r/kv9/blob/fe650ed814757e8172fb03eced102894e35d4a4d/crates/raft/src/driver.rs) error returns through `poison` |
| Finite service reaches done, fenced, or delegated under stated fairness | `RGRankDecreases`, `RGServiceEnabled`, `RGBoundedProgress` | fixed-vector `while`/bounded peek loop, synchronous call returns, and final report |

Selection stops at a metadata/manifest/configuration/no-op/malformed barrier.
For a malformed later item, the peek does not consume it: the valid preceding
group can complete and publish its receipts, and the next iteration fails at
decode. For real configuration entries, the external membership contract
remains responsible for their effect. The model's barrier name does not bless
the legacy application-level `Command::ConfChange` tag; the driver explicitly
rejects that unwired tag.

## Proof and fault gate

The gate uses pinned TLA tools and fresh strict TLAPS executions. It audits the
complete imported module graph, pins the standard libraries, checks every owned
module assumption and theorem declaration, rejects proof holes and added
axioms, and requires exact nonzero obligation counts. The mutation algebra is
freshly proved alongside every group-proof execution; it is not imported as
an unchecked owned theorem library. TLC supplies finite witnesses and
counterexamples, and is never substituted for the parameterized proof.

Positive finite configurations exercise ordered put/delete/put, stale and
all-stale groups, epoch read errors, invalid intermediate positions, default
opt-out, legal oversized singleton fallback, every barrier kind, hidden System
writes, and both count and byte bounds. Different configurations intentionally
exercise different branches; the gate requires the named reachable actions in
each configuration and the union to cover every service action. Two independent
TLC fingerprints must produce the same complete state statistics. Separate
fairness configurations check finite progress and both safety action properties.

Protocol controls change one model source at a time and require a passing
baseline, the named finite violation and named deductive failure, then a fresh
passing restoration. They cover reversed mutation order, stale acceptance,
epoch-read error suppression, hidden System writes, false default opt-in, all
five barrier crossings, count/byte bypass, singleton rejection, intermediate
position bypass, a synthesized tail pair, early SM publication, wrong member
receipt positions or rejected region IDs, ignored engine errors, a lost fence,
early whole-Ready reporting, skipped later malformed work, omitted empty-batch
position, waiting for an unavailable member, and absent service fairness.

The implementation controls at test-only commit `53e2589` provide separate
source evidence for six actual Rust faults: ignoring a group write error,
publishing the SM watermark before writing, allowing a hidden System mutation,
accepting a stale verdict, reversing mutation order, and defaulting arbitrary
adjudicators into grouping. Their diagnostic tests include the real driver
catalog-epoch barrier. This proof task does not rerun or relabel those Rust
controls as fresh results, and does not claim them as physical crash or
Chaos Mesh evidence.

Reproduction is local and reserves logical CPUs 0–5 for benchmark work:

```sh
taskset -c 6-31 python3 scripts/check-raw-group-protocol.py \
  --jar /tmp/kv9-p0-tools/tla2tools-v1.7.4.jar \
  --tlapm /tmp/kv9-p0-tools/tlapm-1.6.0-pre-20260731/bin/tlapm \
  --output /tmp/kv9-raw-group-protocol-new --jobs 4
```

Use a new output directory for every attempt. `--model-only` explicitly emits
finite preflight evidence without proof acceptance. The host is shared; CPU
affinity is not exclusive physical-core isolation. No hosted CI is invoked.

## Retained failed attempts

The first complete protocol gate is retained at
`/tmp/kv9-raw-group-protocol-first`. Its 114 finite checks completed, and its
initial strict proof baseline passed. All first eight parallel control
baselines then failed with TLAPS exit 10 before any protocol mutation was
checked. Those are failed baseline validations, not successful negative
controls. Remaining queued work was stopped after recording the owned process
tree; incomplete directories remain explicitly aborted.

A separate diagnostic on the unchanged original `RGFailStep` type obligation
passed in isolation, while three of eight concurrent fresh copies reported
`internal timeout`. This reproduces sensitivity to the pinned five-second
SMT backend deadline. None of those eight complete baseline failures reached
the 900-second process timeout. The diagnostic does not establish an exclusive
host-level cause for every failed obligation. Its partial line checks are
development evidence and do not replace a complete parameterized proof.

The revised proof decomposes failure, plan-view, terminal, and rank reasoning
into smaller contexts. Model transitions, invariants, assumptions, and backend
limits are unchanged. The owned runner now bounds active control groups and
stops scheduling additional groups after a failed verdict. Original sources,
logs, diagnostics, and development attempts are retained separately from any
later accepted gate.

The second gate, `/tmp/kv9-raw-group-protocol-second`, also completed all
114 finite cases and its strict baseline, then failed all first eight parallel
control baselines. The previously revised obligations passed; the failures
were confined to oversized `RGEngineSuccessStep` contexts. The fail-fast runner
exited cleanly without starting protocol mutants or another batch. These
baseline failures remain failures. The next revision removes unrelated
quantified context from that action and 56 predicates whose variables are
explicitly unchanged by their respective actions.

For three mainline benchmarks during the third gate, only the Python launcher
was paused. Active TLAPS modules and backend timers continued normally to
clean completion before each timing bracket began. The launcher remained
quiescent for each bracket and then resumed; the pause records preserve the
process identities, completed module log hashes, and pause/quiescence/resume
times. The same gate and fresh proof inventory continued afterward. These
launcher pauses do not alter solver deadlines. Reported parent-observed module
wall times can include the launcher wait.

The third gate, `/tmp/kv9-raw-group-protocol-third`, completed all 114 finite
cases and its initial strict baseline. Two later **unmutated restorations**
then each failed one of 1,813 group obligations in `RGPublishStep`: phase order
and publication. Their source bytes match the previously passing baselines
and other passing restorations. These are positive proof failures; they are
not intended negative controls. The already active later control groups were
allowed to finish, and the entire attempt remains failed.

The next development revision removes unrelated context from publication and
from unchanged or phase-only effect, tail, report, and planning predicates.
It explicitly carries the existing natural-count and failure-fence facts into
the smaller obligations. These are consequences of the same invariant, not
new model assumptions. Development failures from this decomposition remain
in the evidence set. The protocol transitions, finite configurations, named
theorem statements, semantic audit, and solver deadlines are unchanged.

After the decomposition, a fresh development module proved all 1,522 group
obligations. Sixteen concurrent partial-line diagnostics of the previously
failing publication obligations also passed with nonzero proof counts. Those
partial checks are diagnostic evidence only. The subsequent fresh complete
gate recorded below proved both owned modules and every required control.

## Accepted local evidence

The accepted gate is `/tmp/kv9-raw-group-protocol-fourth` (terminal exit 0).
Its summary SHA-256 is
`d01ba8a8f46eaee1e904c688727c0b18124a08758a15739cdcda0ab58b42c591`.

| Check | Accepted result |
| --- | --- |
| Mutation algebra | 14 declarations; 183 proved obligations |
| Raw group safety and conditional progress | 55 declarations; 1,522 proved obligations |
| Finite TLC cases | 114 complete verdicts, including witnesses and protocol triples |
| Protocol controls | 25 fresh baseline/mutant/restored triples |
| Semantic controls | 4 baseline/mutant/restored triples; every omitted proof or extra axiom rejected before a proof backend runs |
| Strict TLAPS executions | 168 distinct fresh commands: 143 complete positive executions and 25 intended deductive failures |
| Complete proof/audit cases | 88, with no unexpected failure |
| Output validation controls | 8 TLC and 6 TLAPS malformed-output controls, independently replayed |
| Independent audit | All 202 case records, copied source hashes, pinned tool/library bytes, named failure locations, and restored identities checked |

The independent audit also compares seven runtime files and the source contract
document byte-for-byte with `fe650ed`. All 21 model/configuration files are
unchanged from the first gate, and all 55 group theorem declaration statements
are unchanged from the third gate; only proof bodies and their generated
obligation count changed. The final commit adds proof, harness, and
documentation files without production edits.

The accepted fourth gate contains two separately recorded launcher pauses for
mainline benchmark brackets: 361.956 and 359.332 seconds. In each pause, every
active timed prover completed normally before timing clearance; the stopped
launcher had only exited child processes, and then resumed the same gate.
The records contain process identities, completed-module log hashes, and exact
pause/quiescence/resume timestamps. Solver deadlines were unchanged. These
pauses are distinct from the three pauses in the failed third attempt.

The standalone independent auditor's first invocation assumed every standard
module was a filesystem file. It stopped before case validation because
`Integers`, `Naturals`, and `Sequences` are embedded in the pinned JAR. The
corrected auditor hashes those exact JAR entries against the same inventory.
Both invocations are retained; the corrected audit passed and its fresh
semantic replays retain their copied sources and rejection records.

The evidence archive inventories 48 retained attempt directories, including all
three failed complete gates, development failures, partial diagnostics, source
snapshots, successful final runs, and independent audits. Every archived entry
was read back and compared with its original bytes:

- Archive: `target/correctness-evidence/2026-09-10-fe650ed-raw-group-proof.tar.gz`
- Size: 18,112,094 bytes; 8,466 entries.
- SHA-256: `3fb70a32180beb7ee12180c16db7a1c401a802bc0f9059150603c7c03834f8bc`
- Entry-manifest SHA-256: `0f7d2f3fad60e97ee64f5a63fad5bd6b370c39a6a207e566a87e6638b62feb74`
- Independent audit SHA-256: `13beb7a9ce30ff66a1c7eb5d183ac8c70aec62c7b08790b5255c4b127b957968`

This archive contains local formal evidence. It does not replace physical crash,
Chaos Mesh, cross-host, throughput, or later Ready/queue/read-path acceptance.
