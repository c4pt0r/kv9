# Guarded shared-prefix ordering

`Prefix.lean` proves 18 universal statements over lexicographically ordered
lists of naturals. Unsigned byte keys are a specialization. No finite search
bound, unchecked proof hole or project-specific axiom is used.

The statements establish:

- Removing a prefix shared by both operands preserves lexicographic ordering.
- Computing a common prefix gives a prefix of both operands; shortening an
  existing cache admits a new key while preserving all existing keys.
- Keys between two ordered endpoints share those endpoints' common prefix.
- Prefix membership implies sufficient length and the expected take/drop laws.
- The guarded comparison returns exactly the full-key comparison; a failed
  guard uses the original comparison, including short and unrelated queries.
- Removing keys preserves prefix membership; insertion with a shortened cache
  preserves membership; endpoint reconstruction covers a bounded node.
- Checked operands admit both Rust slice bounds; skipping zero bytes is exact.

The source contract binds the reviewed declarations and full candidate library
hash. It maps insertion admission to prefix shortening, deletion to subset
preservation, and split/borrow/merge reconstruction to the endpoint lemma. The
last endpoint is the final key of the final child subtree, not its separator.
Rust `starts_with` implements the query prefix and length guard.

**Scope:** ordered node intervals, correct separator maintenance and safe Rust
ownership/slice semantics are premises. This is not a mechanized refinement of
Rust, a full proof of B+tree insertion/deletion, or a Raft/durability theorem.
Source hashes detect drift; they do not establish program equivalence. Full
implementation-bound index correctness remains required before production.

```sh
python3 -B scripts/check-packed-prefix-proof.py \
  --lean /path/to/lean-4.33.1/bin/lean \
  --output /path/to/fresh-output
```

The checker compiles fresh modules with warnings as errors and audits transitive
axioms for every theorem. Only Lean's `propext`, `Classical.choice` and
`Quot.sound` foundations are accepted. It also rejects reversed suffix ordering,
an excessive slice bound, `sorry`, a custom false axiom, and four source edits
that bypass the query guard, skip an extra byte, use a separator as the subtree
end, or omit insertion admission. Source-edit controls test binding enforcement;
they are not compiled Rust fault-injection tests.

Original failed proof drafts and their errors are retained alongside the passing
execution in the experiment evidence. No Chaos Mesh acceptance follows from
these local checks.
