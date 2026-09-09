-------------------------- MODULE RouteOwnership --------------------------
EXTENDS Naturals
CONSTANT RTAddresses, RTInitial
ASSUME RTLegalInputs == RTAddresses # {} /\ RTAddresses \subseteq Nat \ {0}
                        /\ RTInitial \in RTAddresses

\* One authorized peer, one owned task, one sampled queued message and batch.
\* The sample represents an arbitrary queued envelope; other envelopes may
\* stutter or consume capacity. FIFO/capacity are separate channel premises.
\* Generations are unbounded ghost labels for immutable live Arc allocations.
VARIABLES rtAddress, rtGeneration, rtWorkers, rtSession, rtSessionAddress,
          rtQueue, rtBatch, rtPhase, rtClosed, rtBad, rtDelivered
rtVars == <<rtAddress, rtGeneration, rtWorkers, rtSession, rtSessionAddress,
            rtQueue, rtBatch, rtPhase, rtClosed, rtBad, rtDelivered>>
RTPhases == {"Idle", "Connect", "Backoff", "Stream", "Blocked", "Stopped"}
RTInit == /\ rtAddress = RTInitial /\ rtGeneration = 1 /\ rtWorkers = 0
          /\ rtSession = 0 /\ rtSessionAddress = 0 /\ rtQueue = 0 /\ rtBatch = 0
          /\ rtPhase = "Idle" /\ rtClosed = FALSE /\ rtBad = FALSE /\ rtDelivered = FALSE
RTUpdate(a) == /\ a \in RTAddresses /\ ~rtClosed /\ a # rtAddress
               /\ rtAddress' = a /\ rtGeneration' = rtGeneration + 1 /\ rtDelivered' = FALSE
               /\ UNCHANGED <<rtWorkers, rtSession, rtSessionAddress, rtQueue, rtBatch,
                              rtPhase, rtClosed, rtBad>>
RTSpawn == /\ ~rtClosed /\ rtWorkers = 0 /\ rtWorkers' = rtWorkers + 1
           /\ UNCHANGED <<rtAddress, rtGeneration, rtSession, rtSessionAddress, rtQueue,
                          rtBatch, rtPhase, rtClosed, rtBad, rtDelivered>>
RTObserve == /\ ~rtClosed /\ rtWorkers = 1 /\ rtSession # rtGeneration
             /\ rtSession' = rtGeneration /\ rtSessionAddress' = rtAddress
             /\ rtPhase' = "Connect" /\ rtBatch' = 0
             /\ UNCHANGED <<rtAddress, rtGeneration, rtWorkers, rtQueue, rtClosed, rtBad, rtDelivered>>
RTEnqueue == /\ ~rtClosed /\ rtWorkers = 1 /\ rtQueue = 0 /\ rtQueue' = rtGeneration
             /\ UNCHANGED <<rtAddress, rtGeneration, rtWorkers, rtSession, rtSessionAddress,
                            rtBatch, rtPhase, rtClosed, rtBad, rtDelivered>>
RTDiscard == /\ rtWorkers = 1 /\ rtPhase = "Stream" /\ rtQueue > 0 /\ rtQueue # rtSession
             /\ rtQueue' = 0
             /\ UNCHANGED <<rtAddress, rtGeneration, rtWorkers, rtSession, rtSessionAddress,
                            rtBatch, rtPhase, rtClosed, rtBad, rtDelivered>>
RTTake == /\ rtWorkers = 1 /\ rtPhase = "Stream" /\ rtQueue > 0 /\ rtBatch = 0
          /\ rtQueue = rtSession /\ rtBatch' = rtQueue /\ rtQueue' = 0
          /\ UNCHANGED <<rtAddress, rtGeneration, rtWorkers, rtSession, rtSessionAddress,
                         rtPhase, rtClosed, rtBad, rtDelivered>>
RTEmit == /\ rtWorkers = 1 /\ rtPhase = "Stream" /\ rtBatch > 0
          /\ rtBad' = (rtBad \/ rtBatch # rtSession)
          /\ rtDelivered' = (rtDelivered \/ rtBatch = rtGeneration) /\ rtBatch' = 0
          /\ UNCHANGED <<rtAddress, rtGeneration, rtWorkers, rtSession, rtSessionAddress,
                         rtQueue, rtPhase, rtClosed>>
RTConnect == /\ rtWorkers = 1 /\ rtSession > 0 /\ rtPhase # "Stream"
             /\ rtPhase' = "Stream"
             /\ UNCHANGED <<rtAddress, rtGeneration, rtWorkers, rtSession, rtSessionAddress,
                            rtQueue, rtBatch, rtClosed, rtBad, rtDelivered>>
RTFail == /\ rtWorkers = 1 /\ rtSession > 0 /\ rtPhase' = "Backoff" /\ rtBatch' = 0
          /\ UNCHANGED <<rtAddress, rtGeneration, rtWorkers, rtSession, rtSessionAddress,
                         rtQueue, rtClosed, rtBad, rtDelivered>>
