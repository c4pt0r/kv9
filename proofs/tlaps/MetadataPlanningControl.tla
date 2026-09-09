----------------------- MODULE MetadataPlanningControl -----------------------
EXTENDS MetadataReceipt, TLAPS

\* The local mutex and monotonically increasing terms identify the sole current-term planner.
TermBounds == planningTerm \in [Requests -> 0..term]
Mutex ==
    \A r, s \in Requests :
        (phase[r] \in Active /\ phase[s] \in Active /\ host[r] = host[s]) => r = s
CurrentOwner ==
    \A r \in Requests : (phase[r] \in Active /\ planningTerm[r] = term) => host[r] = leader
PlanningControl == TermBounds /\ Mutex /\ CurrentOwner

THEOREM PlanningControlInit == Init => PlanningControl
BY SMT DEF Init, PlanningControl, TermBounds, Mutex, CurrentOwner, Active

THEOREM PlanningControlStep == ASSUME Invariant, PlanningControl, Next PROVE PlanningControl'
<1>1. \A r \in Requests : Begin(r) => PlanningControl'
    BY SMT DEF Begin, PlanningControl, TermBounds, Mutex, CurrentOwner, Active, Invariant, Control
<1>2. \A r \in Requests : Barrier(r) => PlanningControl'
    BY SMT DEF Barrier, PlanningControl, TermBounds, Mutex, CurrentOwner, Active, Invariant, Control
<1>3. \A r \in Requests : Plan(r) => PlanningControl'
    BY SMT DEF Plan, PlanningControl, TermBounds, Mutex, CurrentOwner, Active, Invariant, Control
<1>4. \A r \in Requests : Submit(r) => PlanningControl'
    BY SMT DEF Submit, PlanningControl, TermBounds, Mutex, CurrentOwner, Active, Invariant, Control
<1>5. \A r \in Requests : Finish(r) => PlanningControl'
    BY SMT DEF Finish, PlanningControl, TermBounds, Mutex, CurrentOwner, Active, Invariant, Control
<1>6. \A r \in Requests : Timeout(r) => PlanningControl'
    BY SMT DEF Timeout, PlanningControl, TermBounds, Mutex, CurrentOwner, Active, Invariant, Control
<1>7. \A n \in Nodes, keep \in committed..Len(log) : Elect(n, keep) => PlanningControl'
    BY SMT DEF Elect, PlanningControl, TermBounds, Mutex, CurrentOwner, Active, Invariant, Control
<1>8. Commit \/ (\E n \in Nodes : Apply(n)) => PlanningControl'
    BY SMT DEF Commit, Apply, PlanningControl, TermBounds, Mutex, CurrentOwner
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, SMT DEF Next, ElectAny
=============================================================================
