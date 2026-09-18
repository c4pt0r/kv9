# Sealed-parent retirement after a published split

Updated 2026-09-17. Parent [#9](https://github.com/c4pt0r/kv9/issues/9),
D04 [#25](https://github.com/c4pt0r/kv9/issues/25). Closes the manual
split's local loose end: the fenced parent group no longer runs at all.

## What this increment adds

1. **The published directory is the authority.** Each reconcile turn, a
   committed split intent whose parent binding is SEALED in the committed
   directory (the atomic publication's shape — children covering, partial
   shapes unreadable) retires the local parent replica on every hosting
   node: `reconcile_split_retirement` →
   `RegionManager::retire_published_parent`, reusing the factored
   `retire_locally` machinery — driver stopped, raw-directory and
   group-handle entries removed, the checksummed `Phase::Retired` record
   published atomically, the group lock held, storage untouched.
2. **Race-free idempotent publication.** A publication retry after the
   parent already retired confirms from the committed directory alone —
   the local-verification path is only for the FIRST publication, when
   the parent must still be running. (Found by the e2e: the confirm raced
   the automatic retirement.)
3. **Restarts open nothing**, exactly as in storage retirement: discovery
   loads the Retired record and never reopens the raft log or engine; the
   children keep serving the split keyspace throughout.

## What it deliberately does not do

No physical deletion or reclamation, no automatic split triggers, no
cross-range scans. The [3/6/9-host benchmark
contract](HORIZONTAL-SCALING-PLAN.md) remains the only acceptance path
for effective horizontal scaling.

## Evidence

- Serial qualifying workspace run plus strict Clippy/formatting.
- Accepted five-process e2e (`scripts/parent-retirement-e2e.py`;
  `e2e-first` failed on the real publication/retirement race, retained
  and fixed): the full manual split through the atomic publication, then
  every voter retiring its sealed parent automatically with durable
  storage verified intact on every node, the retired state surviving a
  further restart with nothing reopened, and the children serving the
  split keyspace throughout.
- Ten-theorem Lean model
  ([proofs/lean/parent-retirement](../proofs/lean/parent-retirement/README.md))
  with six semantic mutation controls and two proof-policy controls;
  twenty-one sibling Lean models and the retention TLA/TLAPS model
  re-accepted.

Validation packet: [docs/parent-retirement-v1](parent-retirement-v1/README.md).
