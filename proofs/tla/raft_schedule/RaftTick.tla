------------------------- MODULE RaftTick -------------------------
EXTENDS Naturals
CONSTANT RTPeriod
ASSUME RTLegalInputs == RTPeriod \in Nat \ {0}
\* Integer units abstract a monotonic Instant clock with a positive duration.
\* Arithmetic is unbounded here. Representable checked Instant addition and
\* the absence of externally injected production ticks are refinement premises.
VARIABLES rtNow, rtNext, rtLast, rtHas
rtVars == <<rtNow, rtNext, rtLast, rtHas>>
RTInit == rtNow = 0 /\ rtNext = RTPeriod /\ rtLast = 0 /\ rtHas = FALSE
RTAdvance(t) ==
    /\ t \in Nat /\ t >= rtNow /\ rtNow' = t
    /\ UNCHANGED <<rtNext, rtLast, rtHas>>
RTTraffic == UNCHANGED rtVars
RTTick ==
    /\ rtNow >= rtNext
    /\ rtNext' = rtNow + RTPeriod
    /\ rtLast' = rtNow /\ rtHas' = TRUE
    /\ UNCHANGED rtNow
RTNotDue == rtNow < rtNext /\ UNCHANGED rtVars
RTNext == (\E t \in Nat : RTAdvance(t)) \/ RTTraffic \/ RTTick \/ RTNotDue
RTSpec == RTInit /\ [][RTNext]_rtVars
RTType == rtNow \in Nat /\ rtNext \in Nat /\ rtLast \in Nat /\ rtHas \in BOOLEAN
RTInvariant ==
    /\ RTType /\ rtLast <= rtNow
    /\ (IF rtHas THEN rtNext = rtLast + RTPeriod ELSE rtNext = RTPeriod /\ rtLast = 0)
RTSpacing == (rtHas' /\ (~rtHas \/ rtLast' # rtLast)) => rtLast' >= rtNext
RTReset == (rtNext' # rtNext) => rtNext' = rtNow + RTPeriod /\ rtLast' = rtNow
RTMonotonic == rtNow' >= rtNow /\ rtLast' >= rtLast
RTSafety == [][RTSpacing /\ RTReset /\ RTMonotonic]_rtVars
RTFairness == WF_rtVars(RTTick)
RTFairSpec == RTInvariant /\ [][RTNext]_rtVars /\ RTFairness
RTProgress == (rtNow >= rtNext) ~> (rtNow < rtNext)
RTNoTickWitness == ~rtHas
RTNoDelayWitness == rtNow <= rtNext
=============================================================================
