# Restore model

Finite-trace projection of one restore of a complete backup (docs/BACKUP-RESTORE.md §5–§7):
the targets and how many are evidenced, whether an evidence commit ever accepted a
mismatching digest, publication, abort, writes to the source (none) and whether the backup
retired under a live restore (never). The copy itself reuses the split population code,
whose model is `proofs/lean/range-split-population`.

| Model element | Implementation |
| --- | --- |
| `evidence` | `Step::Populate` in `runtime/restore.rs` → `RestoreEvidence::receipt` → the shared evidence path → `restore::plan_restore_evidence` (refuses a non-live restore and rows/digest that differ from the anchor) |
| `publish` | `Step::Publish` → `commit_restore_publication` → `restore::plan_restore_publication` (needs stage `populated`; one transaction inserts keyspace, bindings and routing rows) |
| `abort` | `range_abort::plan_range_abort` with `AbortTarget::Restore` (refuses a published restore); targets retire through `aborted_children` |
| `retireBackup` | `backup::plan_backup_retirement` refusing while `RestoreRows::live_of_backup` is non-empty |

Ten theorems: safety initialization/preservation/reachability; publication needs every
evidence (B4); evidence needs a match and a live restore (B5); abort refuses after
publication; the backup waits for live restores (B1); the source is never written (B3); no
mismatch is ever committed; evidence is bounded. Seven semantic controls and two policy
controls.

Source-mapped abstract reasoning with explicit premises: not verified Rust extraction;
silent about the image contents (the capture and population models hold those) and about
liveness.
