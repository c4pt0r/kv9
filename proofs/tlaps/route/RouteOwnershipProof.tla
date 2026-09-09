---------------------- MODULE RouteOwnershipProof ----------------------
EXTENDS RouteOwnership, TLAPS
THEOREM RTInvariantInit == RTInit => RTInvariant
BY RTLegalInputs, SMT DEF RTInit, RTInvariant, RTType, RTBinding, RTPhases, rtVars
THEOREM RTInvariantStep == ASSUME RTInvariant, [RTNext]_rtVars PROVE RTInvariant'
<1>1. \A a \in RTAddresses : RTUpdate(a) => RTInvariant'
    BY RTLegalInputs, SMT DEF RTUpdate, RTInvariant, RTType, RTBinding, RTPhases, rtVars
<1>2. RTSpawn \/ RTObserve \/ RTEnqueue \/ RTDiscard \/ RTTake => RTInvariant'
    BY RTLegalInputs, SMT DEF RTSpawn, RTObserve, RTEnqueue, RTDiscard, RTTake, RTInvariant, RTType, RTBinding, RTPhases, rtVars
<1>3. RTEmit \/ RTConnect \/ RTFail \/ RTBlock \/ RTDrop => RTInvariant'
    BY RTLegalInputs, SMT DEF RTEmit, RTConnect, RTFail, RTBlock, RTDrop, RTInvariant, RTType, RTBinding, RTPhases, rtVars
<1>4. RTClose \/ RTTerminate \/ RTQuiesce \/ UNCHANGED rtVars => RTInvariant'
    BY RTLegalInputs, SMT DEF RTClose, RTTerminate, RTQuiesce, RTInvariant, RTType, RTBinding, RTPhases, rtVars
<1> QED BY <1>1, <1>2, <1>3, <1>4, SMT DEF RTNext
THEOREM RTInvariantAlways == RTSpec => []RTInvariant
BY RTInvariantInit, RTInvariantStep, PTL DEF RTSpec
THEOREM RTEmissionProof == RTInvariant => RTEmissionSafe
BY SMT DEF RTEmissionSafe, RTEmit, RTInvariant, RTType, RTBinding, RTPhases, rtVars
THEOREM RTDeliveryProof == ASSUME RTInvariant, RTNext PROVE RTDeliverySafe
BY RTLegalInputs, SMT DEF RTDeliverySafe, RTNext, RTUpdate, RTSpawn, RTObserve, RTEnqueue, RTDiscard, RTTake, RTEmit, RTConnect, RTFail, RTBlock, RTDrop, RTClose, RTTerminate, RTQuiesce, RTInvariant, RTType, RTBinding, RTPhases, rtVars
THEOREM RTQueueCaptureProof == ASSUME RTInvariant, RTNext PROVE RTQueueCapture
BY SMT DEF RTQueueCapture, RTNext, RTUpdate, RTSpawn, RTObserve, RTEnqueue,
   RTDiscard, RTTake, RTEmit, RTConnect, RTFail, RTBlock, RTDrop, RTClose,
   RTTerminate, RTQuiesce, rtVars
THEOREM RTEffectsProof == RTSpec => RTEffects
<1>1. RTInvariant /\ [RTNext]_rtVars => [RTStepSafety]_rtVars
    BY RTEmissionProof, RTDeliveryProof, RTQueueCaptureProof, SMT DEF RTStepSafety, rtVars
<1> QED BY RTInvariantAlways, <1>1, PTL DEF RTSpec, RTEffects
THEOREM RTStableInvariant == RTFairSpec => []RTLive
<1>1. RTStableNext => RTNext BY SMT DEF RTStableNext, RTNext
<1>2. RTLive /\ [RTStableNext]_rtVars => RTLive'
    BY <1>1, RTInvariantStep, SMT DEF RTLive, RTStableNext, RTSpawn, RTObserve, RTEnqueue, RTDiscard, RTTake, RTEmit, RTConnect, RTQuiesce, rtVars
<1> QED BY <1>2, PTL DEF RTFairSpec, RTLive

