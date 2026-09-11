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

THEOREM ARCertificatePreservesOrdering ==
    ASSUME NEW s \in Seq(ARReceipts), Len(s) <= ARCapacity,
        NEW c \in BOOLEAN, c => ARStrict(s), NEW e \in ARReceipts
    PROVE ARCertificate(s, c, e) => ARStrict(ARDequePush(s, e))
BY ARInputAssumption, SMT DEF ARLegalInputs, ARCertificate, ARStrict,
    ARDequePush, ARReceipts

THEOREM ARStrictUniqueMatch ==
    ASSUME NEW s \in Seq(ARReceipts), ARStrict(s), NEW key \in ARIndexes,
        NEW i \in ARMatches(s, key), NEW j \in ARMatches(s, key)
    PROVE i = j
BY SMT DEF ARStrict, ARMatches

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
        BY <2>1, <2>2, SMT
    <2>4. (CHOOSE j \in ARMatches(s, key) : \A k \in ARMatches(s, key) : j <= k) = i
        BY <2>1, <2>2, SMT
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
