---------------------------- MODULE MetadataShape ----------------------------
EXTENDS MetadataPlanning, Collections, TLAPS

LogSeq == log \in Seq(Range(log))
Bounds == /\ committed \in 0..Len(log)
          /\ applied \in [Nodes -> 0..committed]
Shape == LogSeq /\ Bounds

THEOREM ShapeInit == Init => Shape
BY SMT DEF Init, Shape, LogSeq, Range, Bounds

THEOREM ShapeStep == ASSUME Shape, Next PROVE Shape'
<1>1. \A r \in Requests : Begin(r) => Shape'
    BY AppendRange, SMT DEF Shape, LogSeq, Range, Bounds, Begin
<1>2. \A r \in Requests : Submit(r) => Shape'
    BY AppendRange, SMT DEF Shape, LogSeq, Range, Bounds, Submit
<1>3. \A n \in Nodes, keep \in committed..Len(log) : Elect(n, keep) => Shape'
    BY AppendRange, SubRange, SMT DEF Shape, LogSeq, Range, Bounds, Elect
<1>4. Commit => Shape'
    BY SMT DEF Shape, LogSeq, Range, Bounds, Commit
<1>5. \A n \in Nodes : Apply(n) => Shape'
    BY SMT DEF Shape, LogSeq, Range, Bounds, Apply
<1>6. \A r \in Requests :
          (Barrier(r) \/ Plan(r) \/ Finish(r) \/ Timeout(r)) => Shape'
    BY SMT DEF Shape, LogSeq, Range, Bounds, Barrier, Plan, Finish, Timeout
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, SMT DEF Next, ElectAny

THEOREM ShapeAlways == Spec => []Shape
<1>1. Init => Shape BY ShapeInit
<1>2. Shape /\ [Next]_vars => Shape'
    BY ShapeStep, SMT DEF Shape, LogSeq, Range, Bounds, vars
<1> QED BY <1>1, <1>2, PTL DEF Spec
=============================================================================
