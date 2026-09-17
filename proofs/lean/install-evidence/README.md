# Install-evidence settlement model

Finite-trace projection of one migration operation's settlement: committed
migration → published image pins → durable adoption → committed
destination-install evidence → source quiesce → source release. Catalog row
immutability, ledger atomicity, adoption exactness and attach/capture
semantics are premises from their own models.

| Model element | Implementation |
| --- | --- |
| `commit` | committed migration intent (kind-103 row) |
| `pin` (migration required; publishes both owners) | `MigrationOwners::bind` Acquire/Publish/Share (`crates/server/src/migration_retention.rs`) |
| `adopt` (migration + pins) | reconciled runtime adoption (`RegionManager::adopt_installed`) |
| `record` (adopted, source pin Published) | `plan_install_evidence` kind-104 row + the published-pin subject cross-check (`crates/meta/src/data_groups/evidence.rs`, `record_install_evidence`) |
| `quiesce` (committed evidence, published pair) | the lifted ledger fence consulting `evidence_matches_owner_pair` from the SAME applied view (`crates/meta/src/retention.rs`) |
| `release` (quiesced source only) | `LedgerRequest::Release` phase transition |
| no destination-release or divergent-subject step | the live replica's pin never drops; a wrong subject refuses at commit |

Thirteen theorems: safety initialization/preservation/reachability, quiesce
and release require the committed evidence, evidence requires adoption under
the pins, adoption requires the committed migration and pins, a pinned
operation names a committed migration, no divergent subject ever commits,
the destination pin never drops, the source phase never regresses, evidence
is permanent, an adopted replica alone releases nothing, and the full
committed chain reaches release. The runner checks nine semantic mutation
controls and two proof-policy controls.

Source-mapped abstract reasoning with explicit premises: not verified Rust
extraction; silent about source log truncation, promotion, removal, split
and scaling.
