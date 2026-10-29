# Replica-addition driver model

Finite-trace projection of one committed replica plan (docs/REPLICA-ADDITION.md): the
committed catalog rows, the facts only one party observes (learner/voter membership at
the group leader, an installed generation and an adoption receipt at the destination),
and the driver's next-step function. Raft safety, capture integrity and the evidence
planner's refusals are premises from their own models.

| Model element | Implementation |
| --- | --- |
| `commit` (plan, mode) | `AddReplica` → `plan_migration` + `plan_replica` in one catalog transaction (`crates/meta/src/data_groups/addition.rs`) |
| `abort` | committed kind-108 row (`MigrationAbort`); every verb refuses afterwards |
| `enabled` / `nextStep` | `NodeRuntime::next_step` (`crates/server/src/runtime/replica_addition.rs`): the step table of §2, in order, for the roles the process holds |
| `attach`, `planImage`, `bind`, `capture`, `promote`, `decide`, `remove` | the existing verbs `attach_migration_learner`, `plan_migration_image`, `bind_migration_image`, `capture_migration_image`, `promote_migration_voter`, `record_source_removal`, `remove_source_replica`, called in-process from the worker thread |
| `install` | `install_captured_record_with_reader` against the destination's own store guard with the record of the committed capture descriptor and the in-process routed ledger |
| `adopt` | the existing `reconcile_adoption` |
| `record` | `emit_install_evidence` + delivery over the node-authenticated `RecordInstallEvidence` RPC, committed by `record_install_evidence` at the metadata leader |
| `quorum` | the group's voter count: promotion adds one, removal subtracts one |

Nineteen theorems: safety initialization/preservation/reachability; promotion requires
evidence; evidence requires an adopted, installed, pinned capture under a plan; removal
requires the decision, the promoted voter and evidence; a decision requires evidence and
the move mode; the quorum never drops below three; a promoted destination means four
voters until the source goes; removal happens only from four voters to three; the
destination is never removed; the source is removed at most once; an abort stops every
driver step but adoption of an already installed image; no step after an abort changes
the voter set; the driver's choice is enabled, exists until the plan is terminal, and is
the table's choice for promotion, removal and install; an addition reaches four voters;
a move reaches three voters without the source. The runner checks ten semantic mutation
controls and two proof-policy controls.

Source-mapped abstract reasoning with explicit premises: not verified Rust extraction;
silent about object-storage availability, placement, and liveness under repeated faults
(each step is retried, but the model proves only that a retry never takes an earlier or
unauthorized step).
