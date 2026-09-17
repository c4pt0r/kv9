# Partition-directory model

Finite-trace projection of one keyspace's range directory across a split,
as the generalized read model validates it. Catalog atomicity is a premise
from the metadata Raft machinery; split authority from the split-intent
model.

| Model element | Implementation |
| --- | --- |
| `legacy` shape | one unsealed full-keyspace binding (the pre-split directory) |
| `split` shape | sealed parent skipped for routing + children partitioning exactly (`read_in`'s sorted/no-overlap/no-gap rules, `crates/meta/src/data_groups/ranges.rs`) |
| `publish` (atomic, one-way) | the future one-to-two catalog transaction — the ONLY legal transition; today no writer exists and the read model refuses every partial shape |
| `route` (always possible, never through sealed) | `route_in` filtering `!sealed` and picking the covering child |

Eleven theorems: safety initialization/preservation/reachability, coverage is
never lost, the directory is never in both shapes, no partial publication
exists, sealed ranges never route, no serving capability is minted, a
split directory is permanent, routing is always possible, and the atomic
publication reaches the split shape. The runner checks five semantic
mutation controls and two proof-policy controls.

Source-mapped abstract reasoning with explicit premises: not verified Rust
extraction; silent about sealing execution, child population, the
publication writer itself and scaling.
