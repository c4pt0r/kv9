------------------------ MODULE ConfigurationCutProof ------------------------
EXTENDS ConfigurationCut, TLAPS
THEOREM HCInvariantInit == HCInit => HCInvariant
BY HCInputs, SMT DEF HCInit, HCInvariant, HCType, HCFound
THEOREM HCInvariantStep == ASSUME HCInvariant, [HCNext]_hcVars PROVE HCInvariant'
<1>1. \A i \in 1..HCLimit : HCStage(i) => HCInvariant'
    BY HCInputs, SMT DEF HCStage, HCInvariant, HCType, HCFound, hcView
<1>2. HCSync \/ HCPublish => HCInvariant'
    BY HCInputs, SMT DEF HCSync, HCPublish, HCInvariant, HCType, HCFound, hcView
<1>3. HCFail \/ HCRecover \/ HCAmbiguous => HCInvariant'
    BY HCInputs, SMT DEF HCFail, HCRecover, HCAmbiguous, HCInvariant, HCType, HCFound, hcView
<1>4. HCQueries => HCInvariant'
    BY HCInputs, SMT DEF HCQueries, HCQuery, HCInvariant, HCType, HCFound, HCLatest
<1>5. UNCHANGED hcVars => HCInvariant'
    BY SMT DEF hcVars, hcView, HCInvariant, HCType, HCFound
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, SMT DEF HCNext
THEOREM HCInvariantAlways == HCSpec => []HCInvariant
BY HCInvariantInit, HCInvariantStep, PTL DEF HCSpec
THEOREM HCFoundIsLatest == HCInvariant /\ hcResult = "Found" => HCLatest(hcCut, hcSelected)
BY DEF HCInvariant, HCFound
THEOREM HCFoundIsDurable == HCInvariant /\ hcResult = "Found" => hcSelected \in {0} \cup hcDurable
BY DEF HCInvariant, HCFound
THEOREM HCFencedLookup ==
    ASSUME NEW q \in 1..HCLimit, NEW s \in 0..HCLimit, NEW claim \in Nat,
           HCQuery(q, s, claim), ~hcWriter
    PROVE hcResult' = "Invalid"
BY SMT DEF HCQuery
THEOREM HCMissingConfiguration ==
    ASSUME NEW q \in 1..HCLimit, NEW s \in 0..HCLimit, NEW claim \in Nat,
           HCQuery(q, s, claim), \E i \in HCChanges : s < i /\ i <= q
    PROVE hcResult' # "Found"
BY SMT DEF HCQuery
THEOREM HCFutureExcluded ==
    ASSUME NEW q \in 1..HCLimit, NEW s \in 0..HCLimit, NEW claim \in Nat, HCQuery(q, s, claim)
    PROVE hcSelected' <= hcCut'
BY SMT DEF HCQuery
THEOREM HCCompleteHistorySucceeds ==
    ASSUME NEW q \in 1..HCLimit, NEW s \in 0..HCLimit, HCInvariant,
           HCQuery(q, s, HCTerms[q]), hcWriter, hcKnown,
           hcApplied \subseteq HCChanges,
           \A i \in HCChanges : i <= q => i \in hcApplied,
           \A i \in hcApplied : i <= q => i <= s,
           \A i \in 1..q : HCTerms[i] <= HCTerms[q]
    PROVE hcResult' = "Found"
BY HCInputs, SMT DEF HCQuery, HCInvariant, HCType
THEOREM HCQuerySettles ==
    ASSUME NEW q \in 1..HCLimit, NEW s \in 0..HCLimit, NEW claim \in Nat, HCQuery(q, s, claim)
    PROVE hcResult' # "Idle"
BY SMT DEF HCQuery
THEOREM HCStableInspectionProgress == HCInspectSpec => HCInspectProgress
<1>1. DEFINE P == hcPending = 0 /\ hcResult = "Idle"
            Q == hcResult # "Idle"
<1>2. HCInvariant /\ P /\ [HCQueries]_hcVars => HCInvariant' /\ (P' \/ Q')
    BY HCInvariantStep, SMT DEF P, Q, HCNext, HCQueries, HCQuery, hcVars, hcView
<1>3. HCInvariant /\ P /\ HCQueries => Q'
    BY SMT DEF P, Q, HCQueries, HCQuery
<1>4. HCInvariant /\ P => ENABLED <<HCQueries>>_hcVars
    <2>1. SUFFICES ASSUME HCInvariant, P PROVE ENABLED <<HCQueries>>_hcVars
        OBVIOUS
    <2>2. ENABLED HCQuery(1, 0, HCTerms[1])
        BY <2>1, HCInputs, ExpandENABLED, Isa DEF HCQuery, hcView, P
    <2>3. HCQuery(1, 0, HCTerms[1]) => HCQueries
        BY HCInputs, SMT DEF HCQueries
    <2>4. ENABLED HCQuery(1, 0, HCTerms[1]) => ENABLED HCQueries
        <3>1. (HCQuery(1, 0, HCTerms[1]) \/ HCQueries) <=> HCQueries
            BY <2>3, SMT
        <3>2. ENABLED (HCQuery(1, 0, HCTerms[1]) \/ HCQueries) <=> ENABLED HCQueries
            BY <3>1, ENABLEDaxioms
        <3>3. ENABLED (HCQuery(1, 0, HCTerms[1]) \/ HCQueries) <=>
                  ((ENABLED HCQuery(1, 0, HCTerms[1])) \/ (ENABLED HCQueries))
            BY ExpandENABLED, Isa DEF HCQueries, HCQuery, hcView
        <3> QED BY <3>2, <3>3, SMT
    <2>5. HCQueries <=> <<HCQueries>>_hcVars
        BY <1>3, <2>1, SMT DEF P, Q, hcVars
    <2>6. ENABLED HCQueries <=> ENABLED <<HCQueries>>_hcVars
        BY <2>5, ENABLEDaxioms
    <2> QED BY <2>2, <2>4, <2>6, SMT
<1>5. HCInspectSpec => []HCInvariant
    <2>1. HCInvariant /\ [HCQueries]_hcVars => HCInvariant'
        BY HCInvariantStep, SMT DEF HCNext
    <2> QED BY <2>1, PTL DEF HCInspectSpec
<1>6. HCInspectSpec => (P ~> Q)
    BY <1>2, <1>3, <1>4, <1>5, PTL DEF HCInspectSpec, P, Q
<1> QED BY <1>6, PTL DEF HCInspectSpec, HCInspectProgress, P, Q
=============================================================================
