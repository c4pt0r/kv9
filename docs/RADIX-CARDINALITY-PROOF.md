# Radix cardinality and local machine-bound proofs

Updated: 2026-09-16 UTC. Tracking: [#51](https://github.com/c4pt0r/kv9/issues/51),
parent [#9](https://github.com/c4pt0r/kv9/issues/9).

Subsequent checkpoint: the [cursor/range model proof](RADIX-CURSOR-PROOF.md)
adds 80 theorems and 25 controls. The cardinality checkpoint below is preserved;
actual Rust loop and heap correspondence still remain open.

The radix model now proves that its stored element count matches the number of
distinct full keys throughout arbitrary finite mutation histories. The new
checkpoint adds **52 Lean theorems**, with **207 checked together**, and a new
set of **17 rejecting controls**. The earlier 155 point-operation theorems and
their separate 17 controls remain available in the
[previous checkpoint](RADIX-POINT-PROOF.md); the totals do not add 155 twice.

The Nat counter refinement is unconditional for valid initial modeled states.
The machine-word refinement is **conditional on resident results fitting the
address space**. Local slice/offset/index bounds are proved under their stated
path, search and length invariants. The actual Rust loop and heap simulations
must establish those premises. This is further algorithm-model evidence, not
complete Rust verification or permission to start timing or promote the index.

## Cardinality follows the actual operation model

`Cardinality.lean` establishes uniqueness from the existing full-key routing
invariant. A branch terminal cannot duplicate an entry below a byte edge, and
different edge labels cannot contain the same full key. No assumption saying
"the implementation is a correct map" is introduced.

The proof relates insertion to a permutation of the fresh key followed by the
old keys with that key removed. This connects the prior point-operation
semantics to exact cardinality, rather than assuming that equal point reads
alone imply an equal number of stored entries. Deletion removes zero or one
entry because full keys are unique.

`Counted` adds a separate stored `size` field and mirrors the source branches:
empty-root insertion initializes one; existing-root insertion adds the returned
inserted flag; absent deletion returns the unchanged state; successful deletion
subtracts one. `counted_history_size` proves this field equals the flattened
root's entry count after every finite history. Successful deletion has a
strictly positive old size. `counted_empty_iff` proves zero size agrees with the
absence of every point lookup.

## What the machine and indexing proofs cover

| Source operation | Checked model obligation | Still required from Rust correspondence |
| --- | --- | --- |
| `size += usize::from(inserted)` | `word_put_good` preserves exact cardinality with USize arithmetic if the resulting resident entry count fits | Distinct addressable entry storage and matching word width; actual insertion-loop simulation |
| `size -= 1` after the presence check | Presence implies positive size; subtraction cannot underflow and agrees with the resulting entry count | Presence/loop/heap correspondence to the same root |
| Full operation histories | `word_history_size` connects stored machine counts to the existing operation model for every resident finite history | Discharge the explicit per-result `ResidentHistory` premise from successful resident allocations |
| `depth + common`, split cuts and slices | Common-prefix bounds keep cuts within both keys; a matching prefix ends within the query | The loop maintains the modeled path/depth relation; slice lengths fit the machine word |
| `end + 1` on a found byte | The next depth stays within the query and strictly decreases the remaining-byte measure; both word additions are exact | A found-byte observation and the loop's current depth relation |
| Detached child removal and restoration | The old successful search index is a valid insertion index after removal | The actual frame retains that index and the matching parent vector |
| Cursor `index + found.is_ok()` | Hit/miss cases stay within the edge vector; word addition cannot wrap | Rust binary-search result and vector/model correspondence |
| Edge vector length | Strict unsigned-byte order implies at most 256 child edges, fitting USize | The existing ordered-vector invariant corresponds to the model |
| Prefix collapse | Parent prefix, edge and child prefix fit within every modeled descendant key | A valid descendant and actual heap/loop correspondence |

USize's standard Lean definitions model 32- or 64-bit word arithmetic. The
proofs establish exact addition/subtraction where Rust must avoid wrapping or
overflow panic. They do not prove the Rust compiler, Vec/slice library or
allocator. In particular, `resident_count_fits` only bridges an explicit
distinct-entry-storage and addressability relation to the count bound; it is
not a heap proof, an inserted runtime guard, or a new public key-length limit.

## Verification and retained evidence

The [source contract](../proofs/lean/radix/size-contract.json) binds the twelve
modules, exact Rust prototype/tests, prior point contract and both verifier
sources. The pinned Lean 4.33.1 run compiles in fresh directories with warnings
denied and `autoImplicit` disabled. Dependency inventories allow only `propext`,
`Classical.choice` and `Quot.sound`; they do not hide the external source/heap
premises.

Eleven semantic controls alter Nat/word counter transitions or assert false
seek/descent/restoration bounds. They fail substantive proofs, rather than only
parsing or lint. A proof hole and custom false axiom are rejected separately.
Four Rust substitutions exercise the same source-hash gate used by the runner;
these are binding checks, not semantic verification of the changed Rust.

The [manifest](radix-size-proof-v1/manifest.json),
[result](radix-size-proof-v1/result.json), [audit](radix-size-proof-v1/audit.json)
and [archive](radix-size-proof-v1/evidence.tar.gz) retain the accepted run, all
controls and the earlier authoring diagnostics. The ten prior proof modules,
candidate Rust and production crates are unchanged. The existing
[213-test Rust qualification](PERSISTENT-RADIX-PROTOTYPE.md) remains applicable;
it was not rerun or counted as new tests in this phase.

## Remaining complete algorithm gate

1. Prove actual cursor seek/traversal, bytewise ordering, bounded ranges,
   predecessor and mixed-direction crossover.
2. Simulate the actual iterative insertion and detached deletion frames,
   establishing the local path, offset and vector-index premises proved here.
3. Establish Arc/COW heap correspondence, saved-root ownership and reclamation,
   including the distinct resident storage needed by the word-count bound.
4. Differentially execute the Lean model and exact Rust candidate to test the
   reviewed source mapping, supplementing the proofs.

Only after those obligations close should the predeclared matched performance
screen start. This phase has no timing, new database QPS, Redis comparison,
recovery/Chaos acceptance or runtime promotion. Production keeps rpds. All
verification is local; hosted CI remains manual-only.
