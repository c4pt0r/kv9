---------------------------- MODULE Collections ----------------------------
EXTENDS Naturals, Sequences, TLAPS
Range(s) == {s[i] : i \in DOMAIN s}

THEOREM SeqRange ==
  ASSUME NEW CONSTANT V, NEW CONSTANT s \in Seq(V)
  PROVE s \in Seq(Range(s))
BY Isa DEF Range

THEOREM AppendRange ==
  ASSUME NEW CONSTANT V, NEW CONSTANT s \in Seq(V), NEW CONSTANT e
  PROVE Append(s,e) \in Seq(Range(Append(s,e)))
<1>1. Append(s,e) \in Seq(V \cup {e}) BY SMT
<1> QED BY <1>1, SeqRange

THEOREM SubRange ==
  ASSUME NEW CONSTANT V, NEW CONSTANT s \in Seq(V), NEW CONSTANT keep \in 0..Len(s)
  PROVE SubSeq(s,1,keep) \in Seq(Range(SubSeq(s,1,keep)))
<1>1. SubSeq(s,1,keep) \in Seq(V) BY SMT
<1> QED BY <1>1, SeqRange

THEOREM FunctionRange ==
  ASSUME NEW CONSTANT D, NEW CONSTANT V, NEW CONSTANT f \in [D -> V]
  PROVE f \in [D -> Range(f)]
BY Isa DEF Range

THEOREM UpdatedRange ==
  ASSUME NEW CONSTANT D, NEW CONSTANT V, NEW CONSTANT f \in [D -> V],
         NEW CONSTANT r \in D, NEW CONSTANT value
  PROVE [f EXCEPT ![r] = value] \in [D -> Range([f EXCEPT ![r] = value])]
<1>1. [f EXCEPT ![r] = value] \in [D -> V \cup {value}] BY SMT
<1> QED BY <1>1, FunctionRange
=============================================================================
