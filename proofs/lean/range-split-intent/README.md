# Range-split intent model

Finite-trace projection of one parent range's committed split intent (docs/RANGE-SPLIT.md §2),
the authority seam of the re-entered split: whether the parent is bound and unsealed, whether
the key is inside, whether the children live on the parent's committed members, how many intents
the parent has, and the later parts (seal, publication, rerouting) that this increment never
touches.

| Model element | Implementation |
| --- | --- |
| `unbind` / `keyOutside` / `childrenElsewhere` | the premises `plan_split` and `validate_against` check against the committed catalog |
| `intend` | `plan_split` (kind 113, `crates/meta/src/data_groups/split.rs`): the two children created and activated in the same transaction, one live intent per parent, exact confirmation |
| no seal / publish / reroute step | parts 2–4 of RANGE-SPLIT, later increments |

Ten theorems: safety initialization/preservation/reachability; one intent per parent; a repeated
intent confirms; an intent is permanent; an intent needs a bound unsealed parent, a key inside and
the members; nothing seals, publishes or reroutes here; an intent readies the children; the chain
reaches an intent. Six semantic controls and two policy controls.

Source-mapped abstract reasoning with explicit premises: not verified Rust extraction; silent
about sealing, population, publication and serving after the split.