THEOREM RTOwnedProgress == RTFairSpec => (RTLive ~> RTOwned)
<1>1. RTLive /\ [RTStableNext]_rtVars => RTLive'
    BY RTLegalInputs, RTInvariantStep, SMT DEF RTNext, RTLive, RTOwned, RTStableNext, RTSpawn, RTObserve, RTEnqueue, RTDiscard, RTTake, RTEmit, RTConnect, RTQuiesce, RTInvariant, RTType, RTBinding, RTPhases, rtVars
<1>2. RTLive /\ RTSpawn => RTOwned'
    BY <1>1, RTLegalInputs, SMT DEF RTLive, RTOwned, RTStableNext, RTSpawn, RTObserve, RTEnqueue, RTDiscard, RTTake, RTEmit, RTConnect, RTQuiesce, RTInvariant, RTType, RTBinding, RTPhases, rtVars
<1>3. RTLive /\ ~RTOwned => ENABLED <<RTSpawn>>_rtVars
    BY RTLegalInputs, ExpandENABLED, SMT DEF RTLive, RTOwned, RTStableNext, RTSpawn, RTObserve, RTEnqueue, RTDiscard, RTTake, RTEmit, RTConnect, RTQuiesce, RTInvariant, RTType, RTBinding, RTPhases, rtVars
<1> QED BY <1>1, <1>2, <1>3, PTL DEF RTFairSpec

THEOREM RTAlignedProgress == RTFairSpec => (RTOwned ~> RTAligned)
<1>1. RTOwned /\ [RTStableNext]_rtVars => RTOwned'
    BY RTLegalInputs, RTInvariantStep, SMT DEF RTNext, RTLive, RTOwned, RTAligned, RTStableNext, RTSpawn, RTObserve, RTEnqueue, RTDiscard, RTTake, RTEmit, RTConnect, RTQuiesce, RTInvariant, RTType, RTBinding, RTPhases, rtVars
<1>2. RTOwned /\ RTObserve => RTAligned'
    BY <1>1, RTLegalInputs, SMT DEF RTLive, RTOwned, RTAligned, RTStableNext, RTSpawn, RTObserve, RTEnqueue, RTDiscard, RTTake, RTEmit, RTConnect, RTQuiesce, RTInvariant, RTType, RTBinding, RTPhases, rtVars
<1>3. RTOwned /\ ~RTAligned => ENABLED <<RTObserve>>_rtVars
    BY RTLegalInputs, ExpandENABLED, SMT DEF RTLive, RTOwned, RTAligned, RTStableNext, RTSpawn, RTObserve, RTEnqueue, RTDiscard, RTTake, RTEmit, RTConnect, RTQuiesce, RTInvariant, RTType, RTBinding, RTPhases, rtVars
<1> QED BY <1>1, <1>2, <1>3, PTL DEF RTFairSpec

THEOREM RTStreamingProgress == RTFairSpec => (RTAligned ~> RTStreaming)
<1>1. RTAligned /\ [RTStableNext]_rtVars => RTAligned'
    BY RTLegalInputs, RTInvariantStep, SMT DEF RTNext, RTLive, RTOwned, RTAligned, RTStreaming, RTStableNext, RTSpawn, RTObserve, RTEnqueue, RTDiscard, RTTake, RTEmit, RTConnect, RTQuiesce, RTInvariant, RTType, RTBinding, RTPhases, rtVars
<1>2. RTAligned /\ RTConnect => RTStreaming'
    BY <1>1, RTLegalInputs, SMT DEF RTLive, RTOwned, RTAligned, RTStreaming, RTStableNext, RTSpawn, RTObserve, RTEnqueue, RTDiscard, RTTake, RTEmit, RTConnect, RTQuiesce, RTInvariant, RTType, RTBinding, RTPhases, rtVars
<1>3. RTAligned /\ ~RTStreaming => ENABLED <<RTConnect>>_rtVars
    BY RTLegalInputs, ExpandENABLED, SMT DEF RTLive, RTOwned, RTAligned, RTStreaming, RTStableNext, RTSpawn, RTObserve, RTEnqueue, RTDiscard, RTTake, RTEmit, RTConnect, RTQuiesce, RTInvariant, RTType, RTBinding, RTPhases, rtVars
