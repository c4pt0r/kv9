# Catalog-scope model

Finite-trace model of one scoped node's catalog (docs/META-REPLICA-MODES.md §2, §3b, §3a R2):
the stream of committed catalog rows a full node routes to it, the regions it is subscribed to,
and what it stores. Written before the implementation, as the specification the Rust code
(`CatalogScope` stream, the row → regions map, the subscriber's applier) must refine.

| Model element | Design |
| --- | --- |
| `Row.regions`, `Row.declared` | the row → regions map by kind (§3b); `declared = false` is a kind with no map |
| `deliver s at r` | the subscriber's applier: one message carrying the row committed at position `at` |
| `relevant scope r` | the router's test "does this row concern a region the subscriber holds" |
| `enter s q` | the message that adds region `q` to the scope, carrying its closure at the current position |
| `leave s q` | the drop marker for region `q` |
| `seen` (ghost) | the full catalog up to the subscriber's position |

Sixteen theorems: safety initialization, preservation by every step and reachability; for a
scoped region the local verdict "is there a row with property P" equals the full catalog's at
the same position, for every P (M1, the absence argument); a node never holds a row the catalog
did not commit by its position (M3); a change that concerns none of a node's regions delivers no
rows, only a position (R2, no broadcast); every delivered row concerns a scoped region; a message
for any other position is ignored and a duplicate is the identity (idempotence); positions never
go back; an undeclared row stops the stream and nothing moves afterwards (fail closed); every row
a node has seen is declared.

Seven semantic controls, each a one-line mutation that must break a theorem: dropping a relevant
row, entering a region without its closure, storing an unrelated row, applying a message at any
position, skipping an undeclared row, delivering after the stream stopped, storing a row the
catalog lacks. Two policy controls reject `sorry` and custom axioms.

What it does not cover: that the Rust row → regions map is total and correct for every catalog
key (a unit test over every table and task kind, failing on any unmapped kind, is part of the
implementation); how full nodes compute subscribers; liveness.

## Refinement (2026-10-30)

The implementation's sources are pinned in `source-contract.json`; the prover refuses a stale pin.

| Model | Rust |
| --- | --- |
| `Row.declared` | `kv9_meta::scope::classify` returning `Some(_)`; `task_subject` for TASKS rows over `DECLARED_KINDS` |
| `Row.regions`, `relevant scope r` | `kv9_meta::scope::route` against a `Closure` (per keyspace, `closure`); `Route::Deliver` |
| `enter s q` with its closure | a widening op returns `Route::Resnapshot`: the server sends a whole snapshot at the current position (`catalog_scope/serve.rs`), which `snapshot_rows` computes with the same `route` |
| `leave s q` | a row that stops routing (or a delete of a held key) is sent as a delete; a snapshot replaces the slice |
| `deliver s at r` only at its position | `catalog_scope/subscribe.rs`: a delta applies iff its `prev` is the slice position; a snapshot older than the slice is refused |
| positions monotonic | `write_applied` refuses a non-advancing position; a same-position snapshot rewrites without moving it |
| an undeclared row stops the stream | `Route::Undeclared` and `snapshot_rows` end the stream with an error (`catalog_scope_error=`) |

The totality obligation the model leaves to the implementation is
`every_key_a_data_keyspace_and_a_drain_write_is_declared` in `crates/meta/src/data_groups/subject.rs`,
plus the e2e (`scripts/meta-scoped-e2e.sh`), which streams every key its verbs write through `classify`.
