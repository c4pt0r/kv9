------------------------- MODULE RaftCompletion -------------------------
EXTENDS Naturals
CONSTANTS RCMaxGeneration, RCWaiters, RCTokens, RCTarget, RCChosen
ASSUME RCLegalInputs ==
    /\ RCMaxGeneration \in Nat \ {0}
    /\ RCWaiters # {} /\ RCWaiters \subseteq Nat \ {0} /\ RCChosen \in RCWaiters
    /\ RCTokens # {} /\ RCTokens \subseteq Nat \ {0}
    /\ RCTarget \in [RCWaiters -> RCTokens]
\* One signal lifetime. Tokens denote already authorized exact predicates;
\* the receipt/context authority producing them is an external premise.
\* Several observable updates may precede one generation publication.
\* The distinguished value Max+1 represents None; it is never a u64 generation.
RCDead == RCMaxGeneration + 1
RCGenerations == 0..RCDead
RCPhases == {"observe", "check", "register", "park", "success", "error", "timeout"}
RCAdvance(g) == IF g = RCDead \/ g = RCMaxGeneration THEN RCDead ELSE g + 1
VARIABLES rcGeneration, rcPublisher, rcEvidence, rcPublished, rcHint,
    rcPhase, rcSeen, rcWake, rcStopped, rcFatal, rcNotifyOk
rcVars == <<rcGeneration, rcPublisher, rcEvidence, rcPublished, rcHint,
    rcPhase, rcSeen, rcWake, rcStopped, rcFatal, rcNotifyOk>>
RCPending == \E t \in RCTokens : rcPublisher[t] = "written"
RCInit ==
    /\ rcGeneration = 0 /\ rcPublisher = [t \in RCTokens |-> "new"]
    /\ rcEvidence = {} /\ rcPublished = {} /\ rcHint = FALSE
    /\ rcPhase = [w \in RCWaiters |-> "observe"]
    /\ rcSeen = [w \in RCWaiters |-> 0] /\ rcWake = {}
    /\ rcStopped = FALSE /\ rcFatal = FALSE /\ rcNotifyOk = TRUE
RCData(t) ==
    /\ t \in RCTokens /\ rcPublisher[t] = "new"
    /\ rcPublisher' = [rcPublisher EXCEPT ![t] = "written"]
    /\ rcEvidence' = rcEvidence \cup {t} /\ rcPublished' = rcPublished \cup {t}
    /\ UNCHANGED <<rcGeneration, rcHint, rcPhase, rcSeen, rcWake, rcStopped, rcFatal, rcNotifyOk>>
RCEvict(t) ==
    /\ t \in RCTokens /\ rcEvidence' = rcEvidence \ {t}
    /\ UNCHANGED <<rcGeneration, rcPublisher, rcPublished, rcHint, rcPhase, rcSeen, rcWake, rcStopped, rcFatal, rcNotifyOk>>
RCRequestHint == rcHint' = TRUE
    /\ UNCHANGED <<rcGeneration, rcPublisher, rcEvidence, rcPublished, rcPhase, rcSeen, rcWake, rcStopped, rcFatal, rcNotifyOk>>
RCStop == rcStopped' = TRUE /\ rcHint' = TRUE
    /\ UNCHANGED <<rcGeneration, rcPublisher, rcEvidence, rcPublished, rcPhase, rcSeen, rcWake, rcFatal, rcNotifyOk>>
RCFatal == rcFatal' = TRUE /\ rcHint' = TRUE
    /\ UNCHANGED <<rcGeneration, rcPublisher, rcEvidence, rcPublished, rcPhase, rcSeen, rcWake, rcStopped, rcNotifyOk>>
RCNotify ==
    /\ (RCPending \/ rcHint)
    /\ rcGeneration' = RCAdvance(rcGeneration)
    /\ rcPublisher' = [t \in RCTokens |-> IF rcPublisher[t] = "written" THEN "done" ELSE rcPublisher[t]]
    /\ rcHint' = FALSE
    /\ rcWake' = {w \in RCWaiters : rcPhase[w] = "park"}
    /\ rcNotifyOk' = (RCAdvance(rcGeneration) # RCDead)
    /\ UNCHANGED <<rcEvidence, rcPublished, rcPhase, rcSeen, rcStopped, rcFatal>>