<1> QED BY <1>1, <1>2, <1>3, PTL DEF RTFairSpec

THEOREM RTCleanProgress == RTFairSpec => (RTStreaming ~> RTClean)
<1>1. RTStreaming /\ [RTStableNext]_rtVars => RTStreaming'
    BY RTLegalInputs, RTInvariantStep, SMT DEF RTNext, RTLive, RTOwned, RTAligned, RTStreaming, RTClean, RTStableNext, RTSpawn, RTObserve, RTEnqueue, RTDiscard, RTTake, RTEmit, RTConnect, RTQuiesce, RTInvariant, RTType, RTBinding, RTPhases, rtVars
<1>2. RTStreaming /\ RTDiscard => RTClean'
    BY <1>1, RTLegalInputs, SMT DEF RTLive, RTOwned, RTAligned, RTStreaming, RTClean, RTStableNext, RTSpawn, RTObserve, RTEnqueue, RTDiscard, RTTake, RTEmit, RTConnect, RTQuiesce, RTInvariant, RTType, RTBinding, RTPhases, rtVars
<1>3. RTStreaming /\ ~RTClean => ENABLED <<RTDiscard>>_rtVars
    BY RTLegalInputs, ExpandENABLED, SMT DEF RTLive, RTOwned, RTAligned, RTStreaming, RTClean, RTStableNext, RTSpawn, RTObserve, RTEnqueue, RTDiscard, RTTake, RTEmit, RTConnect, RTQuiesce, RTInvariant, RTType, RTBinding, RTPhases, rtVars
<1> QED BY <1>1, <1>2, <1>3, PTL DEF RTFairSpec

THEOREM RTOfferedProgress == RTFairSpec => (RTClean ~> RTOffered)
<1>1. RTClean /\ [RTStableNext]_rtVars => RTClean'
    BY RTLegalInputs, RTInvariantStep, SMT DEF RTNext, RTLive, RTOwned, RTAligned, RTStreaming, RTClean, RTOffered, RTStableNext, RTSpawn, RTObserve, RTEnqueue, RTDiscard, RTTake, RTEmit, RTConnect, RTQuiesce, RTInvariant, RTType, RTBinding, RTPhases, rtVars
<1>2. RTClean /\ RTEnqueue => RTOffered'
    BY <1>1, RTLegalInputs, SMT DEF RTLive, RTOwned, RTAligned, RTStreaming, RTClean, RTOffered, RTStableNext, RTSpawn, RTObserve, RTEnqueue, RTDiscard, RTTake, RTEmit, RTConnect, RTQuiesce, RTInvariant, RTType, RTBinding, RTPhases, rtVars
<1>3. RTClean /\ ~RTOffered => ENABLED <<RTEnqueue>>_rtVars
    BY RTLegalInputs, ExpandENABLED, SMT DEF RTLive, RTOwned, RTAligned, RTStreaming, RTClean, RTOffered, RTStableNext, RTSpawn, RTObserve, RTEnqueue, RTDiscard, RTTake, RTEmit, RTConnect, RTQuiesce, RTInvariant, RTType, RTBinding, RTPhases, rtVars
<1> QED BY <1>1, <1>2, <1>3, PTL DEF RTFairSpec

THEOREM RTBatchedProgress == RTFairSpec => (RTOffered ~> RTBatched)
<1>1. RTOffered /\ [RTStableNext]_rtVars => RTOffered'
    BY RTLegalInputs, RTInvariantStep, SMT DEF RTNext, RTLive, RTOwned, RTAligned, RTStreaming, RTClean, RTOffered, RTBatched, RTStableNext, RTSpawn, RTObserve, RTEnqueue, RTDiscard, RTTake, RTEmit, RTConnect, RTQuiesce, RTInvariant, RTType, RTBinding, RTPhases, rtVars
