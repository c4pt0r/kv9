------------------------- MODULE RaftCompletionMC -------------------------
EXTENDS RaftCompletion
RCSharedTarget == [w \in RCWaiters |-> 1]
RCDistinctTarget == [w \in RCWaiters |-> w]
RCMCLive == RCInit /\ [][RCNext]_rcVars /\ RCFairness
=============================================================================
