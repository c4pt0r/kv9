# Committed split intents (D04, part 1)

Updated 2026-09-17. Parent [#9](https://github.com/c4pt0r/kv9/issues/9),
D04 [#25](https://github.com/c4pt0r/kv9/issues/25). D04 proceeds in
committed-authority-first increments exactly like D03 did; this first
increment is the AUTHORITY seam only.

## What this increment adds

**Committed split intents** (TASKS kind 107, `KV9SPL01`,
`crates/meta/src/data_groups/split.rs`; RPC `RecordSplitIntent`, CLI
`record-split-intent`). One immutable intent per parent region binding:

- the parent's exact current kind-102 range binding row (which must be
  committed and unsealed),
- one exact split key, 1..1024 bytes, strictly inside the parent range,
- two DISTINCT committed child creations that are activated, bound to no
  keyspace, and carry exactly the parent's replica set — a split moves no
  replica; movement stays D03's job.

An identical resubmission is a confirmation; any divergence refuses; one
operation never names a second split key; one live intent per parent.
Planning and readback share the full committed cross-validation.

## What it deliberately does not do

Nothing seals the parent, populates a child, republishes the range
directory, reroutes a single key, or serves differently: the parent range
keeps serving unchanged (the e2e proves it under live writes). The rigid
single-range routing model is untouched; generalizing the directory to a
child partition (no overlap, no gap, atomic one-to-two publication) is
the next D04 increment, followed by parent seal and child population.

## Evidence

- Serial qualifying workspace run plus strict Clippy/formatting; catalog
  unit tests for planner refusals (unbound parent, identical/missing/
  unactivated/bound children), idempotent confirmation, one-intent-per-
  parent, and the readback defect matrix over the cross-bound fields.
- Accepted five-process e2e (`scripts/split-intent-e2e.py`, first
  attempt): a bound parent keyspace under writes, two activated unbound
  children, the refusal matrix, exact idempotent commitment, and the
  parent still serving the same data afterward.
- Ten-theorem Lean model
  ([proofs/lean/split-intent](../proofs/lean/split-intent/README.md))
  with seven semantic mutation controls and two proof-policy controls;
  sixteen sibling Lean models and the retention TLA/TLAPS model
  re-accepted.

Validation packet: [docs/split-intent-v1](split-intent-v1/README.md).
