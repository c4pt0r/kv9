# Bounded owner polling experiment

This isolated candidate tests one fixed 32-us owner-poll budget before the
existing mutex/condition-variable wait. It is not selected on main and has no
performance acceptance yet. The [new quorum capture](QUORUM-TRACE-RESULTS.md)
measures follower inbox residence at 1.752–1.817 us, with only 0.479–0.490 us
from follower driver step to response offer. Those intervals do not prove that
all inbox residence is a park/wake cost; this candidate tests that hypothesis.

The earlier worker/global-queue/direct-body/executor experiments retain their
[recorded decisions](PERFORMANCE-EXPERIMENT-INDEX.md). This candidate changes
neither those settings nor notification coalescing (`42e0117`). Busy polling
can consume more CPU or worsen shared-core contention; throughput, mean/p99,
mixed reads and CPU accounting must all remain visible before any promotion.

## Concrete correspondence

`WorkSignal::notify`, `stop` and `begin_turn` preserve their original changes to
`pending`/`stopped` and every condition-variable notification. They additionally
write an atomic hint while holding the existing signal mutex. The hint never
selects a read, work item, Ready, receipt, apply watermark or leader authority.

`wait_until` holds no signal/queue/peer mutex while polling. It stops polling
when the hint is true or the monotonic deadline reaches the earlier of the
original owner deadline and poll start plus 32 us. It then runs the original
mutex-protected predicate and atomic condition-variable park, unchanged.
A stale true hint only skips optional polling. A stale false hint only spends
the finite optional budget: published work or stop is still found under the
mutex. Publication before parking cannot disappear in a new check/park gap.

Erasing hint stores, reads and spin steps leaves the original scheduling
transitions. The model deliberately permits either stale hint value. Each
polling step stutters on every original scheduling variable; the hint cannot
consume pending work. The poll budget is bounded and strictly decreases on
abstract local-time progress. The existing tick deadline is never reset or
extended by polling. Scheduler preemption can still exceed a wall-time target;
no new OS scheduling bound is claimed.

[OwnerPoll](../proofs/tla/owner_poll/OwnerPoll.tla) refines the original
[RaftSchedule](../proofs/tla/raft_schedule/RaftSchedule.tla).
The parameterized [proof](../proofs/tlaps/owner_poll/OwnerPollProof.tla) covers
stuttering refinement, type/budget preservation, no false park authority,
terminal behavior and inherited conditional service progress. Abstract
`OPSpin` is monotonic time-budget progress, not one machine instruction;
equal clock reads can stutter. Safety does not assume clock-derived leadership.
The concrete hint is advisory even if the machine pauses or its clock behaves
poorly; the original ReadIndex/consensus checks remain mandatory.

Five new Rust controls cover both stale hint directions, publication during the
poll phase, fall-through to the authoritative predicate, actual parking and
stop. Existing publication/drain/park, owner uniqueness, tick, partition,
ReadIndex/apply-fence and recovery tests remain required. The proof checker also
injects illegal pending-bit consumption and requires a lost-wakeup counterexample
and failed stuttering proof, plus proof-hole/assumption/output rejection controls.

Fresh Safe ReadIndex, sealed read groups, successful whole-pump completion,
apply/view fences, durable write ACKs, admission/cancellation/deadlines and the
number of services/replicas are unchanged. This experiment does not enable lease
reads or DPDK. It supplies no complete database proof or new Chaos acceptance.

## Qualification plan

Run `scripts/check-owner-poll.py` against the pinned local TLC/TLAPS tools.
Retain original failed proof drafts and every checker/source outcome. Qualify
the default release workspace and standalone read-stage configuration, then a
clean production build and ordinary recovery before timing. Freeze c1 GET,
c64 GET and c64 mixed reads/writes with the original client, both run orders,
CPU/resource accounting and existing storage/CPU limits. Do not tune the budget
or rerun losing cohorts to replace an unfavorable result. Actual Chaos Mesh
and broader relevant API gates are required before default promotion.

## Completed source checkpoint

The [original qualification](owner-poll-source-v1/qualification.json) now records:

- Four finite safety explorations: 3,368 / 7,284 distinct states, each repeated
  under two fingerprint polynomials with matching counts and both poll actions
  covered. The conditional-service exploration covers 650 distinct states.
- Fourteen new parameterized TLAPS theorems, 49 obligations, plus the existing
  33-theorem / 294-obligation scheduling dependency. The semantic audit checks
  the exact assumption and proof closure. Deliberately consuming pending work
  fails both the lost-wakeup invariant and the stuttering proof; proof holes,
  added assumptions and malformed success output are also rejected.
- Five focused poll controls and 714 default workspace tests, 23 existing
  ignored. The independent `kv9-raft/read-stage-timing` configuration passes
  443 Raft/server tests, one existing ignored. Formatting and both Clippy
  configurations pass. These test populations overlap.

The first two proof drafts leave two and one obligations unproved; the third
supplies the missing variable expansion and explicit temporal induction. No
model or assumption was weakened. Every original draft is retained with the
final audited proof in the [archive inventory](owner-poll-source-v1/archive-inventory.json).

The first Rust invocation stops before Cargo because the recently published raw
archives exceed the old build-inventory limits. [Repair `2ccb478`](BUILD-CACHE-SAFETY.md#retained-evidence-in-source-inventories)
keeps ordinary source bounds and accounts separately for fully hashed archives.
The next invocation passes every default check, then rejects a command naming
the diagnostic feature under `kv9-server`. Only the three diagnostic commands
run again with its actual owner, `kv9-raft`; no default test is repeated. Both
failed invocations remain failed and preserved in the original archive.

The fixed experiment reuses 12 two-second smoke cohorts and 24 ten-second timed
cohorts: c1/c64 GET and point mixed 50:50, control/candidate/Redis, two opposite
orders. The full retained scope includes c1 mixed as well as the three primary
cells. Clean default build, ordinary recovery and actual timing remain pending.
No performance gain, default promotion, new Chaos acceptance or original
industrial checklist closure follows from this source checkpoint.
