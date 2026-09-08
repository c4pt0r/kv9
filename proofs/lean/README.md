# Checked protocol lemmas

Toolchain: the exact version in `lean-toolchain`. The project uses Lean's standard
library only. From the repository root, run:

```sh
python3 scripts/check-proofs.py --lean /path/to/lean --self-test
```

The checker compiles fresh sources, requires every module in the theorem
inventory, and inspects each theorem's transitive axioms. Only Lean's standard
foundations (`propext`, `Classical.choice`, `Quot.sound`) are accepted. Three
controls must fail for their specific reasons: a proof hole, an insufficient
membership bound, and an otherwise accepted proof using a custom axiom. The
`protocol-proofs` CI job also checks the exact theorem/control counts externally.

`Quorum.lean` proves disjoint vote bounds, intersection of two strict majorities,
unique election under a fixed single-vote function, and quorum cardinality after
bounded failures. These results quantify over arbitrary finite membership lists
and are not an enumeration of a few cluster sizes.

## Refinement obligations remain open

- Membership must represent the actual authoritative configuration. Runtime
  membership validation must reject duplicated voter identities; joint consensus
  requires separate old/new quorum obligations.
- `unique_election` assumes a single stable vote function for the term. Proving
  that `DiskRaftStorage`, Ready ordering and crash recovery preserve that function
  is an additional obligation, not an axiom about the Rust code established here.
- Quorum cardinality is only an availability prerequisite. Eventual election,
  leader completeness, log matching, receipt correctness and conditional progress
  still need separate proofs and implementation mappings.
- These lemmas do not prove that a one-host test cluster survives losing its host.

The hard acceptance requirements and proof inventory are in
[`CORRECTNESS-GATES.md`](../../docs/CORRECTNESS-GATES.md).
