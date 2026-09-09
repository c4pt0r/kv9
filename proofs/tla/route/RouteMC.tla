----------------------------- MODULE RouteMC -----------------------------
EXTENDS RouteOwnership
CONSTANT RTLimit
RTMCUpdate == rtGeneration < RTLimit /\ (\E a \in RTAddresses : RTUpdate(a))
RTMCNext == RTMCUpdate \/ RTSpawn \/ RTObserve \/ RTEnqueue \/ RTDiscard \/ RTTake
            \/ RTEmit \/ RTConnect \/ RTFail \/ RTBlock \/ RTDrop \/ RTClose \/ RTTerminate \/ RTQuiesce
RTMCSpec == RTInit /\ [][RTMCNext]_rtVars
\* A stable migrated route with an old queued envelope and blocked session.
RTLiveInit == /\ rtAddress = RTInitial /\ rtGeneration = 3 /\ rtWorkers = 1
              /\ rtSession = 1 /\ rtSessionAddress = RTInitial /\ rtQueue = 1 /\ rtBatch = 1
              /\ rtPhase = "Blocked" /\ rtClosed = FALSE /\ rtBad = FALSE /\ rtDelivered = FALSE
RTMCLiveSpec == RTLiveInit /\ RTFairSpec
RTMCStopSpec == RTMCSpec /\ WF_rtVars(RTTerminate)
=============================================================================
