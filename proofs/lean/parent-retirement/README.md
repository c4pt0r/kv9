# Parent-retirement model

Finite-trace projection of the sealed parent's fate after its split
publishes. Publication atomicity and the storage-retirement machinery are
premises from their own models.

| Model element | Implementation |
| --- | --- |
| `publish` | the committed atomic one-to-two directory (split-publication model) |
| `retire` (published only) | `reconcile_split_retirement` → `RegionManager::retire_published_parent`: the sealed committed binding with covering children is the one authority; the factored `retire_locally` machinery stops the driver, cleans the directory/handles and publishes the durable Retired record (`crates/server/src/runtime.rs`, `crates/server/src/region_manager.rs`) |
| `restartRetired` | discovery loads the Retired record, holds the lock, opens nothing |
| no deletion/disturbance step | storage stays durable; the children's serving is untouched |

Eleven theorems: safety initialization/preservation/reachability, retirement
requires the published split, a retired parent never runs, storage is
never deleted, the children are never disturbed, retirement is permanent,
restart preserves retirement, a publication alone retires nothing (the
reconcile is the actor), and the published chain reaches retirement. The
runner checks six semantic mutation controls and two proof-policy
controls.

Source-mapped abstract reasoning with explicit premises: not verified
Rust extraction; silent about physical reclamation, automatic triggers
and scaling.
