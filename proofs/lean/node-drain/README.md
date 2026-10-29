# Node-drain model

Finite-trace projection of one drain as the metadata leader drives it (docs/DRAIN-NODE.md
§3–§5): the data groups that still hold the drained store, the unsettled move plans with
that source under the concurrency bound, the metadata group's voter count and the drained
node's own membership, whether a caught-up learner and a placeable (active, non-draining)
destination exist. The move plan itself has its own model (`proofs/lean/replica-addition`);
the leadership hand-off has `proofs/lean/leader-transfer`.

| Model element | Implementation |
| --- | --- |
| `plan` | `NodeRuntime::plan_drain_steps` → `Step::MovePlan` → `AdminApi::add_replica` (mode move, source = the drained store, destination from `runtime/placement.rs`) |
| `settle` | the replica-plan driver's promote-then-remove (`runtime/replica_addition.rs`) |
| `promote` / `removeVoter` / `removeLearner` | `Step::PromoteLearner` / `Step::RemoveVoter` (`AdminApi::remove_node`, which refuses below three) / `Step::RemoveLearner` |
| `candidateDrains` / `candidateFresh` | `placement::choose_destination` skipping stores named by a committed drain |

Twelve theorems: safety initialization/preservation/reachability; N1 a group never
shrinks; N2 the metadata group never has fewer than three voters; N3 a draining store
receives nothing; N4 bounded concurrency; removal waits for the groups; a three-voter group
promotes first; a promotion needs a ready learner; a plan needs a placeable outsider; N5 a drain finishes. Six semantic
controls and two policy controls.

Source-mapped abstract reasoning with explicit premises: not verified Rust extraction;
silent about which learner is promoted and about liveness when no outsider exists.
