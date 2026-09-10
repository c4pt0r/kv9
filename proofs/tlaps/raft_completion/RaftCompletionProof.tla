------------------------- MODULE RaftCompletionProof -------------------------
EXTENDS RaftCompletion, TLAPS

THEOREM RCInvariantInit == RCInit => RCInvariant
BY RCLegalInputs, SMT DEF RCInit, RCInvariant, RCType, RCGenerationSeen, RCAuthority, RCPublication, RCRegisteredSignal, RCNoLostCompletion, RCNoLostFatal, RCPending, RCAdvance, RCDead, RCGenerations, RCPhases

THEOREM RCDataStep == ASSUME RCInvariant, NEW t \in RCTokens, RCData(t) PROVE RCInvariant'
BY RCLegalInputs, SMT DEF RCData, RCInvariant, RCType, RCGenerationSeen, RCAuthority, RCPublication, RCRegisteredSignal, RCNoLostCompletion, RCNoLostFatal, RCPending, RCAdvance, RCDead, RCGenerations, RCPhases

THEOREM RCEvictStep == ASSUME RCInvariant, NEW t \in RCTokens, RCEvict(t) PROVE RCInvariant'
BY RCLegalInputs, SMT DEF RCEvict, RCInvariant, RCType, RCGenerationSeen, RCAuthority, RCPublication, RCRegisteredSignal, RCNoLostCompletion, RCNoLostFatal, RCPending, RCAdvance, RCDead, RCGenerations, RCPhases

THEOREM RCRequestHintStep == ASSUME RCInvariant, RCRequestHint PROVE RCInvariant'
BY RCLegalInputs, SMT DEF RCRequestHint, RCInvariant, RCType, RCGenerationSeen, RCAuthority, RCPublication, RCRegisteredSignal, RCNoLostCompletion, RCNoLostFatal, RCPending, RCAdvance, RCDead, RCGenerations, RCPhases

THEOREM RCStopStep == ASSUME RCInvariant, RCStop PROVE RCInvariant'
BY RCLegalInputs, SMT DEF RCStop, RCInvariant, RCType, RCGenerationSeen, RCAuthority, RCPublication, RCRegisteredSignal, RCNoLostCompletion, RCNoLostFatal, RCPending, RCAdvance, RCDead, RCGenerations, RCPhases

THEOREM RCFatalStep == ASSUME RCInvariant, RCFatal PROVE RCInvariant'
BY RCLegalInputs, SMT DEF RCFatal, RCInvariant, RCType, RCGenerationSeen, RCAuthority, RCPublication, RCRegisteredSignal, RCNoLostCompletion, RCNoLostFatal, RCPending, RCAdvance, RCDead, RCGenerations, RCPhases

THEOREM RCNotifyStep == ASSUME RCInvariant, RCNotify PROVE RCInvariant'
BY RCLegalInputs, SMT DEF RCNotify, RCInvariant, RCType, RCGenerationSeen, RCAuthority, RCPublication, RCRegisteredSignal, RCNoLostCompletion, RCNoLostFatal, RCPending, RCAdvance, RCDead, RCGenerations, RCPhases

THEOREM RCObserveStep == ASSUME RCInvariant, NEW w \in RCWaiters, RCObserve(w) PROVE RCInvariant'
BY RCLegalInputs, SMT DEF RCObserve, RCInvariant, RCType, RCGenerationSeen, RCAuthority, RCPublication, RCRegisteredSignal, RCNoLostCompletion, RCNoLostFatal, RCPending, RCAdvance, RCDead, RCGenerations, RCPhases

THEOREM RCCheckStep == ASSUME RCInvariant, NEW w \in RCWaiters, RCCheck(w) PROVE RCInvariant'
BY RCLegalInputs, SMT DEF RCCheck, RCInvariant, RCType, RCGenerationSeen, RCAuthority, RCPublication, RCRegisteredSignal, RCNoLostCompletion, RCNoLostFatal, RCPending, RCAdvance, RCDead, RCGenerations, RCPhases

