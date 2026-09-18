# Cascade splits: the sealed-unpublished hang, diagnosed and fixed (D04)

Updated 2026-09-17. Parent [#9](https://github.com/c4pt0r/kv9/issues/9),
D04 [#25](https://github.com/c4pt0r/kv9/issues/25). Follows
[AUTO-SPLIT.md](AUTO-SPLIT.md), which shipped single automatic splits
and retained the cascade hang as an undiagnosed open edge.

## The hang

With `KV9_AUTO_SPLIT_BYTES` small enough that the children of an
automatic split re-trigger their own splits, the auto-split increment's
cascade experiment (retained as `e2e-third`) froze permanently: a
second-level parent stayed sealed-but-unpublished, so every write and
read routed to its key slice refused forever, with nothing in the
control status. A fresh reproduction under the strengthened fixture hung
identically at write 114 of 800.

## Three defects, one hang

1. **Retroactive intent invalidation (the root cause).** The committed
   split-intent readback (`committed_splits`) accepted a SEALED parent
   binding only as the intent's own published shape — requiring both
   children bound *unsealed*. In a cascade, a grandchild's publication
   seals a child binding; the grandparent's intent row then failed
   readback, and because readback is all-or-nothing (genuine divergence
   must refuse), that one historical row froze seal, population,
   publication AND split-retirement on every node, every turn. The fix:
   a bound range never changes its boundaries, so the children's exact
   `start`/`end` remain permanent evidence of the publication — sealed
   or not. Regression test:
   `a_cascade_publication_never_invalidates_its_grandparent_intent`
   (fails on the old code with the exact historical error).
2. **Retirement starvation.** `reconcile_split_retirement` (and the
   removal-driven `reconcile_retirement`) stopped after the FIRST
   matching decision per turn. Retirement confirms idempotently, so the
   earliest published intent's no-op confirmation starved every later
   cascade parent forever. Both now process every matching decision per
   turn.
3. **The error blackout.** The reconcile chain assigned the control
   observation from its final step's result, erasing every inner step's
   recorded error on each clean turn — the auto-split increment's
   "publish errors surfaced" fix was dead on arrival, which is why both
   hangs showed empty control status. Now the observation clears only
   when the whole turn ran clean, and catalog read failures in the
   auto-split driver are recorded rather than silently returning.

A fourth, documented-not-changed behavior: `KV9_AUTO_SPLIT_BYTES=0`
disables the WHOLE pipeline driver, including seal/populate/publish of
already-committed intents — switching the trigger off freezes in-flight
splits until it is switched back on.

## Diagnosis method (evidence retained)

The preserved `e2e-third` node state was copied and REVIVED under
newer binaries — never mutating the original evidence: first revive
showed zero errors (exposing defect 3 and the `=0` freeze), an
instrumented revive printed 6742× the exact readback refusal (defect 1),
a post-fix revive published both frozen second-level splits but left the
parents running (defect 2), and the final revive recovered the
historical hang state end to end — parents 100/101/102 retired, the four
leaves serving. A from-scratch strengthened run then completed: 860
sustained writes all landed through two cascade levels, every written
key read back through public routing, and the cascade world survived a
full-cluster restart.

## What is qualified now

`scripts/cascade-split-e2e.py` gates: sustained writes at a
cascade-provoking threshold all land with ZERO manual verbs (outlasting
the documented bounded seal→publish pauses), children of an automatic
split re-trigger and publish their own splits, every written key reads
back afterward, and a full-cluster restart recovers the cascade world.
On a hang the fixture collects each node's surfaced control errors —
the refusal evidence the original blackout denied.

Machine-checked model
([proofs/lean/cascade-split](../proofs/lean/cascade-split/README.md)):
twelve theorems — a published intent's readback validity is permanent, a
child's seal never invalidates its grandparent, the second pipeline
completes after a sibling's publication, no published parent is starved,
no recorded refusal is erased — with five semantic mutation controls and
two proof-policy controls.

## Not claimed

Load-based triggers, hysteresis and time-based cooldowns; bounded
cascade depth or fan-out; physical reclamation of retired parent
storage; stranded-learner recovery; no chaos campaign in this increment;
no QPS or scaling claims (the 3/6/9-host benchmark contract in
[HORIZONTAL-SCALING-PLAN.md](HORIZONTAL-SCALING-PLAN.md) remains the
only acceptance path); no hosted CI.

Validation packet: [docs/cascade-split-v1](cascade-split-v1/README.md).
