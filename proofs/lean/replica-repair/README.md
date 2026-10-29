# Replica-repair model

Finite-trace projection of one data-group leader's repair decision (docs/REPLICA-REPAIR.md
§1–§3): the silence window of one suspect voter, the leader's own role, the candidate
destination's standing (active, non-member, not the suspect) and the committed plan count
for this loss. The replica-plan driver that carries the plan out has its own model
(`proofs/lean/replica-addition`).

| Model element | Implementation |
| --- | --- |
| `tick` / `heard` | `peer_progress` observation per pass: `recent_active` or a `matched` advance resets the window |
| `lose_leadership` / `lead` | the group driver's role; observations are dropped on a role change |
| `decide` | `NodeRuntime::reconcile_replica_repair` → `AddReplica` (locally at the metadata leader, or over the internal `PlanReplica` RPC) with mode move, source = the lost incarnation, destination = the least-loaded active outsider; derived operation id |

Nine theorems: safety initialization/preservation/reachability; one plan per loss; a
repeated decision confirms; a decision requires leadership, the full window, a committed
voter as source and an active outsider as destination; a plan names the suspect and an
outsider; hearing from the voter resets the window; a lost voter gets a plan. Six semantic
controls and two policy controls.

Source-mapped abstract reasoning with explicit premises: not verified Rust extraction;
silent about the quality of the destination choice and about liveness when no outsider
exists.
