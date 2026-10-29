# Automatic split trigger model

Finite-trace projection of one data group's leader-side split trigger (docs/RANGE-SPLIT.md §11):
the replica footprint against the threshold, the leader role, the binding (bound, unsealed), a
live split/merge/plan on the group, the node's cooldown, the sampled row count and the committed
intents for the binding.

| Model element | Implementation |
| --- | --- |
| `write` | writes grow `data.wal` + `data.segments/` (`auto_split::footprint`) |
| `otherOperation` / `settle` | a committed split or merge naming the group, or an unsettled replica plan |
| `heat` / `cool` | `KV9_AUTO_SPLIT_COOLDOWN_SECS` on the deciding node |
| `decide` | `NodeRuntime::reconcile_auto_split`: the median user key from one snapshot, the derived operation `sha16("kv9-auto-split/v1" ++ root ++ region ++ bind task)`, `SplitRange` locally or the internal `PlanSplit` |

Nine theorems: safety initialization/preservation/reachability; one intent per binding; a repeated
decision confirms; a decision needs every guard; a decision starts the cooldown; the key is inside;
writes alone reach a decision. Seven semantic controls and two policy controls.

Source-mapped abstract reasoning with explicit premises: not verified Rust extraction; silent about
the quality of the median sample, about the seal window the decision opens and about cascades
beyond the cooldown.
