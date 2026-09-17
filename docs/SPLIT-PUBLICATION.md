# The atomic split publication (D04, part 5)

Updated 2026-09-17. Parent [#9](https://github.com/c4pt0r/kv9/issues/9),
D04 [#25](https://github.com/c4pt0r/kv9/issues/25). The manual split
completes: intent → fence → verified population → **one atomic catalog
transaction** → the children serve.

## What this increment adds

1. **The publication writer** (`publish_split`, RPC `PublishSplit`, CLI
   `publish-split`). The publishing metadata voter — which hosts the
   parent and both children on the same replica set — first re-verifies
   EVERYTHING locally from durable state: the committed intent, the
   sealed parent fence, and both children digest-equal to their sealed
   halves. Then ONE catalog transaction seals the parent binding (the
   `may_follow` version+1 successor) and inserts both child bindings
   with their region rows. The partition read model validates the result
   atomically: no reader can ever observe a partial shape. Republication
   confirms idempotently; the intent's readback accepts its own
   published shape as legal history.
2. **Serving follows the committed directory.** Three real serving-path
   defects — found by three retained failed e2e launches — are fixed:
   the split intent's readback refused after its own publication; the
   raw directory resolved keyspaces assuming exactly one range
   (`get_for` is now key-aware: the unique unsealed covering range,
   with keyless leader hints via any unsealed); and directory inserts
   never replaced a stale binding (a sealed parent's entry now refreshes,
   while per-request authorization keeps re-reading the engine row).
3. **The payoff, end to end at real stores:** public routing serves the
   exact pre-split data from the correct children, new writes land on
   the correct child, the sealed parent group keeps refusing with typed
   errors, and everything — directory, children, fence — survives a
   full-cluster restart.

## What it deliberately does not do

The sealed parent group keeps running fenced; its retirement/reclamation
is future work, as are automatic split triggers, cross-range routed
scans, stranded-learner recovery and physical reclamation. The
[3/6/9-host benchmark contract](HORIZONTAL-SCALING-PLAN.md) remains the
only acceptance path for effective horizontal scaling.

## Evidence

- Serial qualifying workspace run plus strict Clippy/formatting; the
  publication catalog test (atomic visibility, idempotent confirmation,
  routing flip inside the same readback that refuses partial shapes).
- Accepted five-process e2e (`scripts/split-publication-e2e.py`,
  `e2e-fourth`; THREE failed launches retained, each exposing a real
  defect fixed above): the full qualified chain, the atomic publication
  with an idempotent retry, children serving the exact pre-split data
  through public routing, correctly routed new writes, and a
  full-cluster restart with everything durable.
- Ten-theorem Lean model
  ([proofs/lean/split-publication](../proofs/lean/split-publication/README.md))
  with six semantic mutation controls and two proof-policy controls;
  twenty sibling Lean models and the retention TLA/TLAPS model
  re-accepted.

Validation packet: [docs/split-publication-v1](split-publication-v1/README.md).
