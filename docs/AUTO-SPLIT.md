# Automatic split triggers (D04)

Updated 2026-09-17. Parent [#9](https://github.com/c4pt0r/kv9/issues/9),
D04 [#25](https://github.com/c4pt0r/kv9/issues/25). Sustained writes now
split a range with no operator verb at all.

## What this increment adds

1. **A write-volume trigger.** With `KV9_AUTO_SPLIT_BYTES > 0` (default
   0: disabled), the metadata leader observes each bound, unsealed
   range's LOCAL replica bytes — the active WAL plus its segment
   directory, an honest write-volume metric that counts overwrites — and
   on a breach samples an approximate median user key (strictly
   interior; too-small ranges never split).
2. **One deterministic operation per region.** The trigger derives the
   split operation as `sha16("kv9-auto-split-v1" ++ root ++ region)` and
   the child creation operations from it — so ANY coordinator, at any
   point, re-derives the same identifiers and every step confirms
   idempotently. One live intent per parent region is the v1 cooldown:
   a region splits once, and its children are new regions.
3. **The proven pipeline drives itself, role-locally.** The committed
   state is the only coordination: the metadata leader creates and
   activates the children and commits the kind-107 intent; the parent
   group's leader seals; each child's leader populates (idempotently);
   the metadata leader publishes exactly when both digests verify; the
   existing reconciles then retire the parent and serve the children.
   Errors — including publish refusals while population converges — are
   surfaced in the control status, never swallowed.

## Known limits, deliberately out of scope

**Cascade splits are real but unqualified.** With a threshold small
enough for children to re-trigger, the cascade experiments produced a
second-level parent stuck sealed-but-unpublished, permanently fencing
its key slice — retained as e2e evidence, root cause not yet diagnosed.
This increment therefore qualifies SINGLE automatic splits (threshold
sized so children stay below it); cascade correctness, load-based
triggers, richer hysteresis and time-based cooldowns are future work.

## Evidence

- Serial qualifying workspace run plus strict Clippy/formatting.
- Accepted five-process e2e (`scripts/auto-split-e2e.py`, `e2e-fourth`;
  three retained failures: a WAL-segmentation metric gap, a
  parent-pinned fill helper, and the cascade experiment above): 160
  sustained writes with ZERO manual verbs → the whole committed pipeline
  self-drives → the parent retires and two children serve → every
  sampled written key survives through public routing → the split world
  survives a full-cluster restart.
- Eleven-theorem Lean model
  ([proofs/lean/auto-split](../proofs/lean/auto-split/README.md)) with
  five semantic mutation controls and two proof-policy controls;
  twenty-three sibling Lean models and the retention TLA/TLAPS model
  re-accepted.

Validation packet: [docs/auto-split-v1](auto-split-v1/README.md).
