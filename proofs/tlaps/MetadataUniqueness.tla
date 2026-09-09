----------------------- MODULE MetadataUniqueness -----------------------
EXTENDS MetadataAllocation, TLAPS

\* Positive, strictly increasing write IDs and absent names strengthen uniqueness to an inductive
\* invariant.
OrderedIds ==
    /\ \A i \in WritesThrough(Len(log)) : log[i].id \in Nat \ {0}
    /\ \A i, j \in WritesThrough(Len(log)) : i < j => log[i].id < log[j].id
UniqueNames ==
    \A i, j \in WritesThrough(Len(log)) : i # j =>
        RequestName[log[i].request] # RequestName[log[j].request]
UniqueCatalog == OrderedIds /\ UniqueNames

THEOREM FreshId ==
    ASSUME Shape, OrderedIds
    PROVE /\ NextId(Len(log)) \in Nat \ {0}
          /\ \A i \in WritesThrough(Len(log)) : log[i].id < NextId(Len(log))
<1>1. CASE WritesThrough(Len(log)) = {}
    BY <1>1, SMT DEF NextId
<1>2. CASE WritesThrough(Len(log)) # {}
    <2>1. /\ LastWrite(Len(log)) \in WritesThrough(Len(log))
           /\ \A j \in WritesThrough(Len(log)) : j <= LastWrite(Len(log))
        BY <1>2, LastWriteMaximum, SMT DEF Shape, LogSeq, Bounds
    <2> QED BY <1>2, <2>1, SMT DEF OrderedIds, NextId
<1> QED BY <1>1, <1>2

THEOREM UniqueInit == Init => UniqueCatalog
BY SMT DEF Init, UniqueCatalog, OrderedIds, UniqueNames, WritesThrough

THEOREM UniqueStep ==
    ASSUME PlanningInvariant, UniqueCatalog, Next
    PROVE UniqueCatalog'
<1>0. /\ NextId(Len(log)) \in Nat \ {0}
       /\ \A i \in WritesThrough(Len(log)) : log[i].id < NextId(Len(log))
    BY FreshId, SMT DEF PlanningInvariant, Invariant, UniqueCatalog
<1>1. \A r \in Requests : Begin(r) => UniqueCatalog'
    BY SMT DEF Begin, UniqueCatalog, OrderedIds, UniqueNames, WritesThrough, Entry, PlanningInvariant,
       Invariant, Shape, LogSeq, Bounds
<1>2. \A r \in Requests : Submit(r) => UniqueCatalog'
    BY <1>0, SMT DEF Submit, UniqueCatalog, OrderedIds, UniqueNames, WritesThrough, Entry,
       PlanningInvariant, FreshPlans, NamesThrough, Invariant, Shape, LogSeq, Bounds
<1>3. \A n \in Nodes, keep \in committed..Len(log) : Elect(n, keep) => UniqueCatalog'
    <2>1. SUFFICES ASSUME NEW n \in Nodes, NEW keep \in committed..Len(log), Elect(n, keep)
                  PROVE UniqueCatalog'
        OBVIOUS
    <2>2. Shape' BY <2>1, ShapeStep, SMT DEF PlanningInvariant, Invariant
    <2>3. \A i \in (WritesThrough(Len(log)))' :
              /\ i \in WritesThrough(Len(log))
              /\ log'[i] = log[i]
        BY <2>1, <2>2, SMT DEF Elect, WritesThrough, Entry, PlanningInvariant, Invariant, Shape, LogSeq,
           Bounds
    <2> QED BY <2>3, SMT DEF UniqueCatalog, OrderedIds, UniqueNames
<1>4. (\E r \in Requests : Barrier(r) \/ Plan(r) \/ Finish(r) \/ Timeout(r)) \/ Commit \/ (\E n \in Nodes : Apply(n)) => UniqueCatalog'
    BY SMT DEF Barrier, Plan, Finish, Timeout, Commit, Apply, UniqueCatalog, OrderedIds, UniqueNames,
       WritesThrough
<1> QED BY <1>1, <1>2, <1>3, <1>4, SMT DEF Next, ElectAny

CatalogInvariant == PlanningInvariant /\ UniqueCatalog
THEOREM CatalogInvariantInit == Init => CatalogInvariant
BY PlanningInvariantInit, UniqueInit DEF CatalogInvariant

THEOREM CatalogInvariantStep == CatalogInvariant /\ [Next]_vars => CatalogInvariant'
BY PlanningInvariantStep, UniqueStep, SMT
   DEF CatalogInvariant, UniqueCatalog, OrderedIds, UniqueNames, WritesThrough, vars

THEOREM UniqueImpliesSafety ==
    ASSUME Shape, UniqueCatalog
    PROVE LogSafety /\ CatalogSafety
<1>1. \A i, j \in WritesThrough(Len(log)) : i # j => i < j \/ j < i
    BY SMT DEF WritesThrough
<1>2. LogSafety
    BY <1>1, SMT DEF UniqueCatalog, OrderedIds, UniqueNames, LogSafety, UniqueThrough
<1>3. WritesThrough(committed) \subseteq WritesThrough(Len(log))
    BY SMT DEF Shape, LogSeq, Bounds, WritesThrough
<1> QED BY <1>2, <1>3, SMT DEF LogSafety, CatalogSafety, UniqueThrough

THEOREM CatalogAlways == Spec => [](LogSafety /\ CatalogSafety)
<1>1. Spec => []CatalogInvariant BY CatalogInvariantInit, CatalogInvariantStep, PTL DEF Spec
<1>2. CatalogInvariant => LogSafety /\ CatalogSafety
    BY UniqueImpliesSafety, SMT DEF CatalogInvariant, PlanningInvariant, Invariant
<1> QED BY <1>1, <1>2, PTL
=============================================================================
