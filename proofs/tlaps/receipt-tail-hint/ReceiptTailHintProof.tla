----------------------- MODULE ReceiptTailHintProof -----------------------
EXTENDS ReceiptTailHint, AppliedReceiptsProof

THEOREM RTHValidatedMember ==
    ASSUME NEW s \in Seq(ARReceipts), NEW key \in ARIndexes, RTHValid(s, key)
    PROVE RTHPosition(s, key) \in ARMatches(s, key)
BY SMT DEF RTHValid, ARMatches

THEOREM RTHOrderedEquivalent ==
    ASSUME NEW s \in Seq(ARReceipts), ARStrict(s), NEW key \in ARIndexes
    PROVE RTHOrdered(s, key) = ARFirst(s, key)
<1>1. CASE ~RTHValid(s, key)
    BY <1>1, ARUniqueLookup DEF RTHOrdered
<1>2. CASE RTHValid(s, key)
    <2>1. RTHPosition(s, key) \in ARMatches(s, key)
        BY <1>2, RTHValidatedMember
    <2>2. \A j \in ARMatches(s, key) : j = RTHPosition(s, key)
        BY <2>1, ARStrictUniqueMatch
    <2>3. (CHOOSE j \in ARMatches(s, key) : TRUE) = RTHPosition(s, key)
        BY <2>1, <2>2, SMT DEF ARMatches
    <2>4. RTHOrdered(s, key) = ARAny(s, key)
        BY <1>2, <2>1, <2>3, SMT DEF RTHOrdered, ARAny
    <2> QED BY <2>4, ARUniqueLookup
<1> QED BY <1>1, <1>2

THEOREM RTHLookupObservation == ARInvariant => RTHRefinement
BY RTHOrderedEquivalent, SMT DEF ARInvariant, ARType, ARRefinement, AROrdering,
    RTHRefinement, RTHLookup

THEOREM RTHLookupAlways == ARSpec => []RTHRefinement
BY ARInvariantAlways, RTHLookupObservation, PTL

(* For a contiguous retained suffix starting at any position p, the calculated
   hint selects that position exactly. Correctness above does not need this
   density premise: a wrong or unrepresentable hint falls back to search. *)
THEOREM RTHDenseSuffixPosition ==
    ASSUME NEW s \in Seq(ARReceipts), NEW p \in 1..Len(s),
        s[Len(s)].index - s[p].index = Len(s) - p
    PROVE RTHPosition(s, s[p].index) = p
BY ARInputAssumption, SMT DEF RTHPosition, ARLegalInputs, ARReceipts

=============================================================================
