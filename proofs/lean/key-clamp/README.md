# Safe key-accessor refinement

`KeyClamp.lean` rechecks the 23 single-buffer representation statements with
the abstract key accessor changed to `take (min keyLength bytes.length)`.
Three more lemmas establish the valid-entry bound identity, equivalence to the
original key prefix under that bound, and validity preserved by copying both
private fields. All 26 theorems pass with warnings denied; all 15 semantic,
source-substitution, proof-hole and custom-axiom controls reject.

The reviewed Rust type has private fields, one checked constructor and derived
Clone. Its constructor establishes `key_len <= bytes.len()`; Clone preserves
the bytes and length. No deserialization or mutable field API is exposed.
Under that invariant, the safe `min` changes neither the key prefix nor any
comparison, borrowed lookup or returned value. The value suffix accessor and
whole-batch allocation-length preflight are unchanged.

This remains conditional representation refinement with reviewed source mapping,
not verified Rust extraction. Safe Rust Vec/Box/slice/Clone behavior, compiler
correctness and the pinned rpds ordering, replacement and persistent-snapshot
contracts are explicit premises. Association-list updates describe contents;
sorted traversal is an rpds premise. Allocation failure, lock poisoning and
Raft are not newly proved. Invalid private buffers are not reachable through
the reviewed safe API; their panic/truncation behavior is not an API promise.

The [source contract](source-contract.json) binds the actual candidate, proof,
runner and inherited models. `scripts/key-clamp/prove.py ROOT LEAN` checks those
hashes and permits only `propext`, `Classical.choice` and `Quot.sound` as theorem
axioms. The initial split-lemma authoring diagnostic is retained separately.
The corrected proof uses the library's `take_eq_take_min` identity explicitly.