THEOREM RCRegisterStep == ASSUME RCInvariant, NEW w \in RCWaiters, RCRegister(w) PROVE RCInvariant'
BY RCLegalInputs, SMT DEF RCRegister, RCInvariant, RCType, RCGenerationSeen, RCAuthority, RCPublication, RCRegisteredSignal, RCNoLostCompletion, RCNoLostFatal, RCPending, RCAdvance, RCDead, RCGenerations, RCPhases

THEOREM RCReturnStep == ASSUME RCInvariant, NEW w \in RCWaiters, RCReturn(w) PROVE RCInvariant'
BY RCLegalInputs, SMT DEF RCReturn, RCInvariant, RCType, RCGenerationSeen, RCAuthority, RCPublication, RCRegisteredSignal, RCNoLostCompletion, RCNoLostFatal, RCPending, RCAdvance, RCDead, RCGenerations, RCPhases

THEOREM RCExpireStep == ASSUME RCInvariant, NEW w \in RCWaiters, RCExpire(w) PROVE RCInvariant'
BY RCLegalInputs, SMT DEF RCExpire, RCInvariant, RCType, RCGenerationSeen, RCAuthority, RCPublication, RCRegisteredSignal, RCNoLostCompletion, RCNoLostFatal, RCPending, RCAdvance, RCDead, RCGenerations, RCPhases

THEOREM RCInvariantStep == ASSUME RCInvariant, [RCNext]_rcVars PROVE RCInvariant'
BY RCDataStep, RCEvictStep, RCRequestHintStep, RCStopStep, RCFatalStep, RCNotifyStep, RCObserveStep, RCCheckStep, RCRegisterStep, RCReturnStep, RCExpireStep, SMT DEF RCNext, RCQuiesce, rcVars, RCInvariant, RCType, RCGenerationSeen, RCAuthority, RCPublication, RCRegisteredSignal, RCNoLostCompletion, RCNoLostFatal, RCPending, RCAdvance, RCDead, RCGenerations, RCPhases

THEOREM RCInvariantAlways == RCSpec => []RCInvariant
BY RCInvariantInit, RCInvariantStep, PTL DEF RCSpec

THEOREM RCOrderStep == ASSUME RCInvariant, RCNext PROVE RCGenerationOrder /\ RCTerminals /\ RCExactResult /\ RCStopHint
BY RCLegalInputs, SMT DEF RCGenerationOrder, RCTerminals, RCExactResult, RCStopHint, RCNext, RCQuiesce, rcVars, RCData, RCEvict, RCRequestHint, RCStop, RCFatal, RCNotify, RCObserve, RCCheck, RCRegister, RCReturn, RCExpire, RCInvariant, RCType, RCGenerationSeen, RCAuthority, RCPublication, RCRegisteredSignal, RCNoLostCompletion, RCNoLostFatal, RCPending, RCAdvance, RCDead, RCGenerations, RCPhases

THEOREM RCOrderAlways == RCSpec => RCSafety
<1>1. RCInvariant /\ [RCNext]_rcVars => [RCGenerationOrder /\ RCTerminals /\ RCExactResult /\ RCStopHint]_rcVars
    BY RCOrderStep, SMT DEF rcVars
<1> QED BY <1>1, RCInvariantAlways, PTL DEF RCSpec, RCSafety

THEOREM RCHintHasNoAuthority == (RCRequestHint \/ RCNotify \/ RCStop \/ RCFatal) => UNCHANGED <<rcEvidence, rcPublished, rcPhase>>
BY SMT DEF RCRequestHint, RCNotify, RCStop, RCFatal

THEOREM RCExhaustionResult == ASSUME RCInvariant, RCNotify, rcGeneration = RCDead \/ rcGeneration = RCMaxGeneration PROVE rcGeneration' = RCDead /\ ~rcNotifyOk'
BY RCLegalInputs, SMT DEF RCNotify, RCAdvance, RCDead