<1>2. RTOffered /\ RTTake => RTBatched'
    BY <1>1, RTLegalInputs, SMT DEF RTLive, RTOwned, RTAligned, RTStreaming, RTClean, RTOffered, RTBatched, RTStableNext, RTSpawn, RTObserve, RTEnqueue, RTDiscard, RTTake, RTEmit, RTConnect, RTQuiesce, RTInvariant, RTType, RTBinding, RTPhases, rtVars
<1>3. RTOffered /\ ~RTBatched => ENABLED <<RTTake>>_rtVars
    BY RTLegalInputs, ExpandENABLED, SMT DEF RTLive, RTOwned, RTAligned, RTStreaming, RTClean, RTOffered, RTBatched, RTStableNext, RTSpawn, RTObserve, RTEnqueue, RTDiscard, RTTake, RTEmit, RTConnect, RTQuiesce, RTInvariant, RTType, RTBinding, RTPhases, rtVars
<1> QED BY <1>1, <1>2, <1>3, PTL DEF RTFairSpec

THEOREM RTDeliveredProgress == RTFairSpec => (RTBatched ~> RTDelivered)
<1>1. RTBatched /\ [RTStableNext]_rtVars => RTBatched'
    BY RTLegalInputs, RTInvariantStep, SMT DEF RTNext, RTLive, RTOwned, RTAligned, RTStreaming, RTClean, RTOffered, RTBatched, RTDelivered, RTStableNext, RTSpawn, RTObserve, RTEnqueue, RTDiscard, RTTake, RTEmit, RTConnect, RTQuiesce, RTInvariant, RTType, RTBinding, RTPhases, rtVars
<1>2. RTBatched /\ RTEmit => RTDelivered'
    BY <1>1, RTLegalInputs, SMT DEF RTLive, RTOwned, RTAligned, RTStreaming, RTClean, RTOffered, RTBatched, RTDelivered, RTStableNext, RTSpawn, RTObserve, RTEnqueue, RTDiscard, RTTake, RTEmit, RTConnect, RTQuiesce, RTInvariant, RTType, RTBinding, RTPhases, rtVars
<1>3. RTBatched /\ ~RTDelivered => ENABLED <<RTEmit>>_rtVars
    BY RTLegalInputs, ExpandENABLED, SMT DEF RTLive, RTOwned, RTAligned, RTStreaming, RTClean, RTOffered, RTBatched, RTDelivered, RTStableNext, RTSpawn, RTObserve, RTEnqueue, RTDiscard, RTTake, RTEmit, RTConnect, RTQuiesce, RTInvariant, RTType, RTBinding, RTPhases, rtVars
<1> QED BY <1>1, <1>2, <1>3, PTL DEF RTFairSpec

THEOREM RTProgressProof == RTFairSpec => RTProgress
BY RTStableInvariant, RTOwnedProgress, RTAlignedProgress, RTStreamingProgress,
   RTCleanProgress, RTOfferedProgress, RTBatchedProgress, RTDeliveredProgress,
   PTL DEF RTProgress, RTDelivered
THEOREM RTStopProgressProof == RTStopSpec => RTStopProgress
<1>1. RTStopping /\ [RTNext]_rtVars => RTStopping'
    BY RTInvariantStep, SMT DEF RTStopping, RTNext, RTUpdate, RTSpawn, RTObserve,
       RTEnqueue, RTDiscard, RTTake, RTEmit, RTConnect, RTFail, RTBlock, RTDrop,
       RTClose, RTTerminate, RTQuiesce, rtVars
<1>2. RTStopping /\ RTTerminate => RTStopped'
    BY <1>1, SMT DEF RTStopping, RTStopped, RTTerminate, RTNext, rtVars
<1>3. RTStopping /\ ~RTStopped => ENABLED <<RTTerminate>>_rtVars
    BY ExpandENABLED, SMT DEF RTStopping, RTStopped, RTTerminate,
       RTInvariant, RTType, RTBinding, RTPhases, rtVars
<1>4. RTStopSpec => (RTStopping ~> RTStopped)
    BY <1>1, <1>2, <1>3, PTL DEF RTStopSpec, RTSpec
<1> QED BY RTInvariantAlways, <1>4, PTL DEF RTStopSpec, RTStopProgress, RTStopping, RTStopped
=============================================================================
