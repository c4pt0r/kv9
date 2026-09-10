---------------------------- MODULE RawGroup ----------------------------
EXTENDS RawMutation
CONSTANTS RGItems, RGMaxCount, RGMaxBytes, RGKind, RGOps, RGWeight, RGPosition,
    RGInitial, RGBasePosition, RGBaseDriver, RGOptIn, RGFenceKey, RGJudgeTable,
    RGReadFails, RGTailSuccess, RGRegion
RGEntries == 1..RGItems
RGKinds == {"put", "write", "fenced", "catalog", "manifest", "conf", "noop", "malformed"}
RGPositions == [term : Nat, index : Nat]
RGOutcomes == {"none", "ok", "stale"}
RGNoReceipt == [term |-> 0, index |-> 0, outcome |-> "none", region |-> 0]
ASSUME RGLegalInputs ==
    /\ RGItems \in Nat \ {0} /\ RGMaxCount \in Nat \ {0} /\ RGMaxBytes \in Nat \ {0}
    /\ RGKind \in [RGEntries -> RGKinds] /\ RGOps \in [RGEntries -> Seq(RMMutations)]
    /\ RGWeight \in [RGEntries -> Nat] /\ RGPosition \in [RGEntries -> RGPositions]
    /\ RGInitial \in RMStates /\ RGBasePosition \in RGPositions /\ RGBaseDriver \in RGPositions
    /\ RGOptIn \in BOOLEAN /\ RGFenceKey \in [RGEntries -> RMKeys]
    /\ RGJudgeTable \in [RGEntries -> [RMValues \cup {0} -> BOOLEAN]]
    /\ RGReadFails \subseteq RGEntries /\ RGTailSuccess \subseteq RGEntries
    /\ RGRegion \in [RGEntries -> Nat]
    \* The opt-in declaration is an explicit read-footprint premise, not a
    \* runtime inference from the Fenced tag. Production reads one System row.
    /\ (RGOptIn => \A i \in RGEntries : RGKind[i] = "fenced" => RGFenceKey[i] \in RMSystemKeys)
RGEligible(i) == RGKind[i] \in {"put", "write", "fenced"} /\ RMRawBatch(RGOps[i])
    /\ (RGKind[i] = "fenced" => RGOptIn)
RGVerdict(i, state) == IF RGKind[i] = "fenced" /\ ~RGJudgeTable[i][state[RGFenceKey[i]]]
    THEN "stale" ELSE "ok"
RGEffective(i, state) == IF RGVerdict(i, state) = "stale" THEN <<>> ELSE RGOps[i]
\* Only the validated Raw prefix is compared with this sequential reference.
\* Values beyond that prefix do not specify singleton/barrier application.
RGSequence == LET trace[i \in 0..RGItems] == IF i = 0 THEN RGInitial
    ELSE RMApply(trace[i - 1], RGEffective(i, trace[i - 1])) IN trace
RGAdvances(previous, next) == next.index > previous.index /\ next.term # 0 /\ next.term >= previous.term
RGGroupFailures == {"position-read", "plan", "before-effect", "after-effect"}
RGPhases == {"choose", "singleton", "load", "prepare", "write", "effect", "publish", "tail", "done", "fenced", "delegated"}
VARIABLES rgPhase, rgCount, rgBytes, rgByteLedger, rgPrepared, rgPrevious,
    rgBatch, rgReference, rgStaged, rgSequential, rgHasEffect, rgImage, rgImagePosition,
    rgEngineOk, rgSmPublication, rgReceipts, rgProcessed, rgDriver, rgFailure
rgVars == <<rgPhase, rgCount, rgBytes, rgByteLedger, rgPrepared, rgPrevious,
    rgBatch, rgReference, rgStaged, rgSequential, rgHasEffect, rgImage, rgImagePosition,
    rgEngineOk, rgSmPublication, rgReceipts, rgProcessed, rgDriver, rgFailure>>
