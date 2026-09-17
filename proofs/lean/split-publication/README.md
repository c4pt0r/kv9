# Split-publication model

Finite-trace projection of the manual split's finale: the atomic
one-to-two publication. Intent, fence, population and partition-directory
semantics are premises from their own models.

| Model element | Implementation |
| --- | --- |
| `commit`/`fence`/`verifyLow`/`verifyHigh` | the qualified prior increments (kind-107 intent, group-side seal, digest-verified population re-checked LOCALLY by the publishing metadata voter) |
| `publish` (full chain required; one atomic transition) | `publish_split`: ONE catalog transaction sealing the parent binding (`may_follow` version+1) and inserting both child bindings with their region rows (`crates/meta/src/data_groups/split.rs`, `PublishSplit`, CLI `publish-split`) |
| routing flips exactly at publication | the partition read model + key-aware raw-directory resolution (`get_for`) with replace-on-insert refresh |
| `confirm` | idempotent re-publication of the identical directory |
| no partial/lost-row step | the partition rules refuse every partial shape; both digests re-verify before the transaction |

Ten theorems: safety initialization/preservation/reachability,
publication requires the full chain, routing flips exactly at
publication, no partial publication and no lost row, publication is
permanent, a confirmation changes nothing, verified population alone
publishes nothing, and the chain reaches the published split. The runner
checks six semantic mutation controls and two proof-policy controls.

Source-mapped abstract reasoning with explicit premises: not verified
Rust extraction; silent about post-split parent retirement, automatic
triggers and scaling.
