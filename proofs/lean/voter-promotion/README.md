# Voter-promotion model

Finite-trace projection of one migration destination's promotion from
attached learner to voter. Attach atomicity, adoption/evidence settlement
and raft quorum semantics are premises from their own models.

| Model element | Implementation |
| --- | --- |
| `commit` | committed migration intent (kind-103 row) |
| `attach` (migration, not already a voter) | `attach_migration_learner` |
| `record` (attached) | committed kind-104 install evidence via the adoption chain |
| `promote` (evidence + learner; one-way) | `promote_migration_voter`: committed-evidence check, leader-only `driver.promote_voter` (`ConfChangeType::AddNode`) through the group's own log, idempotent confirm (`crates/server/src/runtime.rs`) |
| no demotion/serving step | no removal path exists yet; reads still refuse at non-leaders |

Eleven theorems: safety initialization/preservation/reachability, promotion
requires the committed evidence, evidence implies membership, never learner
and voter at once, no serving capability, a voter is permanent, the learner
never returns after promotion, an evidence row alone promotes nothing, and
the committed chain reaches the voter. The runner checks seven semantic
mutation controls and two proof-policy controls.

Source-mapped abstract reasoning with explicit premises: not verified Rust
extraction; silent about removal, stranded-learner recovery, split,
placement and scaling.
