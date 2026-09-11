----------------------- MODULE AppliedReceiptsProof -----------------------
EXTENDS AppliedReceipts, TLAPS

THEOREM ARPushSequenceType ==
    ASSUME NEW s \in Seq(ARReceipts), Len(s) <= ARCapacity,
        NEW e \in ARReceipts
    PROVE ARVectorPush(s, e) \in Seq(ARReceipts) /\
        ARDequePush(s, e) \in Seq(ARReceipts) /\
        Len(ARVectorPush(s, e)) <= ARCapacity /\
        Len(ARDequePush(s, e)) <= ARCapacity
BY ARInputAssumption, SMT DEF ARLegalInputs, ARVectorPush, ARDequePush

THEOREM ARPushSequenceEquality ==
    ASSUME NEW s \in Seq(ARReceipts), Len(s) <= ARCapacity,
        NEW e \in ARReceipts
    PROVE ARVectorPush(s, e) = ARDequePush(s, e)
BY ARInputAssumption, SMT DEF ARLegalInputs, ARVectorPush, ARDequePush

THEOREM ARStrictTail ==
    ASSUME NEW s \in Seq(ARReceipts), Len(s) > 0, ARStrict(s)
    PROVE ARStrict(Tail(s))
BY SMT DEF ARStrict

THEOREM ARStrictAppend ==
    ASSUME NEW s \in Seq(ARReceipts), ARStrict(s), NEW e \in ARReceipts,
        Len(s) = 0 \/ s[Len(s)].index < e.index
    PROVE ARStrict(Append(s, e))
<1>1. SUFFICES ASSUME NEW i \in 1..Len(Append(s, e)),
        NEW j \in 1..Len(Append(s, e)), i < j
    PROVE Append(s, e)[i].index < Append(s, e)[j].index
    BY DEF ARStrict
<1>2. i \in 1..Len(s) /\ j \in 1..(Len(s) + 1) /\
    Append(s, e)[i] = s[i]
    BY <1>1, SMT
<1>3. CASE j <= Len(s)
    BY <1>1, <1>2, <1>3, SMT DEF ARStrict
<1>4. CASE j = Len(s) + 1
    <2>1. Len(s) > 0 /\ s[i].index \in Nat /\
        s[Len(s)].index \in Nat /\ e.index \in Nat
        BY <1>2, ARInputAssumption, SMT DEF ARLegalInputs, ARReceipts
    <2>2. s[i].index <= s[Len(s)].index
        BY <1>2, <2>1, SMT DEF ARStrict
    <2>3. s[Len(s)].index < e.index
        BY <2>1, SMT
    <2> QED BY <1>1, <1>2, <1>4, <2>1, <2>2, <2>3, SMT
<1> QED BY <1>1, <1>2, <1>3, <1>4, SMT

THEOREM ARCertificatePreservesOrdering ==
    ASSUME NEW s \in Seq(ARReceipts), Len(s) <= ARCapacity,
        NEW c \in BOOLEAN, c => ARStrict(s), NEW e \in ARReceipts
    PROVE ARCertificate(s, c, e) => ARStrict(ARDequePush(s, e))
<1>1. SUFFICES ASSUME ARCertificate(s, c, e)
    PROVE ARStrict(ARDequePush(s, e))
    BY SMT
<1>2. ARStrict(s) /\ (Len(s) = 0 \/ s[Len(s)].index < e.index)
    BY <1>1, SMT DEF ARCertificate
<1>3. CASE Len(s) < ARCapacity
    BY <1>2, <1>3, ARStrictAppend, SMT DEF ARDequePush
<1>4. CASE Len(s) = ARCapacity
    <2>1. Len(s) > 0 /\ Tail(s) \in Seq(ARReceipts) /\
        Len(Tail(s)) = Len(s) - 1 /\
        (Len(Tail(s)) > 0 => Tail(s)[Len(Tail(s))] = s[Len(s)])
        BY <1>4, ARInputAssumption, SMT DEF ARLegalInputs
    <2>2. ARStrict(Tail(s)) BY <1>2, <2>1, ARStrictTail
    <2>3. Len(Tail(s)) = 0 \/ Tail(s)[Len(Tail(s))].index < e.index
        BY <1>2, <2>1, SMT
    <2> QED BY <1>4, <2>1, <2>2, <2>3, ARStrictAppend, SMT DEF ARDequePush
<1> QED BY <1>3, <1>4, SMT

THEOREM ARStrictUniqueMatch ==
    ASSUME NEW s \in Seq(ARReceipts), ARStrict(s), NEW key \in ARIndexes,
        NEW i \in ARMatches(s, key), NEW j \in ARMatches(s, key)
    PROVE i = j
BY ARInputAssumption, SMT DEF ARStrict, ARMatches, ARLegalInputs, ARReceipts

THEOREM ARUniqueLookup ==
    ASSUME NEW s \in Seq(ARReceipts), ARStrict(s), NEW key \in ARIndexes
    PROVE ARAny(s, key) = ARFirst(s, key)
<1>1. CASE ARMatches(s, key) = {}
    BY <1>1 DEF ARAny, ARFirst
<1>2. CASE ARMatches(s, key) # {}
    <2>1. PICK i \in ARMatches(s, key) : TRUE
        BY <1>2
    <2>2. \A j \in ARMatches(s, key) : j = i
        BY <2>1, ARStrictUniqueMatch
    <2>3. (CHOOSE j \in ARMatches(s, key) : TRUE) = i
        BY <2>1, <2>2, SMT DEF ARMatches
    <2>4. (CHOOSE j \in ARMatches(s, key) : \A k \in ARMatches(s, key) : j <= k) = i
        BY <2>1, <2>2, SMT DEF ARMatches
    <2> QED BY <1>2, <2>3, <2>4 DEF ARAny, ARFirst
<1> QED BY <1>1, <1>2

THEOREM ARInvariantInit == ARInit => ARInvariant
BY ARInputAssumption, SMT DEF ARInit, ARInvariant, ARType,
    ARRefinement, AROrdering, ARStrict, ARLegalInputs

THEOREM ARInvariantPush ==
    ASSUME ARInvariant, NEW e \in ARReceipts, ARPush(e)
    PROVE ARInvariant'
BY ARPushSequenceType, ARPushSequenceEquality, ARCertificatePreservesOrdering,
    SMT DEF ARInvariant, ARType, ARRefinement, AROrdering, ARPush, ARCertificate

THEOREM ARInvariantStep == ASSUME ARInvariant, [ARNext]_arVars PROVE ARInvariant'
BY ARInvariantPush, SMT DEF ARNext, arVars, ARInvariant, ARType, ARRefinement, AROrdering

THEOREM ARInvariantAlways == ARSpec => []ARInvariant
BY ARInvariantInit, ARInvariantStep, PTL DEF ARSpec

THEOREM ARLookupObservation == ARInvariant => ARLookupRefinement
BY ARUniqueLookup, SMT DEF ARInvariant, ARType, ARRefinement, AROrdering,
    ARLookupRefinement, ARLookup

THEOREM ARLookupAlways == ARSpec => []ARLookupRefinement
BY ARInvariantAlways, ARLookupObservation, PTL

=============================================================================