RCObserve(w) ==
    /\ w \in RCWaiters /\ rcPhase[w] = "observe"
    /\ rcPhase' = [rcPhase EXCEPT ![w] = IF rcGeneration = RCDead THEN "error" ELSE "check"]
    /\ rcSeen' = [rcSeen EXCEPT ![w] = IF rcGeneration = RCDead THEN @ ELSE rcGeneration]
    /\ UNCHANGED <<rcGeneration, rcPublisher, rcEvidence, rcPublished, rcHint, rcWake, rcStopped, rcFatal, rcNotifyOk>>
RCCheck(w) ==
    /\ w \in RCWaiters /\ rcPhase[w] = "check"
    /\ rcPhase' = [rcPhase EXCEPT ![w] = IF rcFatal THEN "error"
          ELSE IF RCTarget[w] \in rcEvidence THEN "success" ELSE "register"]
    /\ UNCHANGED <<rcGeneration, rcPublisher, rcEvidence, rcPublished, rcHint, rcSeen, rcWake, rcStopped, rcFatal, rcNotifyOk>>
RCRegister(w) ==
    /\ w \in RCWaiters /\ rcPhase[w] = "register"
    /\ rcGeneration # RCDead /\ rcGeneration = rcSeen[w]
    /\ rcPhase' = [rcPhase EXCEPT ![w] = "park"]
    /\ rcWake' = rcWake \ {w}
    /\ UNCHANGED <<rcGeneration, rcPublisher, rcEvidence, rcPublished, rcHint, rcSeen, rcStopped, rcFatal, rcNotifyOk>>