RGInit ==
    /\ rgPhase = "choose" /\ rgCount = 1 /\ rgBytes = RGWeight[1]
    /\ rgByteLedger = [i \in 0..RGItems |-> IF i = 1 THEN RGWeight[1] ELSE 0]
    /\ rgPrepared = 0 /\ rgPrevious = RGBasePosition /\ rgBatch = <<>> /\ rgReference = RGInitial
    /\ rgStaged = [i \in RGEntries |-> "none"] /\ rgSequential = [i \in RGEntries |-> "none"]
    /\ rgHasEffect = FALSE /\ rgImage = RGInitial /\ rgImagePosition = RGBasePosition
    /\ rgEngineOk = FALSE /\ rgSmPublication = RGBasePosition
    /\ rgReceipts = [i \in RGEntries |-> RGNoReceipt]
    /\ rgProcessed = 0 /\ rgDriver = RGBaseDriver /\ rgFailure = "none"
RGCanGrow ==
    /\ rgCount < RGMaxCount /\ rgCount < RGItems /\ rgBytes <= RGMaxBytes
    /\ RGEligible(1) /\ RGPosition[1].index > RGBasePosition.index
    /\ RGWeight[rgCount + 1] <= RGMaxBytes - rgBytes /\ RGEligible(rgCount + 1)
RGGrow ==
    /\ rgPhase = "choose" /\ RGCanGrow
    /\ rgCount' = rgCount + 1 /\ rgBytes' = rgBytes + RGWeight[rgCount + 1]
    /\ rgByteLedger' = [rgByteLedger EXCEPT ![rgCount + 1] = rgBytes + RGWeight[rgCount + 1]]
    /\ UNCHANGED <<rgPhase, rgPrepared, rgPrevious, rgBatch, rgReference, rgStaged, rgSequential,
        rgHasEffect, rgImage, rgImagePosition, rgEngineOk, rgSmPublication, rgReceipts, rgProcessed, rgDriver, rgFailure>>
RGChooseDone ==
    /\ rgPhase = "choose" /\ ~RGCanGrow
    /\ rgPhase' = IF rgCount = 1 THEN "singleton" ELSE "load"
    /\ UNCHANGED <<rgCount, rgBytes, rgByteLedger, rgPrepared, rgPrevious, rgBatch, rgReference,
        rgStaged, rgSequential, rgHasEffect, rgImage, rgImagePosition, rgEngineOk,
        rgSmPublication, rgReceipts, rgProcessed, rgDriver, rgFailure>>
RGDelegate == rgPhase = "singleton" /\ rgPhase' = "delegated"
    /\ UNCHANGED <<rgCount, rgBytes, rgByteLedger, rgPrepared, rgPrevious, rgBatch, rgReference,
        rgStaged, rgSequential, rgHasEffect, rgImage, rgImagePosition, rgEngineOk,
        rgSmPublication, rgReceipts, rgProcessed, rgDriver, rgFailure>>
RGLoad == rgPhase = "load" /\ rgPhase' = "prepare"
    /\ UNCHANGED <<rgCount, rgBytes, rgByteLedger, rgPrepared, rgPrevious, rgBatch, rgReference,
        rgStaged, rgSequential, rgHasEffect, rgImage, rgImagePosition, rgEngineOk,
        rgSmPublication, rgReceipts, rgProcessed, rgDriver, rgFailure>>
RGCanPlan == rgPrepared < rgCount /\ RGAdvances(rgPrevious, RGPosition[rgPrepared + 1])
    /\ RGEligible(rgPrepared + 1)
    /\ ~(RGKind[rgPrepared + 1] = "fenced" /\ rgPrepared + 1 \in RGReadFails)
RGPlan ==
    /\ rgPhase = "prepare" /\ RGCanPlan
    /\ rgPrepared' = rgPrepared + 1 /\ rgPrevious' = RGPosition[rgPrepared + 1]
    /\ rgBatch' = rgBatch \o RGEffective(rgPrepared + 1, RGInitial)
    /\ rgReference' = RMApply(rgReference, RGEffective(rgPrepared + 1, rgReference))
    /\ rgStaged' = [rgStaged EXCEPT ![rgPrepared + 1] = RGVerdict(rgPrepared + 1, RGInitial)]
    /\ rgSequential' = [rgSequential EXCEPT ![rgPrepared + 1] = RGVerdict(rgPrepared + 1, rgReference)]
    /\ UNCHANGED <<rgPhase, rgCount, rgBytes, rgByteLedger, rgHasEffect, rgImage, rgImagePosition,
        rgEngineOk, rgSmPublication, rgReceipts, rgProcessed, rgDriver, rgFailure>>
