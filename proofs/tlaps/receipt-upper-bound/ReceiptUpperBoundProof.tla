----------------------- MODULE ReceiptUpperBoundProof -----------------------
EXTENDS ReceiptUpperBound, TLAPS

THEOREM UBSequenceType ==
    ASSUME NEW s \in Seq(UBReceipts), Len(s) <= UBCapacity, NEW e \in UBReceipts
    PROVE UBPushSequence(s, e) \in Seq(UBReceipts) /\ Len(UBPushSequence(s, e)) <= UBCapacity
BY UBInputAssumption, SMT DEF UBLegalInputs, UBPushSequence

THEOREM UBMaximumMonotonic ==
    ASSUME NEW b \in Nat, NEW e \in UBReceipts
    PROVE UBAdvance(b, e) \in Nat /\ UBAdvance(b, e) >= b /\ UBAdvance(b, e) >= e.index
BY UBInputAssumption, SMT DEF UBLegalInputs, UBReceipts, UBAdvance

THEOREM UBTailBound ==
    ASSUME NEW s \in Seq(UBReceipts), NEW b \in Nat, Len(s) > 0, UBBounded(s, b)
    PROVE UBBounded(Tail(s), b)
BY SMT DEF UBBounded

THEOREM UBAppendBound ==
    ASSUME NEW s \in Seq(UBReceipts), NEW b \in Nat, UBBounded(s, b), NEW e \in UBReceipts
    PROVE UBBounded(Append(s, e), UBAdvance(b, e))
<1>1. SUFFICES ASSUME NEW i \in 1..Len(Append(s, e))
    PROVE Append(s, e)[i].index <= UBAdvance(b, e)
    BY DEF UBBounded
<1>2. CASE i <= Len(s)
    <2>1. i \in 1..Len(s) /\ Append(s, e)[i] = s[i]
        BY <1>1, <1>2, SMT
    <2>2. s[i] \in UBReceipts /\ s[i].index \in Nat
        BY <2>1, UBInputAssumption, SMT DEF UBReceipts, UBLegalInputs
    <2>3. s[i].index <= b /\ b <= UBAdvance(b, e) /\ UBAdvance(b, e) \in Nat
        BY <2>1, UBMaximumMonotonic, SMT DEF UBBounded
    <2> QED BY <2>1, <2>2, <2>3, SMT
<1>3. CASE i = Len(s) + 1
    BY <1>1, <1>3, UBMaximumMonotonic, SMT
<1> QED BY <1>1, <1>2, <1>3, SMT

THEOREM UBPushBound ==
    ASSUME NEW s \in Seq(UBReceipts), Len(s) <= UBCapacity,
        NEW b \in Nat, UBBounded(s, b), NEW e \in UBReceipts
    PROVE UBBounded(UBPushSequence(s, e), UBAdvance(b, e))
BY UBAppendBound, UBTailBound, UBMaximumMonotonic, SMT DEF UBPushSequence

THEOREM UBImpossibleMatch ==
    ASSUME NEW s \in Seq(UBReceipts), NEW b \in Nat, UBBounded(s, b),
        NEW key \in Nat, key > b
    PROVE UBMatches(s, key) = {}
BY SMT DEF UBMatches, UBBounded

THEOREM UBLookupSame ==
    ASSUME NEW s \in Seq(UBReceipts), NEW b \in Nat, UBBounded(s, b), NEW key \in Nat
    PROVE UBLookup(s, b, key) = UBFirst(s, key)
<1>1. CASE key > b
    <2>1. UBMatches(s, key) = {}
        BY <1>1, UBImpossibleMatch
    <2>2. UBFirst(s, key) = [found |-> FALSE]
        BY <2>1 DEF UBFirst
    <2> QED BY <1>1, <2>2 DEF UBLookup
<1>2. CASE ~(key > b)
    BY <1>2 DEF UBLookup
<1> QED BY <1>1, <1>2

THEOREM UBContextRefinement ==
    ASSUME NEW s \in Seq(UBReceipts), NEW b \in Nat, UBBounded(s, b),
        NEW key \in Nat, NEW Observe(_)
    PROVE Observe(UBLookup(s, b, key)) = Observe(UBFirst(s, key))
BY UBLookupSame, SMT

THEOREM UBInvariantInit == UBInit => UBInvariant
BY UBInputAssumption, SMT DEF UBInit, UBInvariant, UBType, UBBounded, UBLegalInputs

THEOREM UBInvariantPush ==
    ASSUME UBInvariant, NEW e \in UBReceipts, UBPush(e)
    PROVE UBInvariant'
BY UBSequenceType, UBPushBound, UBMaximumMonotonic, SMT
    DEF UBInvariant, UBType, UBPush

THEOREM UBInvariantStep == ASSUME UBInvariant, [UBNext]_ubVars PROVE UBInvariant'
BY UBInvariantPush, SMT DEF UBNext, ubVars, UBInvariant, UBType, UBBounded

THEOREM UBInvariantAlways == UBSpec => []UBInvariant
BY UBInvariantInit, UBInvariantStep, PTL DEF UBSpec

THEOREM UBLookupObservation == UBInvariant => UBLookupRefinement
BY UBLookupSame, UBInputAssumption, SMT DEF UBLegalInputs, UBInvariant, UBType, UBLookupRefinement

THEOREM UBLookupAlways == UBSpec => []UBLookupRefinement
BY UBInvariantAlways, UBLookupObservation, PTL

=============================================================================
