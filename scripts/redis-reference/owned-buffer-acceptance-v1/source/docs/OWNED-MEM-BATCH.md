# Move owned mutation buffers into the resident index

This experiment builds on CRC candidate `ca0002c7`. It removes the additional
key/value clones in `MemEngine::write` and `write_applied`: both APIs already
receive an owned `WriteBatch`, so its ordered mutation vector can be consumed.
No new public API, index structure, RPC path, persistence format or Raft rule
is introduced. Performance remains unmeasured.

## Observational equivalence

For each column family and byte key/value, the old Put inserts clones of the
input byte vectors; the replacement inserts those owned vectors themselves.
Cloning a `Vec<u8>` preserves its byte sequence, and the engine exposes byte
values rather than the allocation identity of those input buffers. Delete
passes the same key bytes to the same ordered-map removal operation. Both
loops visit the mutation vector in its original order. Induction over that
sequence therefore gives the same map after every completed mutation and the
same final map, including duplicate keys and Put/Delete/Put sequences.

The persistent map implementation still preserves any retained snapshot roots.
Moving a new entry's buffers does not modify an older shared entry. Shared
tree nodes may still be copied by the persistent-map implementation; this
change does not claim that snapshots make later writes free.

Position validation remains before any state mutation or batch consumption.
For accepted positioned writes, the same pure `any` predicate is evaluated
before consuming the vector: a non-manifest-prefixed physical key in any CF
advances the data revision once, with the original saturating increment.
It remains short-circuiting. This includes absent deletes and net-zero batches;
empty and manifest-only batches do not advance the revision. Evaluating the
predicate earlier changes no data because it reads only immutable input keys.
Plain writes still do not update the revision or applied position.

The same state write lock spans ordered map changes, revision update and
applied-position publication. Readers therefore retain the same atomic
visibility boundary. `WalEngine` continues to borrow the complete batch for
append/sync before passing ownership to the resident index. Its log/state lock
order, refusal checks, replay order and durability reporting are unchanged.

This is a source-level equivalence argument using Rust's owned byte-vector
semantics and the unchanged persistent-map contract. It is not a new
machine-checked whole-Rust/Raft refinement. Allocation failure behavior is
not a promise of identical instruction-by-instruction execution.

## Validation and selection

The public-API regression file `crates/engine/tests/owned_batch.rs` covers
revision classification across all CFs, both write entry points, duplicate
ordering, empty values, retained snapshots and refused positions leaving all
data/revision/position state unchanged. It does not assert private allocation
addresses or infer speed from fewer clones.

Local source validation passes all three public behavior regressions, 712
workspace tests (23 ignored), all-target Clippy with warnings denied, and
formatting. These are source gates, not recovery or performance acceptance.
Compare against the CRC parent with identical clients and
resource budgets, retaining whole-call mean/p95/p99 and every outcome. The
existing CRC-only fault and workload recordings remain bound to their original
source and binary; they cannot serve as this candidate's recovery or performance
acceptance. Require separate exact-source process/Chaos and workload acceptance
before selecting this next increment.