RGPlanDone == rgPhase = "prepare" /\ rgPrepared = rgCount /\ rgPhase' = "write"
    /\ UNCHANGED <<rgCount, rgBytes, rgByteLedger, rgPrepared, rgPrevious, rgBatch, rgReference,
        rgStaged, rgSequential, rgHasEffect, rgImage, rgImagePosition, rgEngineOk,
        rgSmPublication, rgReceipts, rgProcessed, rgDriver, rgFailure>>
\* This is the positioned engine's atomic effect premise. It is an abstract
\* group effect witness, not a claim that future writes cannot overwrite keys.
\* Durable recovery interpretation requires a durable positioned engine; the
\* memory engine supplies only atomic volatile effects. No crash is modeled.
RGEffect ==
    /\ rgPhase = "write" /\ rgPhase' = "effect" /\ rgHasEffect' = TRUE
    /\ rgImage' = RMApply(RGInitial, rgBatch) /\ rgImagePosition' = RGPosition[rgCount]
    /\ UNCHANGED <<rgCount, rgBytes, rgByteLedger, rgPrepared, rgPrevious, rgBatch, rgReference,
        rgStaged, rgSequential, rgEngineOk, rgSmPublication, rgReceipts, rgProcessed, rgDriver, rgFailure>>
RGEngineSuccess == rgPhase = "effect" /\ rgPhase' = "publish" /\ rgEngineOk' = TRUE
    /\ UNCHANGED <<rgCount, rgBytes, rgByteLedger, rgPrepared, rgPrevious, rgBatch, rgReference,
        rgStaged, rgSequential, rgHasEffect, rgImage, rgImagePosition,
        rgSmPublication, rgReceipts, rgProcessed, rgDriver, rgFailure>>
RGPublish ==
    /\ rgPhase = "publish" /\ rgEngineOk /\ rgPhase' = "tail"
    /\ rgSmPublication' = RGPosition[rgCount] /\ rgProcessed' = rgCount
    /\ rgReceipts' = [i \in RGEntries |-> IF i <= rgCount
        THEN [term |-> RGPosition[i].term, index |-> RGPosition[i].index, outcome |-> rgStaged[i],
            region |-> IF rgStaged[i] = "stale" THEN RGRegion[i] ELSE 0]
        ELSE RGNoReceipt]
    /\ UNCHANGED <<rgCount, rgBytes, rgByteLedger, rgPrepared, rgPrevious, rgBatch, rgReference,
        rgStaged, rgSequential, rgHasEffect, rgImage, rgImagePosition, rgEngineOk, rgDriver, rgFailure>>
\* A later entry's success is supplied by its own apply contract. This action
\* records ordered completion only; it does not model its data or receipt effects.
RGTailPass ==
    /\ rgPhase = "tail" /\ rgProcessed < RGItems
    /\ RGKind[rgProcessed + 1] # "malformed" /\ rgProcessed + 1 \in RGTailSuccess
    /\ rgProcessed' = rgProcessed + 1
    /\ UNCHANGED <<rgPhase, rgCount, rgBytes, rgByteLedger, rgPrepared, rgPrevious, rgBatch, rgReference,
        rgStaged, rgSequential, rgHasEffect, rgImage, rgImagePosition, rgEngineOk,
        rgSmPublication, rgReceipts, rgDriver, rgFailure>>
RGReport ==
    /\ rgPhase = "tail" /\ rgProcessed = RGItems /\ rgPhase' = "done"
    /\ rgDriver' = IF RGPosition[RGItems].index > rgDriver.index THEN RGPosition[RGItems] ELSE rgDriver
    /\ UNCHANGED <<rgCount, rgBytes, rgByteLedger, rgPrepared, rgPrevious, rgBatch, rgReference,
        rgStaged, rgSequential, rgHasEffect, rgImage, rgImagePosition, rgEngineOk,
        rgSmPublication, rgReceipts, rgProcessed, rgFailure>>
RGFailureAt == CASE rgPhase = "load" -> "position-read"
    [] rgPhase = "prepare" /\ rgPrepared < rgCount /\ ~RGCanPlan -> "plan"
    [] rgPhase = "write" -> "before-effect"
    [] rgPhase = "effect" -> "after-effect"
    [] rgPhase = "tail" /\ rgProcessed < RGItems /\
        (RGKind[rgProcessed + 1] = "malformed" \/ rgProcessed + 1 \notin RGTailSuccess) -> "later"
    [] OTHER -> "none"
