# Backup model

Finite-trace projection of one backup of a keyspace as the cluster drives it
(docs/BACKUP-RESTORE.md §3–§7): the ranges and how many are anchored, the published owner
pairs, the completion, abort and retirement rows, whether the pins were released, and a
counter of splits, merges or plans admitted on a range the backup still held. The capture
itself has its own models (`proofs/lean/source-capture`, `proofs/lean/migration-authority`).

| Model element | Implementation |
| --- | --- |
| `bind` | `Step::Bind` in `runtime/backup.rs` → `AdminApi::bind_migration_image` with the backup subject (`MigrationOwners::new` over `CaptureSubject::Backup`, owner kind `Backup`) |
| `anchor` | `Step::Capture` → `BackupAnchor::receipt` → the shared evidence path → `backup::plan_backup_anchor` (refuses an aborted, complete or retired backup, and a second anchor for a range) |
| `complete` | `Step::Bundle` → `write_backup_bundle` → `backup::plan_backup_completion` (needs stage `anchored`) |
| `abort` | `range_abort::plan_range_abort` with `AbortTarget::Backup` (refuses a complete backup) |
| `retire` | `backup::plan_backup_retirement` (needs stage `complete`) |
| `split` | `live_range_operation` returning `backup` / `backup::live_backup_on_region` consulted by the split, merge and migration planners |

Ten theorems: safety initialization/preservation/reachability; completion needs every anchor
(B4/B5); an anchor needs a live backup; abort refuses after completion; release only by
retirement of a complete backup (B1); a held range takes no split (B7/§11); anchors are
bounded; retirement needs completion. Seven semantic controls and two policy controls.

Source-mapped abstract reasoning with explicit premises: not verified Rust extraction;
silent about the image contents (the capture models hold those) and about liveness.
