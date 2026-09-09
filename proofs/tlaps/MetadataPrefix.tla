---------------------------- MODULE MetadataPrefix ----------------------------
EXTENDS MetadataShape, TLAPS

PrefixStable ==
    /\ committed' >= committed
    /\ \A i \in 1..committed : log'[i] = log[i]

THEOREM PrefixStep ==
    ASSUME NEW CONSTANT Values,
           log \in Seq(Values), committed \in 0..Len(log), Next
    PROVE PrefixStable
<1>1. \A r \in Requests : Begin(r) => PrefixStable
    BY SMT DEF Begin, PrefixStable
<1>2. \A r \in Requests : Submit(r) => PrefixStable
    BY SMT DEF Submit, PrefixStable
<1>3. \A n \in Nodes, keep \in committed..Len(log) : Elect(n, keep) => PrefixStable
    BY SMT DEF Elect, PrefixStable
<1>4. Commit => PrefixStable
    BY SMT DEF Commit, PrefixStable
<1>5. \A n \in Nodes : Apply(n) => PrefixStable
    BY SMT DEF Apply, PrefixStable
<1>6. \A r \in Requests :
          (Barrier(r) \/ Plan(r) \/ Finish(r) \/ Timeout(r)) => PrefixStable
    BY SMT DEF Barrier, Plan, Finish, Timeout, PrefixStable
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, SMT DEF Next, ElectAny

THEOREM PrefixAlways == Spec => [][PrefixStable]_vars
<1>1. Shape /\ [Next]_vars => [PrefixStable]_vars
    BY PrefixStep, SMT DEF Shape, LogSeq, Bounds, PrefixStable, vars
<1>2. Spec => []Shape BY ShapeAlways
<1> QED BY <1>1, <1>2, PTL DEF Spec
=============================================================================
