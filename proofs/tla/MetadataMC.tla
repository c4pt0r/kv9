---------------------------- MODULE MetadataMC ----------------------------
EXTENDS MetadataPlanning

DistinctNames == [r \in Requests |-> r]
SameNames == [r \in Requests |-> "shared"]

\* Deliberately false invariants produce witnesses for non-vacuous scenarios.
NoTwoSuccesses == Cardinality(succeeded) < 2
NoUnknownCommit ==
    ~\E r \in unknownPending :
        /\ writeAt[r] <= committed
        /\ Exact(writeAt[r], "write", r, writeTerm[r])
NoReplacedWrite ==
    ~\E r \in Requests :
        /\ writeAt[r] \in 1..committed
        /\ ~Exact(writeAt[r], "write", r, writeTerm[r])

=============================================================================