THEOREM RCFairInvariant == RCFairSpec => []RCInvariant
BY RCInvariantStep, PTL DEF RCFairSpec

THEOREM RCPublicationProgress == RCFairSpec => ((RCNeedsPublication) ~> (RCDelivered \/ rcPhase[RCChosen] = "observe" \/ RCRechecked))
<1>1. DEFINE P == RCNeedsPublication
            Q == RCDelivered \/ rcPhase[RCChosen] = "observe" \/ RCRechecked
<1>2. RCInvariant /\ P /\ [RCNext]_rcVars => RCInvariant' /\ (P' \/ Q')
    BY RCInvariantStep, RCLegalInputs, SMT DEF P, Q, RCNext, RCData, RCEvict, RCRequestHint, RCStop, RCFatal, RCNotify, RCObserve, RCCheck, RCRegister, RCReturn, RCExpire, RCQuiesce, rcVars, RCInvariant, RCType, RCGenerationSeen, RCAuthority, RCPublication, RCRegisteredSignal, RCNoLostCompletion, RCNoLostFatal, RCPending, RCAdvance, RCDead, RCGenerations, RCPhases, RCNeedsPublication, RCWaiting, RCDelivered, RCRechecked
<1>3. RCInvariant /\ P /\ RCNotify => Q'
    BY RCLegalInputs, SMT DEF P, Q, RCNotify, RCInvariant, RCType, RCGenerationSeen, RCAuthority, RCPublication, RCRegisteredSignal, RCNoLostCompletion, RCNoLostFatal, RCPending, RCAdvance, RCDead, RCGenerations, RCPhases, RCNeedsPublication, RCWaiting, RCDelivered, RCRechecked
<1>4. RCInvariant /\ P => ENABLED <<RCNotify>>_rcVars
    <2>1. SUFFICES ASSUME RCInvariant, P PROVE ENABLED <<RCNotify>>_rcVars
        OBVIOUS
    <2>2. ENABLED RCNotify
        <3>1. RCPending \/ rcHint
            BY <2>1, RCLegalInputs, SMT DEF P, RCNeedsPublication, RCPending
        <3> QED BY <3>1, ExpandENABLED, Isa DEF RCNotify
    <2>3. RCNotify <=> <<RCNotify>>_rcVars
        BY <2>1, RCLegalInputs, SMT DEF P, RCNotify, rcVars, RCInvariant, RCType, RCGenerationSeen, RCAuthority, RCPublication, RCRegisteredSignal, RCNoLostCompletion, RCNoLostFatal, RCPending, RCAdvance, RCDead, RCGenerations, RCPhases, RCNeedsPublication, RCWaiting, RCDelivered, RCRechecked
    <2>4. ENABLED RCNotify <=> ENABLED <<RCNotify>>_rcVars
        BY <2>3, ENABLEDaxioms
    <2> QED BY <2>2, <2>4, SMT
<1> QED BY RCFairInvariant, <1>2, <1>3, <1>4, PTL DEF RCFairSpec, RCFairness, P, Q

THEOREM RCReturnProgress == RCFairSpec => ((RCDelivered) ~> (rcPhase[RCChosen] = "observe" \/ RCRechecked))
<1>1. DEFINE P == RCDelivered
            Q == rcPhase[RCChosen] = "observe" \/ RCRechecked
<1>2. RCInvariant /\ P /\ [RCNext]_rcVars => RCInvariant' /\ (P' \/ Q')
    BY RCInvariantStep, RCLegalInputs, SMT DEF P, Q, RCNext, RCData, RCEvict, RCRequestHint, RCStop, RCFatal, RCNotify, RCObserve, RCCheck, RCRegister, RCReturn, RCExpire, RCQuiesce, rcVars, RCInvariant, RCType, RCGenerationSeen, RCAuthority, RCPublication, RCRegisteredSignal, RCNoLostCompletion, RCNoLostFatal, RCPending, RCAdvance, RCDead, RCGenerations, RCPhases, RCNeedsPublication, RCWaiting, RCDelivered, RCRechecked
