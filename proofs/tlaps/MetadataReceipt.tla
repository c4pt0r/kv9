---------------------------- MODULE MetadataReceipt ----------------------------
EXTENDS MetadataShape, MetadataPrefix, TLAPS

Control ==
    /\ term \in Nat
    /\ plannedId \in [Requests -> Range(plannedId)]
    /\ writeAt \in [Requests -> Nat]
    /\ writeTerm \in [Requests -> Nat]
    /\ barrierAt \in [Requests -> Nat]
    /\ planningTerm \in [Requests -> Nat]
    /\ leader \in Nodes
    /\ phase \in [Requests -> Active \cup {"new", "done"}]
    /\ host \in [Requests -> Nodes \cup {0}]
    /\ \A r \in Requests : phase[r] # "new" => host[r] \in Nodes
    /\ succeeded \subseteq Requests
    /\ \A r \in succeeded : phase[r] = "done"

Written ==
    \A i \in WritesThrough(Len(log)) :
        LET r == log[i].request IN
        /\ r \in Requests
        /\ phase[r] \in {"write", "done"}
        /\ writeAt[r] = i
        /\ writeTerm[r] = log[i].epoch
        /\ plannedId[r] = log[i].id

Invariant == Shape /\ Control /\ Written /\ ReceiptSafety

THEOREM ControlInit == Init => Control
BY FunctionRange, SMT DEF Init, Control, Active, Range

THEOREM ControlStep == ASSUME Shape, Control, Next PROVE Control'
<1>1. \A r \in Requests : Begin(r) => Control'
    BY SMT DEF Begin, Control, Active, Shape, LogSeq, Range
<1>2. \A r \in Requests : Barrier(r) => Control'
    BY SMT DEF Barrier, Control, Active, Shape, LogSeq, Range
<1>3. \A r \in Requests : Plan(r) => Control'
    BY UpdatedRange, SMT DEF Plan, Control, Active, Shape, LogSeq, Range
<1>4. \A r \in Requests : Submit(r) => Control'
    BY SMT DEF Submit, Control, Active, Shape, LogSeq, Range
<1>5. \A r \in Requests : Finish(r) => Control'
    BY SMT DEF Finish, Control, Active, Shape, LogSeq, Range
<1>6. \A r \in Requests : Timeout(r) => Control'
    BY SMT DEF Timeout, Control, Active, Shape, LogSeq, Range
<1>7. \A n \in Nodes, keep \in committed..Len(log) : Elect(n, keep) => Control'
    BY SMT DEF Elect, Control
<1>8. Commit \/ (\E n \in Nodes : Apply(n)) => Control'
    BY SMT DEF Commit, Apply, Control
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, SMT DEF Next, ElectAny

THEOREM WrittenInit == Init => Written
BY SMT DEF Init, Written, WritesThrough

THEOREM WrittenStep == ASSUME Shape, Control, Written, Next PROVE Written'
<1>1. \A r \in Requests : Begin(r) => Written'
    BY SMT DEF Begin, Written, WritesThrough, Shape, LogSeq, Range, Entry, Control
<1>2. \A r \in Requests : Submit(r) => Written'
    BY SMT DEF Submit, Written, WritesThrough, Shape, LogSeq, Range, Entry, Control
<1>3. \A n \in Nodes, keep \in committed..Len(log) : Elect(n, keep) => Written'
    BY SMT DEF Elect, Written, WritesThrough, Shape, LogSeq, Range, Bounds, Entry
<1>4. \A r \in Requests : Plan(r) => Written'
    BY SMT DEF Plan, Written, WritesThrough, Control
<1>5. \A r \in Requests : (Barrier(r) \/ Finish(r) \/ Timeout(r)) => Written'
    BY SMT DEF Barrier, Finish, Timeout, Written, WritesThrough, Active, Control
<1>6. Commit \/ (\E n \in Nodes : Apply(n)) => Written'
    BY SMT DEF Commit, Apply, Written, WritesThrough
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, SMT DEF Next, ElectAny

THEOREM ReceiptInit == Init => ReceiptSafety
BY SMT DEF Init, ReceiptSafety

THEOREM ReceiptStep == ASSUME Invariant, Next PROVE ReceiptSafety'
<1>0. Shape' BY ShapeStep, SMT DEF Invariant
<1>1. PrefixStable BY PrefixStep, SMT DEF Invariant, Shape, LogSeq, Bounds
<1>2. \A r \in Requests : Finish(r) => ReceiptSafety'
    BY <1>0, <1>1, SMT DEF Shape, Bounds, LogSeq, Range, Invariant, Shape, Bounds, Control, Written, WritesThrough,
                         Finish, ReceiptSafety, Exact, PrefixStable
<1>3. \A r \in Requests : (Begin(r) \/ Submit(r) \/ Plan(r)) => ReceiptSafety'
    BY <1>0, <1>1, SMT DEF Shape, Bounds, LogSeq, Range, Invariant, Control, Begin, Submit, Plan, ReceiptSafety, Exact, PrefixStable
<1>4. \A r \in Requests : (Barrier(r) \/ Timeout(r)) => ReceiptSafety'
    BY <1>0, <1>1, SMT DEF Shape, Bounds, LogSeq, Range, Invariant, Barrier, Timeout, ReceiptSafety, Exact, PrefixStable
<1>5. \A n \in Nodes, keep \in committed..Len(log) : Elect(n, keep) => ReceiptSafety'
    BY <1>0, <1>1, SMT DEF Shape, Bounds, LogSeq, Range, Invariant, Elect, ReceiptSafety, Exact, PrefixStable
<1>6. Commit \/ (\E n \in Nodes : Apply(n)) => ReceiptSafety'
    BY <1>0, <1>1, SMT DEF Shape, Bounds, LogSeq, Range, Invariant, Commit, Apply, ReceiptSafety, Exact, PrefixStable
<1> QED BY <1>2, <1>3, <1>4, <1>5, <1>6, SMT DEF Next, ElectAny

THEOREM InvariantInit == Init => Invariant
BY ShapeInit, ControlInit, WrittenInit, ReceiptInit, SMT DEF Invariant

THEOREM InvariantStep == Invariant /\ [Next]_vars => Invariant'
BY ShapeStep, ControlStep, WrittenStep, ReceiptStep, SMT
   DEF Invariant, Shape, LogSeq, Bounds, Control, Written, ReceiptSafety,
       Exact, WritesThrough, Range, vars

THEOREM ReceiptAlways == Spec => []ReceiptSafety
<1>1. Spec => []Invariant BY InvariantInit, InvariantStep, PTL DEF Spec
<1>2. Invariant => ReceiptSafety BY SMT DEF Invariant
<1> QED BY <1>1, <1>2, PTL
=============================================================================
