# Range-merge model

Finite-trace projection of one merge of two adjacent sibling ranges into one child
(docs/RANGE-SPLIT.md §6): the intent's premises (adjacent, same committed members, both
unsealed), the child serving, the two seals, the copy of the low parent then the high parent,
verification, evidence and the one-transaction publication.

| Model element | Implementation |
| --- | --- |
| `intend` | `plan_merge` (kind 115, `crates/meta/src/data_groups/merge.rs`): the child created and activated in the same transaction, one live merge per pair |
| `serve` / `sealLow` / `sealHigh` | `runtime/range_merge.rs`: the child's descriptor `[low.start, high.end)`, then each parent leader's `DataRange` CAS once the child serves |
| `copyLow` / `copyHigh` / `verify` | the child leader's pages from the local sealed low replica, then the high one; both chained through one digest against the child |
| `evidence` | `plan_merge_evidence` (kind 116) over the shared evidence RPC |
| `publish` | `plan_merge_publication`: both parent bindings sealed in place, the child bound, in one transaction |

Thirteen theorems: safety initialization/preservation/reachability; one intent per pair; an intent
needs adjacent unsealed siblings on one store set; evidence means both halves verified; the low
parent is copied first; a copy reads only sealed parents; a seal waits for the child; publication
needs the evidence; routing follows publication atomically; retirement needs publication; the chain
reaches a published merge. Eight semantic controls and two policy controls.

Source-mapped abstract reasoning with explicit premises: not verified Rust extraction; silent
about the digest function; retirement is modelled as the step the sealed binding authorizes.