RGFail ==
    /\ RGFailureAt # "none" /\ rgPhase' = "fenced" /\ rgFailure' = RGFailureAt
    /\ UNCHANGED <<rgCount, rgBytes, rgByteLedger, rgPrepared, rgPrevious, rgBatch, rgReference,
        rgStaged, rgSequential, rgHasEffect, rgImage, rgImagePosition, rgEngineOk,
        rgSmPublication, rgReceipts, rgProcessed, rgDriver>>
RGService == RGGrow \/ RGChooseDone \/ RGDelegate \/ RGLoad \/ RGPlan \/ RGPlanDone \/
    RGEffect \/ RGEngineSuccess \/ RGPublish \/ RGTailPass \/ RGReport \/ RGFail
RGQuiesce == UNCHANGED rgVars
RGNext == RGService \/ RGQuiesce
RGSpec == RGInit /\ [][RGNext]_rgVars
RGTerminal == rgPhase \in {"done", "fenced", "delegated"}
RGType ==
    /\ rgPhase \in RGPhases /\ rgCount \in 1..RGItems /\ rgBytes \in Nat
    /\ rgByteLedger \in [0..RGItems -> Nat] /\ rgPrepared \in 0..rgCount /\ rgPrevious \in RGPositions
    /\ rgBatch \in Seq(RMMutations) /\ rgReference \in RMStates
    /\ rgStaged \in [RGEntries -> RGOutcomes] /\ rgSequential \in [RGEntries -> RGOutcomes]
    /\ rgHasEffect \in BOOLEAN /\ rgImage \in RMStates /\ rgImagePosition \in RGPositions
    /\ rgEngineOk \in BOOLEAN /\ rgSmPublication \in RGPositions
    /\ rgReceipts \in [RGEntries -> [term : Nat, index : Nat, outcome : RGOutcomes, region : Nat]]
    /\ rgProcessed \in 0..RGItems /\ rgDriver \in RGPositions /\ rgFailure \in RGGroupFailures \cup {"none", "later"}
