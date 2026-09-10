------------------------- MODULE RaftTickMC -------------------------
EXTENDS RaftTick
CONSTANT RTHorizon
RTMCNext == (\E t \in 0..RTHorizon : RTAdvance(t)) \/ RTTraffic \/ RTTick \/ RTNotDue
RTMCSpec == RTInit /\ [][RTMCNext]_rtVars
RTMCLive == RTMCSpec /\ RTFairness
=============================================================================