RTBlock == /\ rtWorkers = 1 /\ rtPhase = "Stream" /\ rtPhase' = "Blocked"
           /\ UNCHANGED <<rtAddress, rtGeneration, rtWorkers, rtSession, rtSessionAddress,
                          rtQueue, rtBatch, rtClosed, rtBad, rtDelivered>>
RTDrop == /\ rtQueue' = 0 /\ rtBatch' = 0
          /\ UNCHANGED <<rtAddress, rtGeneration, rtWorkers, rtSession, rtSessionAddress,
                         rtPhase, rtClosed, rtBad, rtDelivered>>
RTClose == /\ ~rtClosed /\ rtClosed' = TRUE
           /\ UNCHANGED <<rtAddress, rtGeneration, rtWorkers, rtSession, rtSessionAddress,
                          rtQueue, rtBatch, rtPhase, rtBad, rtDelivered>>
RTTerminate == /\ rtClosed /\ rtWorkers = 1 /\ rtWorkers' = 0
               /\ rtSession' = 0 /\ rtSessionAddress' = 0 /\ rtQueue' = 0 /\ rtBatch' = 0
               /\ rtPhase' = "Stopped"
               /\ UNCHANGED <<rtAddress, rtGeneration, rtClosed, rtBad, rtDelivered>>
RTQuiesce == UNCHANGED rtVars
RTNext == (\E a \in RTAddresses : RTUpdate(a)) \/ RTSpawn \/ RTObserve \/ RTEnqueue
          \/ RTDiscard \/ RTTake \/ RTEmit \/ RTConnect \/ RTFail \/ RTBlock \/ RTDrop
          \/ RTClose \/ RTTerminate \/ RTQuiesce
RTSpec == RTInit /\ [][RTNext]_rtVars
RTType == /\ rtAddress \in RTAddresses /\ rtGeneration \in Nat \ {0}
          /\ rtWorkers \in {0, 1} /\ rtSession \in 0..rtGeneration
          /\ rtSessionAddress \in RTAddresses \cup {0} /\ rtQueue \in 0..rtGeneration
          /\ rtBatch \in 0..rtGeneration /\ rtPhase \in RTPhases
          /\ rtClosed \in BOOLEAN /\ rtBad \in BOOLEAN /\ rtDelivered \in BOOLEAN
RTBinding == /\ rtBatch \in {0, rtSession}
             /\ (rtSession = rtGeneration => rtSessionAddress = rtAddress)
             /\ (rtPhase \in {"Connect", "Backoff", "Stream", "Blocked"} => rtWorkers = 1 /\ rtSession > 0)
             /\ (rtWorkers = 0 => rtSession = 0 /\ rtBatch = 0 /\ rtQueue = 0)
RTInvariant == RTType /\ RTBinding /\ ~rtBad
RTEmissionSafe == RTEmit => rtBatch = rtSession
RTQueueCapture == (rtQueue = 0 /\ rtQueue' > 0) => rtQueue' = rtGeneration
RTDeliverySafe == (~rtDelivered /\ rtDelivered') => rtBatch = rtGeneration /\ rtSession = rtGeneration
RTStepSafety == RTEmissionSafe /\ RTQueueCapture /\ RTDeliverySafe
RTEffects == [][RTStepSafety]_rtVars

\* Eventual delivery assumes a stable authorized route, a healthy endpoint,
\* eventual queue admission/retransmission and fair task/I/O scheduling.
\* It is not delivery during indefinite route churn, outage, or overload.
RTStableNext == RTSpawn \/ RTObserve \/ RTEnqueue \/ RTDiscard \/ RTTake \/ RTEmit \/ RTConnect \/ RTQuiesce
RTFairSpec == RTInvariant /\ ~rtClosed /\ [][RTStableNext]_rtVars
              /\ WF_rtVars(RTSpawn) /\ WF_rtVars(RTObserve) /\ WF_rtVars(RTConnect)
              /\ WF_rtVars(RTDiscard) /\ WF_rtVars(RTEnqueue) /\ WF_rtVars(RTTake) /\ WF_rtVars(RTEmit)
RTLive == RTInvariant /\ ~rtClosed
RTOwned == RTLive /\ rtWorkers = 1
RTAligned == RTOwned /\ rtSession = rtGeneration
RTStreaming == RTAligned /\ rtPhase = "Stream"
RTClean == RTStreaming /\ rtQueue \in {0, rtGeneration}
RTOffered == RTClean /\ (rtQueue = rtGeneration \/ rtBatch = rtGeneration \/ rtDelivered)
RTBatched == RTOffered /\ (rtBatch = rtGeneration \/ rtDelivered)
RTDelivered == RTBatched /\ rtDelivered
RTProgress == <>rtDelivered
RTStopSpec == RTSpec /\ WF_rtVars(RTTerminate)
RTStopping == RTInvariant /\ rtClosed
RTStopped == RTStopping /\ rtWorkers = 0
RTStopProgress == rtClosed ~> (rtWorkers = 0)
RTNoAddressReuse == ~(rtGeneration > 1 /\ rtAddress = RTInitial)
RTNoDelivery == ~rtDelivered
=============================================================================
