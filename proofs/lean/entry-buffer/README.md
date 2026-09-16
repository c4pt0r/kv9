# Single key/value buffer refinement

`EntryBuffer.lean` proves 23 representation lemmas in Lean 4.33.1. The
[source contract](source-contract.json) binds the reviewed Rust type, candidate
engine integration, proof, model, preparation and allocator probe. It also pins
the actual rpds source whose equal-key insertion replaces the entire stored
Entry, including the key object carrying the value bytes.

The proof covers key/value round trips, the checked combined length, key-only
equality/order/borrowing, lookup, complete same-key replacement, deletion,
range/reverse/limit projection, arbitrary ordered mutation histories and
whole-batch length refusal before mutation. Association-list updates describe
map contents; ordered traversal and persistent old roots are rpds premises.

This is conditional refinement with a reviewed source mapping, not verified
Rust extraction. Safe Rust Vec/Box/slice/copy behavior, compiler correctness,
byte comparison and rpds ordering/replacement/persistence remain premises.
Allocation failure, tree balancing, locks, poisoning and Raft are not modeled.
Generic rpds map equality would ignore payload bytes because the key's Eq sees
only key bytes and the map value is unit. The private engine does not use that
equality, serialization or mutable map-value access as public value semantics.

`scripts/entry-buffer/prove.py ROOT LEAN` denies warnings and checks each theorem's
axioms against `propext`, `Classical.choice` and `Quot.sound`. Thirteen controls
reject: wrong key/value boundaries, whole-buffer order/equality, omitted length
cap, empty replacement value, retained deleted keys, reversed histories,
mutation after refused preflight, a proof hole, a custom axiom, and two Rust
source substitutions. A source hash is an audit binding, not a proof that the
compiler implements the abstract model. Initial proof-authoring diagnostics
remain separate in the evidence packet.

See the [qualification report](../../../docs/ENTRY-BUFFER-QUALIFICATION.md) for
actual runtime tests, layout observations, source audit and retained limits.
