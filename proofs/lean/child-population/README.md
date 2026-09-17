# Child-population model

Finite-trace projection of one split's child population under the parent
fence. Seal semantics and child-log atomicity are premises from their own
models.

| Model element | Implementation |
| --- | --- |
| `fence` | the sealed parent range row (parent-seal model) |
| `copyLow`/`copyHigh` (fence required; exact by construction) | `populate_split_child`: bounded `Command::Write` batches through the child's own log, then the parent-half and child digests recomputed independently — a mismatch refuses instead of committing (`crates/server/src/runtime.rs`) |
| `recopy` (fence + already copied) | idempotent rerun converging to the same rows, including after a child leader restart |
| no divergence/serving step | a divergent child never commits; routing still points at the fenced parent until the atomic publication |

Eleven theorems: safety initialization/preservation/reachability,
population requires the fence, every committed copy is exact, no
divergent child ever commits, children serve nothing here, a copy is
permanent, a rerun converges, a fence alone copies nothing, and the chain
reaches both exact children. The runner checks six semantic mutation
controls and two proof-policy controls.

Source-mapped abstract reasoning with explicit premises: not verified
Rust extraction; silent about the atomic publication, rerouting and
scaling.