RCReturn(w) ==
    /\ w \in RCWaiters
    /\ ((rcPhase[w] = "register" /\ (rcGeneration # rcSeen[w] \/ rcGeneration = RCDead))
          \/ (rcPhase[w] = "park" /\ w \in rcWake))
    /\ rcPhase' = [rcPhase EXCEPT ![w] = IF rcGeneration = RCDead THEN "error" ELSE "observe"]
    /\ rcWake' = rcWake \ {w}
    /\ UNCHANGED <<rcGeneration, rcPublisher, rcEvidence, rcPublished, rcHint, rcSeen, rcStopped, rcFatal, rcNotifyOk>>
RCExpire(w) ==
    /\ w \in RCWaiters /\ rcPhase[w] \notin {"success", "error", "timeout"}
    /\ rcPhase' = [rcPhase EXCEPT ![w] = "timeout"] /\ rcWake' = rcWake \ {w}
    /\ UNCHANGED <<rcGeneration, rcPublisher, rcEvidence, rcPublished, rcHint, rcSeen, rcStopped, rcFatal, rcNotifyOk>>
\* A spurious wake is a stutter after the condition-variable predicate loop.
RCQuiesce == UNCHANGED rcVars
RCNext == (\E t \in RCTokens : RCData(t) \/ RCEvict(t))
    \/ RCRequestHint \/ RCStop \/ RCFatal \/ RCNotify
    \/ (\E w \in RCWaiters : RCObserve(w) \/ RCCheck(w) \/ RCRegister(w) \/ RCReturn(w) \/ RCExpire(w)) \/ RCQuiesce
RCSpec == RCInit /\ [][RCNext]_rcVars
RCType ==
    /\ rcGeneration \in RCGenerations /\ rcPublisher \in [RCTokens -> {"new", "written", "done"}]
    /\ rcEvidence \subseteq RCTokens /\ rcPublished \subseteq RCTokens /\ rcHint \in BOOLEAN
    /\ rcPhase \in [RCWaiters -> RCPhases] /\ rcSeen \in [RCWaiters -> 0..RCMaxGeneration]
    /\ rcWake \subseteq RCWaiters /\ rcStopped \in BOOLEAN /\ rcFatal \in BOOLEAN /\ rcNotifyOk \in BOOLEAN
RCGenerationSeen == rcGeneration # RCDead => \A w \in RCWaiters : rcSeen[w] <= rcGeneration
RCAuthority ==
    /\ rcEvidence \subseteq rcPublished
    /\ \A w \in RCWaiters : rcPhase[w] = "success" => RCTarget[w] \in rcPublished
RCPublication ==
    /\ \A t \in RCTokens : (t \in rcPublished) <=> (rcPublisher[t] # "new")
    /\ (rcNotifyOk <=> rcGeneration # RCDead)
RCRegisteredSignal ==
    /\ \A w \in rcWake : rcPhase[w] = "park"
    /\ \A w \in RCWaiters : rcPhase[w] = "park" /\ rcGeneration # rcSeen[w] => w \in rcWake
RCNoLostCompletion == \A w \in RCWaiters : RCTarget[w] \in rcEvidence =>
    /\ (rcPhase[w] = "register" => rcGeneration # rcSeen[w] \/ rcPublisher[RCTarget[w]] = "written")
    /\ (rcPhase[w] = "park" => w \in rcWake \/ rcPublisher[RCTarget[w]] = "written")
RCNoLostFatal == rcFatal => \A w \in RCWaiters :
    /\ (rcPhase[w] = "register" => rcGeneration # rcSeen[w] \/ rcHint)
    /\ (rcPhase[w] = "park" => w \in rcWake \/ rcHint)
RCInvariant == RCType /\ RCGenerationSeen /\ RCAuthority /\ RCPublication /\ RCRegisteredSignal /\ RCNoLostCompletion /\ RCNoLostFatal
RCGenerationOrder ==
    /\ (rcGeneration = RCDead => rcGeneration' = RCDead)
    /\ (rcGeneration # RCDead /\ rcGeneration' # RCDead => rcGeneration' >= rcGeneration)
RCTerminals == \A w \in RCWaiters : rcPhase[w] \in {"success", "error", "timeout"} => rcPhase'[w] = rcPhase[w]
RCExactResult == \A w \in RCWaiters : rcPhase'[w] = "success" /\ rcPhase[w] # "success" => RCTarget[w] \in rcEvidence
RCStopHint == ~rcStopped /\ rcStopped' => rcHint'
RCSafety == [][RCGenerationOrder /\ RCTerminals /\ RCExactResult /\ RCStopHint]_rcVars
RCFairness == WF_rcVars(RCNotify) /\ WF_rcVars(RCReturn(RCChosen))
    /\ WF_rcVars(RCObserve(RCChosen)) /\ WF_rcVars(RCCheck(RCChosen))
RCFairSpec == RCInvariant /\ [][RCNext]_rcVars /\ RCFairness
RCRechecked == rcPhase[RCChosen] \in {"check", "success", "error", "timeout"}
RCWaiting == rcPhase[RCChosen] \in {"register", "park"}
RCDelivered == (rcPhase[RCChosen] = "register" /\ rcGeneration # rcSeen[RCChosen]) \/
    (rcPhase[RCChosen] = "park" /\ RCChosen \in rcWake)
RCNeedsPublication == RCWaiting /\ (rcPublisher[RCTarget[RCChosen]] = "written" \/ rcHint)
RCHintRecheckProgress == (RCWaiting /\ rcHint) ~> RCRechecked
RCCheckProgress == (rcPhase[RCChosen] = "check") ~>
    (rcPhase[RCChosen] \in {"register", "success", "error", "timeout"})
RCFatalProgress == rcFatal ~> (rcPhase[RCChosen] \in {"success", "error", "timeout"})
RCRecheckProgress == (rcPhase[RCChosen] \in {"register", "park"} /\
    (rcGeneration # rcSeen[RCChosen] \/ RCTarget[RCChosen] \in rcEvidence)) ~> RCRechecked
RCNoSuccessWitness == \A w \in RCWaiters : rcPhase[w] # "success"
RCNoExhaustionWitness == rcGeneration # RCDead
RCNoAllWakeWitness == rcWake # RCWaiters
RCNoEvictedWitness == rcEvidence = rcPublished
=============================================================================
