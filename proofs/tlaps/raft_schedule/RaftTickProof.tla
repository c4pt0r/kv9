------------------------- MODULE RaftTickProof -------------------------
EXTENDS RaftTick, TLAPS
THEOREM RTInvariantInit == RTInit => RTInvariant
BY RTLegalInputs, SMT DEF RTInit, RTInvariant, RTType
THEOREM RTInvariantStep == ASSUME RTInvariant, [RTNext]_rtVars PROVE RTInvariant'
BY RTLegalInputs, SMT DEF RTNext, RTAdvance, RTTraffic, RTTick, RTNotDue, rtVars, RTInvariant, RTType
THEOREM RTInvariantAlways == RTSpec => []RTInvariant
BY RTInvariantInit, RTInvariantStep, PTL DEF RTSpec
THEOREM RTTrafficIndependent == RTTraffic => rtNext' = rtNext /\ rtLast' = rtLast /\ rtHas' = rtHas
BY SMT DEF RTTraffic, rtVars
THEOREM RTTickSpacing == ASSUME RTInvariant, RTNext PROVE RTSpacing
BY RTLegalInputs, SMT DEF RTSpacing, RTNext, RTAdvance, RTTraffic, RTTick, RTNotDue, rtVars, RTInvariant, RTType
THEOREM RTTickReset == ASSUME RTInvariant, RTNext PROVE RTReset
BY RTLegalInputs, SMT DEF RTReset, RTNext, RTAdvance, RTTraffic, RTTick, RTNotDue, rtVars, RTInvariant, RTType
THEOREM RTTimeMonotonic == ASSUME RTInvariant, RTNext PROVE RTMonotonic
BY RTLegalInputs, SMT DEF RTMonotonic, RTNext, RTAdvance, RTTraffic, RTTick, RTNotDue, rtVars, RTInvariant, RTType
THEOREM RTTickSafety == RTSpec => RTSafety
BY RTTickSpacing, RTTickReset, RTTimeMonotonic, RTInvariantAlways, PTL DEF RTSpec, RTSafety
THEOREM RTStableInvariant == RTFairSpec => []RTInvariant
BY RTInvariantStep, PTL DEF RTFairSpec
THEOREM RTTickProgress == RTFairSpec => RTProgress
<1>1. RTInvariant /\ rtNow >= rtNext /\ [RTNext]_rtVars =>
    RTInvariant' /\ ((rtNow >= rtNext)' \/ (rtNow < rtNext)')
    BY RTInvariantStep, RTLegalInputs, SMT DEF RTInvariant, RTType
<1>2. RTInvariant /\ rtNow >= rtNext /\ RTTick => (rtNow < rtNext)'
    BY RTLegalInputs, SMT DEF RTTick, RTInvariant, RTType
<1>3. RTInvariant /\ rtNow >= rtNext => ENABLED <<RTTick>>_rtVars
    BY ExpandENABLED, RTLegalInputs, SMT DEF RTTick, rtVars, RTInvariant, RTType
<1> QED BY RTStableInvariant, <1>1, <1>2, <1>3, PTL DEF RTFairSpec, RTFairness, RTProgress
=============================================================================
