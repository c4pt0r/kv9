------------------------ MODULE MetadataAllocation ------------------------
EXTENDS MetadataFreshness, TLAPS

\* Prove the maximum used by CHOOSE exists; do not assume the unproved declarations in the standard
\* induction modules.
THEOREM NaturalInduction ==
    ASSUME NEW P(_), P(0), \A n \in Nat : P(n) => P(n + 1)
    PROVE \A n \in Nat : P(n)
BY IsaM("(intro natInduct, auto)")

THEOREM BoundedMaximum ==
    ASSUME NEW CONSTANT bound \in Nat, NEW CONSTANT S,
           S \subseteq 1..bound, S # {}
    PROVE \E m \in S : \A j \in S : j <= m
<1>1. DEFINE HasMax(n) == \A T : (T \subseteq 1..n /\ T # {}) =>
                       \E m \in T : \A j \in T : j <= m
<1>2. HasMax(0) BY SMT DEF HasMax
<1>3. \A n \in Nat : HasMax(n) => HasMax(n + 1)
    <2>1. SUFFICES ASSUME NEW n \in Nat, HasMax(n), NEW T,
                         T \subseteq 1..(n + 1), T # {}
                  PROVE \E m \in T : \A j \in T : j <= m
        BY DEF HasMax
    <2>2. CASE n + 1 \in T
        BY <2>1, <2>2, SMT
    <2>3. CASE n + 1 \notin T
        <3>1. T \subseteq 1..n BY <2>1, <2>3, SMT
        <3> QED BY <2>1, <3>1, Isa DEF HasMax
    <2> QED BY <2>2, <2>3
<1>4. \A n \in Nat : HasMax(n) BY <1>2, <1>3, NaturalInduction, Isa
<1> QED BY <1>4 DEF HasMax

THEOREM LastWriteMaximum ==
    ASSUME Shape, NEW CONSTANT cut \in 0..Len(log), WritesThrough(cut) # {}
    PROVE /\ LastWrite(cut) \in WritesThrough(cut)
          /\ \A j \in WritesThrough(cut) : j <= LastWrite(cut)
<1>0. /\ cut \in Nat
       /\ WritesThrough(cut) \subseteq 1..cut
    BY SMT DEF Shape, LogSeq, Bounds, WritesThrough
<1>1. \E m \in WritesThrough(cut) : \A j \in WritesThrough(cut) : j <= m
    BY <1>0, BoundedMaximum, SMT
<1> QED BY <1>1 DEF LastWrite
=============================================================================
