# Metadata task history without a lifetime row ceiling

Updated 2026-10-03, working tree based on
`4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`.

Creation, activation, range binding, split, migration, evidence, abort, removal,
truncation and compaction shared a 255-row **total history** budget. Routine
successful maintenance could exhaust it even with few live groups. Removing
local worker/FD leaks alone therefore could not sustain group churn.

## Change and compatibility

`MetaTxn::scan_after` returns a bounded page exclusively after a complete,
validated primary key, using the transaction's original snapshot and overlay.
The continuation starts at the encoded key with one zero byte appended. It
does not skip corrupt keys extending that exact key. `task_rows` holds one
decoded page of 128 rows and walks to exhaustion; an exactly full final page
is followed by an empty read. A read/decode failure is returned once, after
which the iterator is exhausted. It is never converted into successful EOF.

All data-group task readers/planners use that traversal. Ownership probes use
the engine's streaming iterator on their supplied historical `ReadView`.
Cross-referenced creation records still come from the same transaction view.
Compaction readback point-reads its exact creation task. Creation retries and
owner probes finish validation before returning a match, so a later corrupted
page cannot be hidden by an earlier matching receipt. Duplicate creation
operations across pages refuse both readback and planning.

The change retains every task record and all ID allocator state. It performs
no catalog deletion, task-ID reuse, format migration, or change to replicated
intent bytes. Old operations continue to resolve to their original identities;
split/migration references and retention evidence remain available. The
255-slot **concurrent local group** limit is separate and remains in force.

An old binary retaining the 255-row checks cannot handle a catalog exceeding
that count. Upgrade all metadata readers/planners before growing past the old
limit; mixed-version operation and downgrade after that point are unsupported.
Unchanged encoding alone does not establish upgrade compatibility.

## Validation

Commands, retained logs, source hashes and scope flags are in
[the validation manifest](task-history-validation-20261003/manifest.json).

- Before the fix, two new regressions failed at activation/compaction task
  capacity. Both now pass: 270 creations plus activations (540 retained rows)
  preserve every retry, and 300 compaction decisions retain the first receipt
  while recovering the highest floor.
- All 130 metadata tests pass, including the doctest; one MinIO-dependent test
  remains ignored. Pagination covers fixed snapshots across concurrent commits,
  overlay insert/update/delete, empty and full final pages, invalid/partial
  cursors, corrupt key extensions, late corruption and cross-page duplicates.
  Existing split, migration settlement, evidence, abort, removal and truncation
  checks now exercise authority after more than 255 retained task rows.
- A WAL reopen test retains 130 creations and 130 activations, rechecks every
  old retry, then verifies that the next region allocation advances.
- All 14 RegionManager tests pass. The churn test retains all 261 creations and
  activations (522 rows); **the former test-only catalog pruning is removed**.
  One group stays live while 260 others are created and retired; half reclaim
  payload and half keep it. Per-store Linux group FD counts return to baseline,
  manager recovery rejects resurrection, and historical retries remain read-only.
  This is local component recovery, not multi-process quorum or power-loss evidence.
- Strict Clippy for meta, server and raft, all targets, and workspace formatting
  pass. No throughput, RSS-soak or cross-host claim follows from these tests.

## Traversal model and refinement

[Pagination.tla](../proofs/tla/task-history/Pagination.tla) separates page loading,
row emission and unrelated live-store commits. Its immutable snapshot invariant
requires `emitted ++ buffer` to be exactly the scanned prefix; completion yields
the full snapshot, including when its length is a multiple of the page budget.
Conditional completion assumes finite reads and recurring load/emission steps.
Configurations and the runner retain four semantic counterexample controls.

[Pagination.lean](../proofs/lean/task-history/Pagination.lean) proves ten statements:
initial partition, step and arbitrary finite-run partition preservation, page
bound, strict progress on nonempty history, exact remaining suffix, finite
exhaustion, complete output, lookup preservation and a full final page. These
are generic finite-list results, not an enumeration up to 255 rows. The exact
Lean 4.33.1 toolchain and axiom audit pass; overlapping pages, skipped rows,
frozen cursors and a zero page budget each fail the corresponding proof.
Two proof-policy controls also reject.

| Model fact | Implementation mapping / remaining assumption |
| --- | --- |
| Immutable ordered snapshot | `MetaStore::begin`, engine `ReadView`, and a fixed transaction overlay; pagination does not begin a new transaction per page |
| Remaining suffix | `scan_after` validates the whole PK and seeks strictly after it; relies on canonical key encoding and ordered, exhaustive engine iteration |
| Page bound and partition | `merged_scan(...).take(limit)` and decoded pages; `task_rows` consumes each row before loading the next page |
| Finite exhaustion | Positive page size, finite snapshot and terminating engine calls; CPU scheduling fairness and storage completion are assumptions |
| Same lookup authority | Same full row sequence, existing protocol decoders, unchanged task bytes and no deletion; Rust tests cover errors, bindings and retries |

The list proof abstracts each page as a complete emission; the TLA model adds
the buffered row steps. Neither mechanically verifies Rust, iterator errors,
Raft ordering, allocator behavior, storage durability or whole-protocol
composition. Those remain explicit assumptions or separate test/proof gates.
SANY parsing passes. TLC exploration is **not run** here because the previously
observed sandbox socket restriction blocks its fingerprint-store RMI listener;
the combined proof gate remains unaccepted. Actual Chaos Mesh also remains open.

```sh
python3 scripts/prove-task-history.py \
  --lean /path/to/lean-4.33.1 --jar /path/to/pinned/tla2tools.jar \
  --output /fresh/validation-directory
```

The recorded `--lean-only` run sets `lean_accepted: true` and `tla_parsed: true`,
while keeping `accepted` and `tla_accepted` false. Existing protocol source-pin
reports are historical; this change does not repin or reaccept them.

## Remaining production work

Pagination removes an artificial lifetime admission ceiling. It does not bound
total history bytes, engine-resident catalog state, returned protocol vectors,
local tombstone memory or discovery time. Some decoders still perform repeated
scans and quadratic duplicate/cross-reference checks. The existing merged-range
helper also copies the in-range transaction overlay; the page budget applies
to decoded row payloads, not a universal memory cap or per-turn work budget.

Next add indexed authority lookups and bounded maintenance scheduling, and
define a separately proven retention/compaction protocol for dedup receipts and
ownership history. Concurrent backpressure reservation remains another immediate
production gap. Raw KV storage steady state, TTL, recoverable split/merge and
their combined failure acceptance remain required by the
[production scope](RAW-KV-PRODUCTION-SCOPE.md).
