# Parent-seal model

Finite-trace projection of one split parent's group-side write fence.
Split-intent authority and group-log atomicity are premises from their
own models.

| Model element | Implementation |
| --- | --- |
| `commit` | committed kind-107 split intent |
| `fence` (intent required; one-way) | `seal_split_parent`: leader-only `Command::DataRange` CAS from the current row digest to its sealed version+1 successor (`may_follow`), applied through the parent group's own log (`crates/server/src/runtime.rs`) |
| `restart` | recovery replays the sealed row; `authorize` reads it per request and refuses with a stale epoch |
| no serve/catalog/unseal step | a sealed parent serves nothing; the catalog directory changes only in the atomic publication increment; sealing never reverts |

Eleven theorems: safety initialization/preservation/reachability, a seal
requires the committed intent, nothing serves through a seal, the catalog
is untouched here, the seal never reverts, a seal is permanent, restart
preserves the fence, an intent alone fences nothing, and the committed
chain reaches the fence. The runner checks five semantic mutation
controls and two proof-policy controls.

Source-mapped abstract reasoning with explicit premises: not verified
Rust extraction; silent about child population, the atomic publication,
rerouting and scaling.
