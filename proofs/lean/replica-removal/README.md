# Replica-removal model

Finite-trace projection of one migration source replica's retirement from
the voter set. Evidence settlement, promotion and raft joint-consensus
semantics are premises from their own models.

| Model element | Implementation |
| --- | --- |
| `commit` | committed migration intent (kind-103 row) |
| `record` | committed kind-104 install evidence via the adoption chain |
| `promote` (evidence; quorum 3 → 4) | `promote_migration_voter` |
| `decide` (evidence; names one exact creation replica, never the destination) | `plan_removal` kind-105/106 row (`crates/meta/src/data_groups/removal.rs`) |
| `remove` (decision + destination voting + four voters; quorum 4 → 3) | `remove_source_replica`: leader-only `driver.remove_voter` (`ConfChangeType::RemoveNode`), idempotent confirm, no self-removal (`crates/server/src/runtime.rs`) |
| no destination-removal or serving step | the planner refuses the destination as target; reads still refuse at non-leaders |

Twelve theorems: safety initialization/preservation/reachability, removal
requires the committed decision and the promoted voter, a decision requires
the committed evidence, the quorum never drops below three, the destination
is never removed, no serving capability, removal is permanent, a removed
chain carries the full authority, a decision alone removes nothing, and the
committed chain reaches removal at exactly three voters. The runner checks
seven semantic mutation controls and two proof-policy controls.

Source-mapped abstract reasoning with explicit premises: not verified Rust
extraction; silent about local retirement/reclamation of the removed
replica's storage, stranded-learner recovery, split and scaling.
