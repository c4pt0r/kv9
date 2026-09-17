# Storage-retirement model

Finite-trace projection of one removed replica's local fate. The removal
decision's authority chain (evidence, quorum floor, never the destination)
is a premise from the replica-removal model.

| Model element | Implementation |
| --- | --- |
| `commit` | committed kind-106 removal decision naming this exact store |
| `run` | the excluded replica still running locally |
| `retire` (decision names this node + incarnation) | `RegionManager::retire_removed` via `reconcile_retirement`: driver stopped, raw-directory entry removed, durable `Phase::Retired` record (`crates/server/src/region_manager.rs`, `crates/server/src/runtime.rs`) |
| `restartRetired` | discovery loads the Retired record, holds the group lock, opens nothing |
| no deletion or serving step | storage stays durable; physical reclamation is a later increment |

Eleven theorems: safety initialization/preservation/reachability, retirement
requires the committed removal, a retired replica never runs, storage is
never deleted, no serving capability, retirement is permanent, restart
preserves retirement, a removal alone retires nothing (reconcile is the
actor), and the committed chain reaches retirement. The runner checks six
semantic mutation controls and two proof-policy controls.

Source-mapped abstract reasoning with explicit premises: not verified Rust
extraction; silent about physical reclamation, stranded-learner recovery,
split and scaling.
