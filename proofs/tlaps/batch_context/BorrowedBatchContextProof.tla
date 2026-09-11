-------------------- MODULE BorrowedBatchContextProof --------------------
EXTENDS Naturals, TLAPS

\* Positions are one-based here. Rust position 0 is position 1 below.
\* owns[i] abstracts the logical verdict of the original owner check at
\* position i on one fixed metadata view. It is not an I/O outcome oracle.
BCAnchorResolved(n, owns) == n = 0 \/ owns[1]
BCAllOwners(n, owns) == \A i \in 1..n : owns[i]
BCTailOwners(n, owns) == \A i \in 2..n : owns[i]
BCOldFailures(n, owns) == {i \in 1..n : ~owns[i]}
BCNewFailures(n, owns) == {i \in 2..n : ~owns[i]}
BCFirstFailure(failures, i) ==
    /\ i \in failures
    /\ \A j \in failures : i <= j

\* No positive batch-length assumption: internal empty batches are included.
THEOREM BCPositionCoverage ==
    ASSUME NEW CONSTANT n \in Nat
    PROVE /\ \A i \in 1..n : i = 1 \/ i \in 2..n
          /\ \A i \in 2..n : i \in 1..n
BY SMT

\* Materializing key references and indexing the borrowed request have the
\* same pointwise key projection. There is no uniqueness premise on keys.
THEOREM BCBorrowedProjection ==
    ASSUME NEW CONSTANT n \in Nat,
           NEW CONSTANT Keys,
           NEW CONSTANT keys \in [1..n -> Keys]
    PROVE [i \in 1..n |-> keys[i]] = keys
BY Isa

THEOREM BCAuthorizationRefinement ==
    ASSUME NEW CONSTANT n \in Nat,
           NEW CONSTANT owns \in [1..n -> BOOLEAN],
           BCAnchorResolved(n, owns)
    PROVE BCAllOwners(n, owns) <=> BCTailOwners(n, owns)
BY BCPositionCoverage, SMT
   DEF BCAnchorResolved, BCAllOwners, BCTailOwners

\* Header means keyspace existence, Raw API type, resolved anchor and epoch
\* have all passed. The theorem does not assume arbitrary headers pass.
THEOREM BCGatedAuthorizationRefinement ==
    ASSUME NEW CONSTANT n \in Nat,
           NEW CONSTANT owns \in [1..n -> BOOLEAN],
           NEW CONSTANT header \in BOOLEAN,
           header => BCAnchorResolved(n, owns)
    PROVE (header /\ BCAllOwners(n, owns))
          <=> (header /\ BCTailOwners(n, owns))
BY BCAuthorizationRefinement, SMT

\* Since only an already successful position is removed, every failing
\* non-anchor position, including a duplicate key position, remains.
THEOREM BCFailurePositionsRefinement ==
    ASSUME NEW CONSTANT n \in Nat,
           NEW CONSTANT owns \in [1..n -> BOOLEAN],
           BCAnchorResolved(n, owns)
    PROVE BCOldFailures(n, owns) = BCNewFailures(n, owns)
BY BCPositionCoverage, SMT
   DEF BCAnchorResolved, BCOldFailures, BCNewFailures

THEOREM BCFirstFailureRefinement ==
    ASSUME NEW CONSTANT n \in Nat,
           NEW CONSTANT owns \in [1..n -> BOOLEAN],
           NEW CONSTANT i \in Nat,
           BCAnchorResolved(n, owns)
    PROVE BCFirstFailure(BCOldFailures(n, owns), i)
          <=> BCFirstFailure(BCNewFailures(n, owns), i)
BY BCFailurePositionsRefinement, SMT DEF BCFirstFailure
=============================================================================
