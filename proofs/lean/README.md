# Checked protocol lemmas

TLA+ is the primary protocol specification; see the [TLA+ inventory](../tla/README.md).
These Lean results remain deductively checked lemmas with the scopes below.
Future protocol proofs may use TLAPS or Lean, with explicit model/source mappings.

Toolchain: the exact version in `lean-toolchain`. The project uses Lean's standard
library only. From the repository root, run:

```sh
python3 scripts/check-proofs.py --lean /path/to/lean --self-test
```

The checker compiles fresh sources, requires every module in the theorem
inventory, and inspects each theorem's transitive axioms. Only Lean's standard
foundations (`propext`, `Classical.choice`, `Quot.sound`) are accepted. Nine
controls must fail for their specific reasons: a proof hole, an insufficient
membership bound, an otherwise accepted proof using a custom axiom, and removal
of the durable namespace precondition from the vote-reply transition, removal
of certificate eligibility, weakening the strict real-time ordering premise, and a
frozen registration cursor, and removal of either public admission bound. The
`protocol-proofs` CI job also checks the exact theorem/control counts externally.

`Quorum.lean` proves disjoint vote bounds, intersection of two strict majorities,
unique election under a fixed single-vote function, and quorum cardinality after
bounded failures. These results quantify over arbitrary finite membership lists
and are not an enumeration of a few cluster sizes.

`DurableVote.lean` defines a fixed-term state machine with choose, data sync,
directory publication, reply, crash and stop transitions. Induction establishes
that all replies agree for arbitrary finite executions, including repeated
crashes. Its assumptions and mapping to the actual Ready/storage operations are
documented in [`RAFT-PERSISTENCE.md`](../../docs/RAFT-PERSISTENCE.md).

`History.lean` proves equivalence between a generic Boolean certificate evaluator
and a logical execution relation, plus the real-time ordering consequence of
operation interval bounds. It does not compile or verify the Python checker.
The concrete model, search argument and source refinement obligations are in
[`HISTORY-CHECKING.md`](../../docs/HISTORY-CHECKING.md).

`Registration.lean` proves cyclic cursor advancement, range preservation and
coverage of every first-seed position within one retry cycle. The induction
connects actual repeated updates to the modulo formula for arbitrary sizes and
pass counts. Its conditional routing-progress assumptions and Rust controls are
in [`REGISTRATION-ROUTING.md`](../../docs/REGISTRATION-ROUTING.md). The combined
inventory also includes `Admission.lean`: seven local resource declarations for
count/byte conservation, transition preservation and induction over arbitrary
finite reservation traces. Ownership and cancellation refinement obligations are
in [`PUBLIC-ADMISSION.md`](../../docs/PUBLIC-ADMISSION.md). The complete inventory
contains 24 declarations and nine invalid controls.

## Refinement obligations remain open

- Membership must represent the actual authoritative configuration. Runtime
  membership validation must reject duplicated voter identities; joint consensus
  requires separate old/new quorum obligations.
- `unique_election` assumes a single stable vote function for the term. The
  DurableVote state machine supplies an abstract preservation proof. Its Rust
  refinement still depends on the explicit filesystem, decoder and upstream
  Raft assumptions; this is not a mechanical verification of the Rust binary.
- Quorum cardinality is only an availability prerequisite. Eventual election,
  leader completeness, log matching, receipt correctness and conditional progress
  still need separate proofs and implementation mappings.
- These lemmas do not prove that a one-host test cluster survives losing its host.

The hard acceptance requirements and proof inventory are in
[`CORRECTNESS-GATES.md`](../../docs/CORRECTNESS-GATES.md).