RGSelection ==
    /\ rgCount <= RGMaxCount /\ (rgCount > 1 => rgBytes <= RGMaxBytes)
    /\ rgByteLedger[0] = 0 /\ rgBytes = rgByteLedger[rgCount]
    /\ \A i \in 1..rgCount : rgByteLedger[i] = rgByteLedger[i - 1] + RGWeight[i]
    /\ (rgCount > 1 => \A i \in 1..rgCount :
        RGKind[i] \in {"put", "write", "fenced"} /\ RMRawBatch(RGOps[i]) /\ (RGKind[i] = "fenced" => RGOptIn))
    /\ (rgPhase # "choose" => ~RGCanGrow)
RGPlanIdentity ==
    /\ rgPrevious = IF rgPrepared = 0 THEN RGBasePosition ELSE RGPosition[rgPrepared]
    /\ \A i \in 1..rgPrepared : RGAdvances(IF i = 1 THEN RGBasePosition ELSE RGPosition[i - 1], RGPosition[i])
    /\ \A i \in 1..rgPrepared : ~(RGKind[i] = "fenced" /\ i \in RGReadFails)
    /\ \A i \in RGEntries :
        /\ (i <= rgPrepared => rgStaged[i] = rgSequential[i] /\ rgStaged[i] = RGVerdict(i, RGInitial) /\ rgSequential[i] = RGVerdict(i, RGSequence[i - 1]))
        /\ (i > rgPrepared => rgStaged[i] = "none" /\ rgSequential[i] = "none")
RGComposition == RMApply(RGInitial, rgBatch) = rgReference /\ rgReference = RGSequence[rgPrepared] /\ RMRawBatch(rgBatch)
RGSystemUnchanged == \A key \in RMSystemKeys : rgReference[key] = RGInitial[key]
RGPhaseOrder ==
    /\ (rgPhase \in {"singleton", "delegated"} => rgCount = 1)
    /\ (rgPhase \in {"choose", "singleton", "load", "delegated"} => rgPrepared = 0)
    /\ (rgPhase \in {"load", "prepare", "write", "effect", "publish", "tail", "done"} => rgCount > 1)
    /\ (rgPhase \in {"write", "effect", "publish", "tail", "done"} => rgPrepared = rgCount)
    /\ (rgHasEffect <=> rgPhase \in {"effect", "publish", "tail", "done"} \/ rgFailure \in {"after-effect", "later"})
    /\ (rgEngineOk <=> rgPhase \in {"publish", "tail", "done"} \/ rgFailure = "later")
RGEngineBinding ==
    /\ (rgHasEffect => rgPrepared = rgCount /\ rgImage = rgReference /\ rgImagePosition = RGPosition[rgCount])
    /\ (~rgHasEffect => rgImage = RGInitial /\ rgImagePosition = RGBasePosition)
RGReceiptsExact == \A i \in RGEntries :
    /\ (rgReceipts[i].outcome # "none" =>
        /\ rgEngineOk /\ rgHasEffect /\ i <= rgCount
        /\ rgReceipts[i] = [term |-> RGPosition[i].term, index |-> RGPosition[i].index, outcome |-> rgSequential[i],
            region |-> IF rgSequential[i] = "stale" THEN RGRegion[i] ELSE 0])
    /\ ((rgPhase \in {"tail", "done"} \/ rgFailure = "later") /\ i <= rgCount => rgReceipts[i].outcome # "none")
RGPublication ==
    /\ (rgPhase \in {"tail", "done"} \/ rgFailure = "later" => rgSmPublication = RGPosition[rgCount] /\ rgProcessed >= rgCount)
    /\ (~(rgPhase \in {"tail", "done"} \/ rgFailure = "later") =>
        rgSmPublication = RGBasePosition /\ rgProcessed = 0 /\ rgReceipts = [i \in RGEntries |-> RGNoReceipt])
    /\ (rgFailure \in RGGroupFailures => ~rgEngineOk /\ rgSmPublication = RGBasePosition /\ rgDriver = RGBaseDriver)
RGContiguity == \A i \in (rgCount + 1)..rgProcessed : RGKind[i] # "malformed" /\ i \in RGTailSuccess
RGDriverAuthority ==
    /\ (rgDriver # RGBaseDriver => rgPhase = "done" /\ rgProcessed = RGItems /\ rgDriver = RGPosition[RGItems])
    /\ rgDriver.index >= RGBaseDriver.index
RGFenced == rgFailure # "none" => rgPhase = "fenced"
RGInvariant == RGType /\ RGSelection /\ RGPlanIdentity /\ RGComposition /\ RGSystemUnchanged /\ RGPhaseOrder /\
    RGEngineBinding /\ RGReceiptsExact /\ RGPublication /\ RGContiguity /\ RGDriverAuthority /\ RGFenced
RGFallbackStep == (rgPhase = "choose" /\ ~RGCanGrow /\ rgCount = 1 /\ rgPhase' # rgPhase) => rgPhase' = "singleton"
RGFallbackSafety == [][RGFallbackStep]_rgVars
RGTerminalStable == RGTerminal => UNCHANGED rgVars
RGSafety == [][RGTerminalStable]_rgVars
RGRank == CASE rgPhase = "choose" -> 4 * RGItems + 7 - rgCount
    [] rgPhase = "load" -> 3 * RGItems + 6
    [] rgPhase = "prepare" -> 3 * RGItems + 5 - rgPrepared
    [] rgPhase = "write" -> 2 * RGItems + 4
    [] rgPhase = "effect" -> 2 * RGItems + 3
    [] rgPhase = "publish" -> 2 * RGItems + 2
    [] rgPhase = "tail" -> RGItems + 1 - rgProcessed
    [] rgPhase = "singleton" -> 1
    [] OTHER -> 0
RGFairness == WF_rgVars(RGService)
RGFairSpec == RGInvariant /\ [][RGNext]_rgVars /\ RGFairness
RGProgress == TRUE ~> RGTerminal
RGNoSuccessWitness == rgPhase # "done"
RGNoUnknownWitness == ~(rgHasEffect /\ rgFailure = "after-effect")
RGNoLaterFailureWitness == ~(rgFailure = "later" /\ rgEngineOk)
RGNoStaleWitness == \A i \in RGEntries : rgReceipts[i].outcome # "stale"
RGNoSingletonWitness == rgPhase # "delegated"
RGNoEmptyEffectWitness == ~(rgHasEffect /\ rgBatch = <<>>)
=============================================================================
