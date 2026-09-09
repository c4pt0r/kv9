----------------------- MODULE MetadataPlanningBarrier -----------------------
EXTENDS MetadataPlanningControl, TLAPS

\* A current-term barrier has no later write before submission. An observed exact barrier is
\* retained after elections.
BeforeSubmit == {"barrier", "plan", "ready"}
AfterBarrier == {"plan", "ready", "write"}
BarrierTail ==
    \A r \in Requests :
        (phase[r] \in BeforeSubmit /\ planningTerm[r] = term) =>
        /\ Exact(barrierAt[r], "barrier", r, planningTerm[r])
        /\ \A i \in (barrierAt[r] + 1)..Len(log) : log[i].kind # "write"
SeenBarrier ==
    \A r \in Requests : phase[r] \in AfterBarrier =>
        /\ Exact(barrierAt[r], "barrier", r, planningTerm[r])
        /\ barrierAt[r] <= applied[host[r]]
Barriers == BarrierTail /\ SeenBarrier

THEOREM BarriersInit == Init => Barriers
BY SMT DEF Init, Barriers, BarrierTail, SeenBarrier, BeforeSubmit, AfterBarrier

THEOREM BarriersStep == ASSUME Invariant, PlanningControl, Barriers, Next PROVE Barriers'
<1>0. Shape' BY ShapeStep, SMT DEF Invariant
<1>1. PrefixStable BY PrefixStep, SMT DEF Invariant, Shape, LogSeq, Bounds
<1>2. \A r \in Requests : Begin(r) => Barriers'
    BY <1>0, <1>1, SMT DEF Begin, Barriers, BarrierTail, SeenBarrier, BeforeSubmit, AfterBarrier,
       PlanningControl, TermBounds, Mutex, CurrentOwner, Active, Invariant, Shape, LogSeq, Range, Bounds,
       Control, Exact, Entry, PrefixStable
<1>3. \A r \in Requests : Barrier(r) => Barriers'
    BY <1>0, <1>1, SMT DEF Barrier, Barriers, BarrierTail, SeenBarrier, BeforeSubmit, AfterBarrier,
       PlanningControl, TermBounds, Mutex, CurrentOwner, Active, Invariant, Shape, LogSeq, Range, Bounds,
       Control, Exact, Entry, PrefixStable
<1>4. \A r \in Requests : Plan(r) => Barriers'
    BY <1>0, <1>1, SMT DEF Plan, Barriers, BarrierTail, SeenBarrier, BeforeSubmit, AfterBarrier,
       PlanningControl, TermBounds, Mutex, CurrentOwner, Active, Invariant, Shape, LogSeq, Range, Bounds,
       Control, Exact, Entry, PrefixStable
<1>5. \A r \in Requests : Submit(r) => Barriers'
    BY <1>0, <1>1, SMT DEF Submit, Barriers, BarrierTail, SeenBarrier, BeforeSubmit, AfterBarrier,
       PlanningControl, TermBounds, Mutex, CurrentOwner, Active, Invariant, Shape, LogSeq, Range, Bounds,
       Control, Exact, Entry, PrefixStable
<1>6. \A r \in Requests : Finish(r) => Barriers'
    BY <1>0, <1>1, SMT DEF Finish, Barriers, BarrierTail, SeenBarrier, BeforeSubmit, AfterBarrier,
       PlanningControl, TermBounds, Mutex, CurrentOwner, Active, Invariant, Shape, LogSeq, Range, Bounds,
       Control, Exact, Entry, PrefixStable
<1>7. \A r \in Requests : Timeout(r) => Barriers'
    BY <1>0, <1>1, SMT DEF Timeout, Barriers, BarrierTail, SeenBarrier, BeforeSubmit, AfterBarrier,
       PlanningControl, TermBounds, Mutex, CurrentOwner, Active, Invariant, Shape, LogSeq, Range, Bounds,
       Control, Exact, Entry, PrefixStable
<1>8. \A n \in Nodes, keep \in committed..Len(log) : Elect(n, keep) => Barriers'
    BY <1>0, <1>1, SMT DEF Elect, Barriers, BarrierTail, SeenBarrier, BeforeSubmit, AfterBarrier,
       PlanningControl, TermBounds, Mutex, CurrentOwner, Active, Invariant, Shape, LogSeq, Range, Bounds,
       Control, Exact, Entry, PrefixStable
<1>9. Commit \/ (\E n \in Nodes : Apply(n)) => Barriers'
    BY <1>0, <1>1, SMT DEF Commit, Apply, Barriers, BarrierTail, SeenBarrier, BeforeSubmit, AfterBarrier,
       PlanningControl, TermBounds, Mutex, CurrentOwner, Active, Invariant, Shape, LogSeq, Range, Bounds,
       Control, Exact, Entry, PrefixStable
<1> QED BY <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, SMT DEF Next, ElectAny
=============================================================================
