# Inline-key representation refinement

`InlineKey.lean` proves byte identity after inline-or-heap encoding, validity and
the exact u8 length cast, padding/representation independence, equality/order/
borrow coherence, lookup/erase/put preservation, range/reverse/limit projection,
and preservation of arbitrary ordered mutation histories. Nineteen theorems are
checked with warnings as errors and complete axiom-dependency reports.

The association-list mutation model represents map contents; its prepend-based
put is not a balanced or sorted tree implementation. Traversal projection
preserves the supplied sequence's order. Correct sorted traversal and persistent
snapshots are explicit rpds premises, also exercised against an independent
BTreeMap model through the actual engine interfaces. Rust safe copy, Vec/Clone,
byte comparison and compiler semantics remain premises. This is a conditional
representation proof with reviewed source correspondence, not verified Rust
extraction, allocation/lock/panic analysis or a new Raft proof.

The source contract binds both engine variants, the private key, patch generator
and proof. `scripts/inline-key/prove.py` rejects exposed padding, length-only
equality/order, retained deleted entries, reversed histories, proof holes, custom
axioms, changed capacity and changed Rust ordering. Source hash controls establish
that a reviewed mapping cannot silently substitute a different implementation;
they do not establish that mapping automatically.
