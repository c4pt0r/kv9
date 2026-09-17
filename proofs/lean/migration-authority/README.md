# Committed migration authority model

Finite-trace projection of the committed migration intent and image-owner
binding for one data group. `authority` abstracts the exact committed
root/operation/group/destination-incarnation binding; `image` abstracts the
canonical manifest digest and its complete SST closure. Replication atomicity,
ledger recovery and the immutable owner-binding rule are premises supplied by
the existing retention-ledger TLA/TLAPS model, not re-proved here.

The model has deliberately **no transition** that quiesces or releases either
pin, and none that grants voting or serving: `Pin.quiesced`/`Pin.released`
exist in the state space precisely so their unreachability is a theorem, not a
typing accident. Confirmation of an existing intent is a pure receipt.

| Model element | Implementation |
| --- | --- |
| `commit` / `confirm` | `plan_migration` insert / exact-retry path (`crates/meta/src/data_groups/migration.rs`) |
| `acquire`, `publishSource` | `MigrationOwners::bind` source flow (`crates/server/src/migration_retention.rs`) |
| `share`, `publishDestination` | `MigrationOwners::bind` destination flow through `LedgerRequest::Share` |
| no quiesce/release step | migration-kind fence in `plan_retention` (`crates/meta/src/retention.rs`) |
| `voting = serving = false` | no RegionManager, transport or peer path consumes the new rows |

14 theorems: safety initialization/preservation/reachability, owners require
the committed intent, the destination owner requires the published source and
binds the exact subject, quiesce/release/voting/serving unreachability, four
monotonicity theorems, a committed description binding nothing by itself, and
confirmation being a pure receipt. The runner checks ten semantic mutation
controls and two proof-policy controls, and audits axiom dependencies.

This is source-mapped abstract reasoning with explicit premises. It is not
verified Rust extraction, not a transfer/settlement model, and proves nothing
about network snapshots, learner catchup or horizontal scaling.
