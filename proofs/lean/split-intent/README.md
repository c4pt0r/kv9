# Split-intent model

Finite-trace projection of one parent range's committed split intent —
the D04 authority seam. Catalog row immutability and the current
single-range routing model are premises from the data-range model.

| Model element | Implementation |
| --- | --- |
| `bindParent` | committed kind-102 range binding (unsealed) |
| `activateChildren` | two committed, activated, UNBOUND child creations on the parent's replica set |
| `record` (bound unsealed parent, ready unbound children, once per parent) | `plan_split` kind-107 row (`crates/meta/src/data_groups/split.rs`, `RecordSplitIntent`, CLI `record-split-intent`) |
| no seal/populate/republish/reroute/serve step | every one of those is a later D04 increment |

Eleven theorems: safety initialization/preservation/reachability, an intent
requires the bound parent and ready children, nothing seals here, nothing
republishes or reroutes here, no serving capability, an intent is
permanent, one intent per parent, readiness alone records nothing, and
the committed chain reaches the intent. The runner checks seven semantic
mutation controls and two proof-policy controls.

Source-mapped abstract reasoning with explicit premises: not verified
Rust extraction; silent about sealing, child population, atomic directory
republication, rerouting and scaling.
