# Auto-split model

Finite-trace projection of the automatic trigger over the proven manual
pipeline. Every pipeline stage's own safety (intent, seal, population,
publication, retirement) is a premise from its model.

| Model element | Implementation |
| --- | --- |
| `bind` | a committed, unsealed, keyspace-bound range |
| `observe` | the metadata leader's local-replica disk observation (`engine_disk_bytes`: active WAL + segments) crossing `KV9_AUTO_SPLIT_BYTES` |
| `trigger` (bound + breached, once per region) | the deterministic operation (`sha16("kv9-auto-split-v1" ++ root ++ region)`) with children created/activated and the kind-107 intent committed (`reconcile_auto_split`, `crates/server/src/runtime.rs`) |
| `drive` | the role-local pipeline steps: seal at the parent leader, population at child leaders, publication at the metadata leader when both digests verify |
| `resume` | committed state IS the coordination: any coordinator re-derives the same operation and continues |
| no second-trigger/skip step | one intent per parent region; every stage's own committed gate stays |

Eleven theorems: safety initialization/preservation/reachability, a
trigger requires a breached bound range, the pipeline runs only when
triggered, no second trigger fires, no stage is skipped, a trigger is
permanent, any coordinator resumes, a breach alone triggers nothing, and
the triggered chain completes. The runner checks five semantic mutation
controls and two proof-policy controls.

Source-mapped abstract reasoning with explicit premises: not verified
Rust extraction; silent about cascade splits, load-based triggers,
hysteresis beyond one-trigger-per-region, and scaling.
