# Learner-attach model

Finite-trace projection of one destination's attachment to one source group
under one committed migration operation. Group-consensus atomicity of the
configuration entry and the capture model's cut semantics are premises from
their own models.

| Model element | Implementation |
| --- | --- |
| `commit` | committed migration intent (kind-103 row) |
| `attach` (intent required, learner only, cut advance) | `attach_migration_learner`: `driver.add_learner` + `wait_conf_applied` + `Command::Noop` cut push (`crates/server/src/runtime.rs`) |
| `confirm` (re-advances the cut) | idempotent attach retry receipt |
| no voter/serving step | no promotion path exists; the installed generation stays isolated |

Nine theorems: safety initialization/preservation/reachability, attach
requires the committed intent and an advanced cut, the destination is never
a voter, no serving capability, learner monotonicity, confirmation as a pure
cut-advancing receipt, and an intent alone attaching nothing. The runner
checks seven semantic mutation controls and two proof-policy controls.

Source-mapped abstract reasoning with explicit premises: not verified Rust
extraction; silent about replication, catchup, promotion and scaling.