<1>3. RCInvariant /\ P /\ RCReturn(RCChosen) => Q'
    BY RCLegalInputs, SMT DEF P, Q, RCReturn, RCInvariant, RCType, RCGenerationSeen, RCAuthority, RCPublication, RCRegisteredSignal, RCNoLostCompletion, RCNoLostFatal, RCPending, RCAdvance, RCDead, RCGenerations, RCPhases, RCNeedsPublication, RCWaiting, RCDelivered, RCRechecked
<1>4. RCInvariant /\ P => ENABLED <<RCReturn(RCChosen)>>_rcVars
    <2>1. SUFFICES ASSUME RCInvariant, P PROVE ENABLED <<RCReturn(RCChosen)>>_rcVars
        OBVIOUS
    <2>2. ENABLED RCReturn(RCChosen)
        BY <2>1, ExpandENABLED, RCLegalInputs, SMT DEF P, RCReturn, RCInvariant, RCType, RCGenerationSeen, RCAuthority, RCPublication, RCRegisteredSignal, RCNoLostCompletion, RCNoLostFatal, RCPending, RCAdvance, RCDead, RCGenerations, RCPhases, RCNeedsPublication, RCWaiting, RCDelivered, RCRechecked
    <2>3. RCReturn(RCChosen) <=> <<RCReturn(RCChosen)>>_rcVars
        BY <2>1, RCLegalInputs, SMT DEF P, RCReturn, rcVars, RCInvariant, RCType, RCGenerationSeen, RCAuthority, RCPublication, RCRegisteredSignal, RCNoLostCompletion, RCNoLostFatal, RCPending, RCAdvance, RCDead, RCGenerations, RCPhases, RCNeedsPublication, RCWaiting, RCDelivered, RCRechecked
    <2>4. ENABLED RCReturn(RCChosen) <=> ENABLED <<RCReturn(RCChosen)>>_rcVars
        BY <2>3, ENABLEDaxioms
    <2> QED BY <2>2, <2>4, SMT
<1> QED BY RCFairInvariant, <1>2, <1>3, <1>4, PTL DEF RCFairSpec, RCFairness, P, Q

THEOREM RCObserveProgress == RCFairSpec => ((rcPhase[RCChosen] = "observe") ~> (RCRechecked))
<1>1. DEFINE P == rcPhase[RCChosen] = "observe"
            Q == RCRechecked
<1>2. RCInvariant /\ P /\ [RCNext]_rcVars => RCInvariant' /\ (P' \/ Q')
    BY RCInvariantStep, RCLegalInputs, SMT DEF P, Q, RCNext, RCData, RCEvict, RCRequestHint, RCStop, RCFatal, RCNotify, RCObserve, RCCheck, RCRegister, RCReturn, RCExpire, RCQuiesce, rcVars, RCInvariant, RCType, RCGenerationSeen, RCAuthority, RCPublication, RCRegisteredSignal, RCNoLostCompletion, RCNoLostFatal, RCPending, RCAdvance, RCDead, RCGenerations, RCPhases, RCNeedsPublication, RCWaiting, RCDelivered, RCRechecked
<1>3. RCInvariant /\ P /\ RCObserve(RCChosen) => Q'
    BY RCLegalInputs, SMT DEF P, Q, RCObserve, RCInvariant, RCType, RCGenerationSeen, RCAuthority, RCPublication, RCRegisteredSignal, RCNoLostCompletion, RCNoLostFatal, RCPending, RCAdvance, RCDead, RCGenerations, RCPhases, RCNeedsPublication, RCWaiting, RCDelivered, RCRechecked
