------------------------- MODULE MetadataFreshness -------------------------
EXTENDS MetadataPlanningBarrier, TLAPS

\* The applied barrier and planner isolation make the local planning snapshot equal to the complete
\* retained write cut.
THEOREM EqualWriteCuts ==
    ASSUME NEW CONSTANT a, NEW CONSTANT b,
           WritesThrough(a) = WritesThrough(b)
    PROVE /\ NamesThrough(a) = NamesThrough(b)
          /\ NextId(a) = NextId(b)
BY Isa DEF NamesThrough, NextId, LastWrite

THEOREM DrainedWriteCut ==
    ASSUME Shape, NEW CONSTANT a \in 0..Len(log), NEW CONSTANT b \in 0..a,
           \A i \in (b + 1)..Len(log) : log[i].kind # "write"
    PROVE WritesThrough(a) = WritesThrough(Len(log))
BY SMT DEF Shape, LogSeq, Bounds, WritesThrough

THEOREM PlannedSnapshot ==
    ASSUME Invariant, PlanningControl, Barriers, NEW CONSTANT r \in Requests,
           phase[r] = "plan", planningTerm[r] = term
    PROVE /\ NamesThrough(applied[host[r]]) = NamesThrough(Len(log))
          /\ NextId(applied[host[r]]) = NextId(Len(log))
<1>0. /\ applied[host[r]] \in 0..Len(log)
       /\ barrierAt[r] \in 0..applied[host[r]]
       /\ \A i \in (barrierAt[r] + 1)..Len(log) : log[i].kind # "write"
    BY SMT DEF Invariant, Shape, LogSeq, Bounds, Control, Barriers,
       BarrierTail, SeenBarrier, BeforeSubmit, AfterBarrier, Exact
<1>1. WritesThrough(applied[host[r]]) = WritesThrough(Len(log))
    BY <1>0, DrainedWriteCut, Isa DEF Invariant
<1> QED BY <1>1, EqualWriteCuts

THEOREM SnapshotFrame ==
    ASSUME UNCHANGED log
    PROVE \A cut : /\ (NextId(cut))' = NextId(cut)
                    /\ (NamesThrough(cut))' = NamesThrough(cut)
BY Isa DEF NextId, LastWrite, NamesThrough, WritesThrough

THEOREM FreshStep == ASSUME Invariant, PlanningControl, Barriers, FreshPlans, Next PROVE FreshPlans'
<1>1. \A r \in Requests : Begin(r) => FreshPlans'
    BY SMT DEF Begin, FreshPlans, Invariant, Control, PlanningControl, TermBounds, Mutex, CurrentOwner,
       Active
<1>2. \A r \in Requests : Barrier(r) => FreshPlans'
    BY SnapshotFrame, SMT DEF Barrier, FreshPlans, Invariant, Control, PlanningControl, TermBounds,
       Mutex, CurrentOwner, Active
<1>3. \A r \in Requests : Plan(r) => FreshPlans'
    BY SnapshotFrame, PlannedSnapshot, SMT DEF Plan, FreshPlans, Invariant, Control, PlanningControl,
       TermBounds, Mutex, CurrentOwner, Active
<1>4. \A r \in Requests : Submit(r) => FreshPlans'
    BY SMT DEF Submit, FreshPlans, Invariant, Control, PlanningControl, TermBounds, Mutex, CurrentOwner,
       Active
<1>5. \A r \in Requests : Finish(r) => FreshPlans'
    BY SnapshotFrame, SMT DEF Finish, FreshPlans, Invariant, Control, PlanningControl, TermBounds, Mutex,
       CurrentOwner, Active
<1>6. \A r \in Requests : Timeout(r) => FreshPlans'
    BY SnapshotFrame, SMT DEF Timeout, FreshPlans, Invariant, Control, PlanningControl, TermBounds,
       Mutex, CurrentOwner, Active
<1>7. \A n \in Nodes, keep \in committed..Len(log) : Elect(n, keep) => FreshPlans'
    BY SMT DEF Elect, FreshPlans, Invariant, Control, PlanningControl, TermBounds, Mutex, CurrentOwner,
       Active
<1>8. Commit \/ (\E n \in Nodes : Apply(n)) => FreshPlans'
    BY SnapshotFrame, SMT DEF Commit, Apply, FreshPlans
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, SMT DEF Next, ElectAny

PlanningInvariant == Invariant /\ PlanningControl /\ Barriers /\ FreshPlans
THEOREM PlanningInvariantInit == Init => PlanningInvariant
BY InvariantInit, PlanningControlInit, BarriersInit, SMT
   DEF Init, PlanningInvariant, FreshPlans

THEOREM PlanningInvariantStep == PlanningInvariant /\ [Next]_vars => PlanningInvariant'
BY InvariantStep, PlanningControlStep, BarriersStep, FreshStep, SnapshotFrame, SMT
   DEF PlanningInvariant, PlanningControl, TermBounds, Mutex, CurrentOwner,
       Barriers, BarrierTail, SeenBarrier, FreshPlans, Exact, vars

THEOREM FreshAlways == Spec => []FreshPlans
<1>1. Spec => []PlanningInvariant BY PlanningInvariantInit, PlanningInvariantStep, PTL DEF Spec
<1>2. PlanningInvariant => FreshPlans BY SMT DEF PlanningInvariant
<1> QED BY <1>1, <1>2, PTL
=============================================================================
