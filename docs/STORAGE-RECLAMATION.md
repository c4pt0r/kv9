# Physical reclamation of retired data-group storage

Updated 2026-09-17. Parent [#9](https://github.com/c4pt0r/kv9/issues/9),
D03 [#24](https://github.com/c4pt0r/kv9/issues/24) /
D04 [#25](https://github.com/c4pt0r/kv9/issues/25). Retired local groups
kept their payload forever; cascades made that real cost (one cascade
e2e retires 28 parents per node). Their payload is now physically
deleted — under committed authority, with the fence surviving.

## What this increment adds

1. **A `Reclaimed` local phase.** The durable group record — the same
   crash-safe record machinery every local phase uses — gains phase
   `Reclaimed = 4`. The record is REWRITTEN to `Reclaimed` *before* any
   file is deleted: the durable record commits the deletion, so a crash
   mid-deletion resumes idempotently at discovery, which finishes the
   deletion and never opens the group. Only the payload dies — the
   engine WAL (`data.wal`), its segment directory (`data.segments`) and
   the raft log (`raft/`); the record and lock survive as the permanent
   fence.
2. **Re-verified committed authority.** `reconcile_reclamation` (off
   unless `KV9_RECLAIM_RETIRED=1`) walks the locally RETIRED groups and
   reclaims only those whose authority re-verifies from LOCAL applied
   state: a SEALED binding for the region (the published split — its
   children's bindings are the permanent evidence) or a committed
   removal decision naming this exact store incarnation. A retired
   group with neither is left untouched — refusal in the safe
   direction. Every candidate is processed each turn (no starvation
   behind idempotent confirms — the cascade lesson).
3. **Observability.** Status reports `"reclaimed"` per group;
   reclamation errors surface in the control status and retry.

## Known limits, deliberately out of scope

Object-store reclamation is NOT here: retention-ledger image pins
remain the object-store authority, and no MinIO object is deleted by
this increment. No automatic enablement (the env var is an explicit
operator decision), no disk-usage accounting or quotas, no cascade
depth bounds, no chaos campaign, no performance claims.

## Evidence

- Serial qualifying workspace run (933 tests) plus strict
  Clippy/formatting; the new unit test drives
  active-refusal → retire → reclaim (payload gone, fence retained,
  bytes counted) → idempotent confirm → crash-resume at discovery →
  never re-prepares.
- Accepted zero-verb e2e (`scripts/reclamation-e2e.py`): sustained
  cascade writes (860, all landing and reading back), then EVERY
  retired group on all three nodes reclaims — 28 regions per node, 84
  payload directories verified deleted on disk with records retained —
  leaves keep serving, and a full-cluster restart holds every fence and
  serves every key.
- Thirteen-theorem Lean model
  ([proofs/lean/storage-reclamation](../proofs/lean/storage-reclamation/README.md))
  with five semantic mutation controls and two proof-policy controls;
  twenty-five sibling Lean models and the retention TLA/TLAPS model
  re-accepted.

Validation packet: [docs/storage-reclamation-v1](storage-reclamation-v1/README.md).