<1>4. RCInvariant /\ P => ENABLED <<RCObserve(RCChosen)>>_rcVars
    BY ExpandENABLED, RCLegalInputs, SMT DEF P, RCObserve, rcVars, RCInvariant, RCType, RCGenerationSeen, RCAuthority, RCPublication, RCRegisteredSignal, RCNoLostCompletion, RCNoLostFatal, RCPending, RCAdvance, RCDead, RCGenerations, RCPhases, RCNeedsPublication, RCWaiting, RCDelivered, RCRechecked
<1> QED BY RCFairInvariant, <1>2, <1>3, <1>4, PTL DEF RCFairSpec, RCFairness, P, Q

THEOREM RCCheckService == RCFairSpec => ((rcPhase[RCChosen] = "check") ~> (rcPhase[RCChosen] \in {"register", "success", "error", "timeout"}))
<1>1. DEFINE P == rcPhase[RCChosen] = "check"
            Q == rcPhase[RCChosen] \in {"register", "success", "error", "timeout"}
<1>2. RCInvariant /\ P /\ [RCNext]_rcVars => RCInvariant' /\ (P' \/ Q')
    BY RCInvariantStep, RCLegalInputs, SMT DEF P, Q, RCNext, RCData, RCEvict, RCRequestHint, RCStop, RCFatal, RCNotify, RCObserve, RCCheck, RCRegister, RCReturn, RCExpire, RCQuiesce, rcVars, RCInvariant, RCType, RCGenerationSeen, RCAuthority, RCPublication, RCRegisteredSignal, RCNoLostCompletion, RCNoLostFatal, RCPending, RCAdvance, RCDead, RCGenerations, RCPhases, RCNeedsPublication, RCWaiting, RCDelivered, RCRechecked
<1>3. RCInvariant /\ P /\ RCCheck(RCChosen) => Q'
    BY RCLegalInputs, SMT DEF P, Q, RCCheck, RCInvariant, RCType, RCGenerationSeen, RCAuthority, RCPublication, RCRegisteredSignal, RCNoLostCompletion, RCNoLostFatal, RCPending, RCAdvance, RCDead, RCGenerations, RCPhases, RCNeedsPublication, RCWaiting, RCDelivered, RCRechecked
<1>4. RCInvariant /\ P => ENABLED <<RCCheck(RCChosen)>>_rcVars
    BY ExpandENABLED, RCLegalInputs, SMT DEF P, RCCheck, rcVars, RCInvariant, RCType, RCGenerationSeen, RCAuthority, RCPublication, RCRegisteredSignal, RCNoLostCompletion, RCNoLostFatal, RCPending, RCAdvance, RCDead, RCGenerations, RCPhases, RCNeedsPublication, RCWaiting, RCDelivered, RCRechecked
<1> QED BY RCFairInvariant, <1>2, <1>3, <1>4, PTL DEF RCFairSpec, RCFairness, P, Q

