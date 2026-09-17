# The partition range directory (D04, part 2)

Updated 2026-09-17. Parent [#9](https://github.com/c4pt0r/kv9/issues/9),
D04 [#25](https://github.com/c4pt0r/kv9/issues/25). Builds on
[split intents](SPLIT-INTENT.md): the READERS learn the post-split world
before any writer produces it.

## What this increment adds

The kind-102 range directory read model
(`crates/meta/src/data_groups/ranges.rs`) generalizes from the rigid
single-range shape to a **validated partition**:

- Bindings may carry real bounds, versions above one, and the sealed
  flag; per-row root/creation/region/keyspace/namespace binding rules and
  region uniqueness are unchanged.
- For every keyspace, the UNSEALED bindings must cover the whole key
  space exactly once: sorted by start, first start empty, last end empty,
  every boundary shared with its neighbor — no overlap, no gap, and no
  keyspace without unsealed coverage. Anything else refuses the whole
  readback, exactly as the old model refused non-canonical rows.
- Sealed bindings are routing history: validated, region-unique, and
  never routed — `route_in` picks the unsealed covering range only.

No writer produces child rows yet: the atomic one-to-two publication is
the next D04 increment, followed by parent seal execution and child
population. The `DataRange::may_follow` one-way sealing rule and the
range reconciliation path are untouched and remain the seal-execution
seam.

## Evidence

- Serial qualifying workspace run plus strict Clippy/formatting; new
  catalog tests hand-craft the post-split directory: a sealed parent with
  no children refuses (uncovered), a gapped child set refuses, the exact
  partition is accepted with routing skipping the sealed parent and the
  boundary key belonging to the high child, and an overlapping extra
  child refuses.
- Accepted five-process regression e2e (`scripts/split-intent-e2e.py`
  rerun against the generalized read model): the legacy trivial partition
  keeps serving unchanged through the whole committed split-intent flow.
- Ten-theorem Lean model
  ([proofs/lean/partition-directory](../proofs/lean/partition-directory/README.md))
  with five semantic mutation controls and two proof-policy controls;
  seventeen sibling Lean models and the retention TLA/TLAPS model
  re-accepted.

Validation packet: [docs/partition-directory-v1](partition-directory-v1/README.md).
