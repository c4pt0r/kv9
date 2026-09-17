# Unified-cut source capture model

Finite-trace projection of one capture attempt for one committed migration
operation. `engineCut` abstracts the frozen durable engine position and
`configAt` the commit position of the attached configuration; ledger
atomicity, the retention model and the installer's validation are premises
from their own models, not re-proved here.

| Model element | Implementation |
| --- | --- |
| `freeze` (config ≤ cut premise) | `plan_capture` + `configuration_at_committed` (`crates/raft/src/snapshot_install/capture.rs`, `crates/raft/src/storage/configuration.rs`) |
| `pin` | committed `MigrationOwners` for the planned manifest (`bind_migration_image` at the metadata leader) |
| `upload` (requires pin) | `verify_published_locally` before `PlannedCapture::upload` (`crates/server/src/runtime.rs`, `crates/server/src/migration_retention.rs`) |
| `describe` / `retry` | capture RPC receipt; idempotent re-capture of an unchanged cut |
| no serving/voting step | capture returns a description only; installer gates unchanged |

Ten theorems: safety initialization/preservation/reachability, the attached
configuration is at-or-before the cut, upload requires committed pins, the
receipt requires a pinned upload of the exact cut, no serving/voting
capability, the frozen cut is immutable, one image per operation, retry is a
pure receipt, and a frozen cut alone uploads nothing. The runner checks nine
semantic mutation controls and two proof-policy controls, and audits axiom
dependencies.

Source-mapped abstract reasoning with explicit premises: not verified Rust
extraction, not a transfer/settlement model, and silent about network
snapshots, learner catchup and scaling.