THEOREM RCRecheckProgressProof == RCFairSpec => RCRecheckProgress
<1>1. RCInvariant /\ rcPhase[RCChosen] \in {"register", "park"} /\
    (rcGeneration # rcSeen[RCChosen] \/ RCTarget[RCChosen] \in rcEvidence) => RCNeedsPublication \/ RCDelivered
    BY RCLegalInputs, SMT DEF RCNeedsPublication, RCWaiting, RCDelivered, RCInvariant, RCType, RCGenerationSeen, RCAuthority, RCPublication, RCRegisteredSignal, RCNoLostCompletion, RCNoLostFatal, RCPending, RCAdvance, RCDead, RCGenerations, RCPhases
<1> QED BY <1>1, RCFairInvariant, RCPublicationProgress, RCReturnProgress, RCObserveProgress,
    PTL DEF RCRecheckProgress

THEOREM RCHintRecheckProgressProof == RCFairSpec => RCHintRecheckProgress
<1>1. RCWaiting /\ rcHint => RCNeedsPublication BY SMT DEF RCNeedsPublication
<1> QED BY <1>1, RCPublicationProgress, RCReturnProgress, RCObserveProgress, PTL DEF RCHintRecheckProgress

THEOREM RCCheckProgressProof == RCFairSpec => RCCheckProgress
BY RCCheckService DEF RCCheckProgress

THEOREM RCFatalCheckProgress == RCFairSpec => ((rcFatal /\ rcPhase[RCChosen] = "check") ~>
    (rcPhase[RCChosen] \in {"success", "error", "timeout"}))
<1>1. DEFINE P == rcFatal /\ rcPhase[RCChosen] = "check"
            Q == rcPhase[RCChosen] \in {"success", "error", "timeout"}
<1>2. RCInvariant /\ P /\ [RCNext]_rcVars => RCInvariant' /\ (P' \/ Q')
    BY RCInvariantStep, RCLegalInputs, SMT DEF P, Q, RCNext, RCData, RCEvict, RCRequestHint, RCStop, RCFatal, RCNotify, RCObserve, RCCheck, RCRegister, RCReturn, RCExpire, RCQuiesce, rcVars, RCInvariant, RCType, RCGenerationSeen, RCAuthority, RCPublication, RCRegisteredSignal, RCNoLostCompletion, RCNoLostFatal, RCPending, RCAdvance, RCDead, RCGenerations, RCPhases
<1>3. RCInvariant /\ P /\ RCCheck(RCChosen) => Q'
    BY RCLegalInputs, SMT DEF P, Q, RCCheck, RCInvariant, RCType, RCGenerationSeen, RCAuthority, RCPublication, RCRegisteredSignal, RCNoLostCompletion, RCNoLostFatal, RCPending, RCAdvance, RCDead, RCGenerations, RCPhases
<1>4. RCInvariant /\ P => ENABLED <<RCCheck(RCChosen)>>_rcVars
    BY ExpandENABLED, RCLegalInputs, SMT DEF P, RCCheck, rcVars, RCInvariant, RCType, RCGenerationSeen, RCAuthority, RCPublication, RCRegisteredSignal, RCNoLostCompletion, RCNoLostFatal, RCPending, RCAdvance, RCDead, RCGenerations, RCPhases
<1> QED BY RCFairInvariant, <1>2, <1>3, <1>4, PTL DEF RCFairSpec, RCFairness, P, Q

THEOREM RCFatalProgressProof == RCFairSpec => RCFatalProgress
<1>1. RCInvariant /\ rcFatal =>
    RCNeedsPublication \/ RCDelivered \/ rcPhase[RCChosen] = "observe" \/ RCRechecked
    BY RCLegalInputs, SMT DEF RCNeedsPublication, RCWaiting, RCDelivered, RCRechecked, RCInvariant, RCType, RCGenerationSeen, RCAuthority, RCPublication, RCRegisteredSignal, RCNoLostCompletion, RCNoLostFatal, RCPending, RCAdvance, RCDead, RCGenerations, RCPhases
<1>2. RCInvariant /\ rcFatal /\ [RCNext]_rcVars => rcFatal'
    BY RCLegalInputs, SMT DEF RCNext, RCData, RCEvict, RCRequestHint, RCStop, RCFatal, RCNotify, RCObserve, RCCheck, RCRegister, RCReturn, RCExpire, RCQuiesce, rcVars
<1>3. RCRechecked <=> (rcPhase[RCChosen] = "check" \/ rcPhase[RCChosen] \in {"success", "error", "timeout"})
    BY SMT DEF RCRechecked
<1> QED BY <1>1, <1>2, <1>3, RCFairInvariant, RCPublicationProgress, RCReturnProgress,
    RCObserveProgress, RCFatalCheckProgress, PTL DEF RCFairSpec, RCFatalProgress
=============================================================================
