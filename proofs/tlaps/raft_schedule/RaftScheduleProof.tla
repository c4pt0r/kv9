------------------------- MODULE RaftScheduleProof -------------------------
EXTENDS RaftSchedule, TLAPS

THEOREM RSInvariantInit == RSInit => RSInvariant
BY RSLegalInputs, SMT DEF RSInit, RSInvariant, RSType, RSUniqueOwner, RSThread, RSParkedSignal, RSNoLostWake, RSFinishHint, RSUnnotified, RSPhases, RSProducerPhases

THEOREM RSStartPublishStep == ASSUME RSInvariant, NEW p \in RSProducers, RSStartPublish(p) PROVE RSInvariant'
BY RSLegalInputs, SMT DEF RSStartPublish, RSInvariant, RSType, RSUniqueOwner, RSThread, RSParkedSignal, RSNoLostWake, RSFinishHint, RSUnnotified, RSPhases, RSProducerPhases

THEOREM RSPublishStep == ASSUME RSInvariant, NEW p \in RSProducers, RSPublish(p) PROVE RSInvariant'
BY RSLegalInputs, SMT DEF RSPublish, RSInvariant, RSType, RSUniqueOwner, RSThread, RSParkedSignal, RSNoLostWake, RSFinishHint, RSUnnotified, RSPhases, RSProducerPhases

THEOREM RSRefuseStep == ASSUME RSInvariant, NEW p \in RSProducers, RSRefuse(p) PROVE RSInvariant'
BY RSLegalInputs, SMT DEF RSRefuse, RSInvariant, RSType, RSUniqueOwner, RSThread, RSParkedSignal, RSNoLostWake, RSFinishHint, RSUnnotified, RSPhases, RSProducerPhases

THEOREM RSNotifyStep == ASSUME RSInvariant, NEW p \in RSProducers, RSNotify(p) PROVE RSInvariant'
BY RSLegalInputs, SMT DEF RSNotify, RSInvariant, RSType, RSUniqueOwner, RSThread, RSParkedSignal, RSNoLostWake, RSFinishHint, RSUnnotified, RSPhases, RSProducerPhases

THEOREM RSHintStep == ASSUME RSInvariant, RSHint PROVE RSInvariant'
BY RSLegalInputs, SMT DEF RSHint, RSInvariant, RSType, RSUniqueOwner, RSThread, RSParkedSignal, RSNoLostWake, RSFinishHint, RSUnnotified, RSPhases, RSProducerPhases

THEOREM RSSpawnStep == ASSUME RSInvariant, NEW o \in RSOwners, RSSpawn(o) PROVE RSInvariant'
BY RSLegalInputs, SMT DEF RSSpawn, RSInvariant, RSType, RSUniqueOwner, RSThread, RSParkedSignal, RSNoLostWake, RSFinishHint, RSUnnotified, RSPhases, RSProducerPhases

THEOREM RSBeginStep == ASSUME RSInvariant, RSBegin PROVE RSInvariant'
BY RSLegalInputs, SMT DEF RSBegin, RSInvariant, RSType, RSUniqueOwner, RSThread, RSParkedSignal, RSNoLostWake, RSFinishHint, RSUnnotified, RSPhases, RSProducerPhases

THEOREM RSDrainStep == ASSUME RSInvariant, NEW k \in 0..RSDrainMax, RSDrain(k) PROVE RSInvariant'
BY RSLegalInputs, SMT DEF RSDrain, RSInvariant, RSType, RSUniqueOwner, RSThread, RSParkedSignal, RSNoLostWake, RSFinishHint, RSUnnotified, RSPhases, RSProducerPhases

THEOREM RSFinishStep == ASSUME RSInvariant, RSFinish PROVE RSInvariant'
BY RSLegalInputs, SMT DEF RSFinish, RSInvariant, RSType, RSUniqueOwner, RSThread, RSParkedSignal, RSNoLostWake, RSFinishHint, RSUnnotified, RSPhases, RSProducerPhases

THEOREM RSParkStep == ASSUME RSInvariant, RSPark PROVE RSInvariant'
BY RSLegalInputs, SMT DEF RSPark, RSInvariant, RSType, RSUniqueOwner, RSThread, RSParkedSignal, RSNoLostWake, RSFinishHint, RSUnnotified, RSPhases, RSProducerPhases

THEOREM RSWakeStep == ASSUME RSInvariant, RSWake PROVE RSInvariant'
BY RSLegalInputs, SMT DEF RSWake, RSInvariant, RSType, RSUniqueOwner, RSThread, RSParkedSignal, RSNoLostWake, RSFinishHint, RSUnnotified, RSPhases, RSProducerPhases

THEOREM RSTimeoutStep == ASSUME RSInvariant, RSTimeout PROVE RSInvariant'
BY RSLegalInputs, SMT DEF RSTimeout, RSInvariant, RSType, RSUniqueOwner, RSThread, RSParkedSignal, RSNoLostWake, RSFinishHint, RSUnnotified, RSPhases, RSProducerPhases

THEOREM RSStopStep == ASSUME RSInvariant, RSStop PROVE RSInvariant'
BY RSLegalInputs, SMT DEF RSStop, RSInvariant, RSType, RSUniqueOwner, RSThread, RSParkedSignal, RSNoLostWake, RSFinishHint, RSUnnotified, RSPhases, RSProducerPhases

THEOREM RSExitStep == ASSUME RSInvariant, NEW clear \in BOOLEAN, RSExit(clear) PROVE RSInvariant'
BY RSLegalInputs, SMT DEF RSExit, RSInvariant, RSType, RSUniqueOwner, RSThread, RSParkedSignal, RSNoLostWake, RSFinishHint, RSUnnotified, RSPhases, RSProducerPhases

THEOREM RSInvariantStep == ASSUME RSInvariant, [RSNext]_rsVars PROVE RSInvariant'
BY RSStartPublishStep, RSPublishStep, RSRefuseStep, RSNotifyStep, RSHintStep, RSSpawnStep, RSBeginStep, RSDrainStep, RSFinishStep, RSParkStep, RSWakeStep, RSTimeoutStep, RSStopStep, RSExitStep, SMT DEF RSNext, RSWorkNext, RSQuiesce, rsVars, RSInvariant, RSType, RSUniqueOwner, RSThread, RSParkedSignal, RSNoLostWake, RSFinishHint, RSUnnotified, RSPhases, RSProducerPhases

THEOREM RSInvariantAlways == RSSpec => []RSInvariant
BY RSInvariantInit, RSInvariantStep, PTL DEF RSSpec

THEOREM RSTerminalStep == ASSUME RSInvariant, RSNext PROVE RSStopMonotonic /\ RSHaltTerminal /\ RSNoBeginAfterStop
BY RSLegalInputs, SMT DEF RSStopMonotonic, RSHaltTerminal, RSNoBeginAfterStop, RSNext, RSWorkNext, RSQuiesce, rsVars, RSStartPublish, RSPublish, RSRefuse, RSNotify, RSHint, RSSpawn, RSBegin, RSDrain, RSFinish, RSPark, RSWake, RSTimeout, RSStop, RSExit, RSInvariant, RSType, RSUniqueOwner, RSThread, RSParkedSignal, RSNoLostWake, RSFinishHint, RSUnnotified, RSPhases, RSProducerPhases

THEOREM RSTerminalAlways == RSSpec => RSStopAlways
<1>1. RSInvariant /\ [RSNext]_rsVars => [RSStopMonotonic /\ RSHaltTerminal /\ RSNoBeginAfterStop]_rsVars
    BY RSTerminalStep, SMT DEF rsVars
<1> QED BY <1>1, RSInvariantAlways, PTL DEF RSSpec, RSStopAlways

THEOREM RSServiceInvariant == RSServiceSpec => [](RSInvariant /\ ~rsStopped /\ rsClaimed # {})
<1>1. RSWorkNext => RSNext BY SMT DEF RSWorkNext, RSNext
<1>2. RSInvariant /\ ~rsStopped /\ rsClaimed # {} /\ [RSWorkNext]_rsVars =>
    (RSInvariant /\ ~rsStopped /\ rsClaimed # {})'
    BY <1>1, RSInvariantStep, SMT DEF RSWorkNext, RSStartPublish, RSPublish, RSRefuse, RSNotify, RSHint, RSBegin, RSDrain, RSFinish, RSPark, RSWake, RSQuiesce, rsVars
<1> QED BY <1>2, PTL DEF RSServiceSpec

THEOREM RSCallbackProgress == RSServiceSpec => ((rsQueue > 0 /\ rsPhase \in {"wait", "park"} /\ ~rsPending /\ ~rsWake) ~> (rsQueue > 0 /\ rsPhase \in {"wait", "park"} /\ (rsPending \/ rsWake)))
<1>1. DEFINE P == rsQueue > 0 /\ rsPhase \in {"wait", "park"} /\ ~rsPending /\ ~rsWake
            Q == rsQueue > 0 /\ rsPhase \in {"wait", "park"} /\ (rsPending \/ rsWake)
<1>2. RSInvariant /\ ~rsStopped /\ P /\ [RSWorkNext]_rsVars => RSInvariant' /\ (P' \/ Q')
    BY RSInvariantStep, RSLegalInputs, SMT DEF P, Q, RSNext, RSWorkNext, RSStartPublish, RSPublish, RSRefuse, RSNotify, RSHint, RSBegin, RSDrain, RSFinish, RSPark, RSWake, RSQuiesce, rsVars, RSInvariant, RSType, RSUniqueOwner, RSThread, RSParkedSignal, RSNoLostWake, RSFinishHint, RSUnnotified, RSPhases, RSProducerPhases
<1>3. RSInvariant /\ ~rsStopped /\ P /\ RSNotifyAny => Q'
    BY SMT DEF P, Q, RSNotifyAny, RSNotify
<1>4. RSInvariant /\ ~rsStopped /\ P => ENABLED <<RSNotifyAny>>_rsVars
    <2>1. SUFFICES ASSUME RSInvariant, ~rsStopped, P PROVE ENABLED <<RSNotifyAny>>_rsVars
        OBVIOUS
    <2>2. PICK p \in RSProducers : rsProducer[p] = "published"
        BY <2>1, SMT DEF P, RSInvariant, RSType, RSUniqueOwner, RSThread, RSParkedSignal, RSNoLostWake, RSFinishHint, RSUnnotified, RSPhases, RSProducerPhases
    <2>3. rsProducer # [rsProducer EXCEPT ![p] = "idle"]
        BY <2>1, <2>2, SMT DEF RSInvariant, RSType, RSUniqueOwner, RSThread, RSParkedSignal, RSNoLostWake, RSFinishHint, RSUnnotified, RSPhases, RSProducerPhases
    <2>4. ENABLED RSNotifyAny
        BY <2>1, <2>2, <2>3, ExpandENABLED, SMT DEF P, RSNotifyAny, RSNotify
    <2>5. RSNotifyAny <=> <<RSNotifyAny>>_rsVars
        BY <2>1, SMT DEF P, RSNotifyAny, RSNotify, rsVars
    <2>6. ENABLED RSNotifyAny <=> ENABLED <<RSNotifyAny>>_rsVars
        BY <2>5, ENABLEDaxioms
    <2> QED BY <2>4, <2>6, SMT
<1> QED BY RSServiceInvariant, <1>2, <1>3, <1>4, PTL DEF RSServiceSpec, RSFairness, P, Q

THEOREM RSWakeProgress == RSServiceSpec => ((rsPhase \in {"wait", "park"} /\ (rsPending \/ rsWake)) ~> (rsPhase = "awake"))
<1>1. DEFINE P == rsPhase \in {"wait", "park"} /\ (rsPending \/ rsWake)
            Q == rsPhase = "awake"
<1>2. RSInvariant /\ ~rsStopped /\ P /\ [RSWorkNext]_rsVars => RSInvariant' /\ (P' \/ Q')
    BY RSInvariantStep, RSLegalInputs, SMT DEF P, Q, RSNext, RSWorkNext, RSStartPublish, RSPublish, RSRefuse, RSNotify, RSHint, RSBegin, RSDrain, RSFinish, RSPark, RSWake, RSQuiesce, rsVars, RSInvariant, RSType, RSUniqueOwner, RSThread, RSParkedSignal, RSNoLostWake, RSFinishHint, RSUnnotified, RSPhases, RSProducerPhases
<1>3. RSInvariant /\ ~rsStopped /\ P /\ RSWake => Q'
    BY SMT DEF P, Q, RSWake
<1>4. RSInvariant /\ ~rsStopped /\ P => ENABLED <<RSWake>>_rsVars
    BY ExpandENABLED, RSLegalInputs, SMT DEF P, RSWake, rsVars, RSInvariant, RSType, RSUniqueOwner, RSThread, RSParkedSignal, RSNoLostWake, RSFinishHint, RSUnnotified, RSPhases, RSProducerPhases
<1> QED BY RSServiceInvariant, <1>2, <1>3, <1>4, PTL DEF RSServiceSpec, RSFairness, P, Q

THEOREM RSBeginProgress == RSServiceSpec => ((rsPhase = "awake") ~> (rsPhase = "drain"))
<1>1. DEFINE P == rsPhase = "awake"
            Q == rsPhase = "drain"
<1>2. RSInvariant /\ ~rsStopped /\ P /\ [RSWorkNext]_rsVars => RSInvariant' /\ (P' \/ Q')
    BY RSInvariantStep, RSLegalInputs, SMT DEF P, Q, RSNext, RSWorkNext, RSStartPublish, RSPublish, RSRefuse, RSNotify, RSHint, RSBegin, RSDrain, RSFinish, RSPark, RSWake, RSQuiesce, rsVars, RSInvariant, RSType, RSUniqueOwner, RSThread, RSParkedSignal, RSNoLostWake, RSFinishHint, RSUnnotified, RSPhases, RSProducerPhases
<1>3. RSInvariant /\ ~rsStopped /\ P /\ RSBegin => Q'
    BY SMT DEF P, Q, RSBegin
<1>4. RSInvariant /\ ~rsStopped /\ P => ENABLED <<RSBegin>>_rsVars
    BY ExpandENABLED, RSLegalInputs, SMT DEF P, RSBegin, rsVars, RSInvariant, RSType, RSUniqueOwner, RSThread, RSParkedSignal, RSNoLostWake, RSFinishHint, RSUnnotified, RSPhases, RSProducerPhases
<1> QED BY RSServiceInvariant, <1>2, <1>3, <1>4, PTL DEF RSServiceSpec, RSFairness, P, Q

THEOREM RSDrainProgress == RSServiceSpec => ((rsPhase = "drain") ~> (rsPhase = "finish"))
<1>1. DEFINE P == rsPhase = "drain"
            Q == rsPhase = "finish"
<1>2. RSInvariant /\ ~rsStopped /\ P /\ [RSWorkNext]_rsVars => RSInvariant' /\ (P' \/ Q')
    BY RSInvariantStep, RSLegalInputs, SMT DEF P, Q, RSNext, RSWorkNext, RSStartPublish, RSPublish, RSRefuse, RSNotify, RSHint, RSBegin, RSDrain, RSFinish, RSPark, RSWake, RSQuiesce, rsVars, RSInvariant, RSType, RSUniqueOwner, RSThread, RSParkedSignal, RSNoLostWake, RSFinishHint, RSUnnotified, RSPhases, RSProducerPhases
<1>3. RSInvariant /\ ~rsStopped /\ P /\ RSDrainAny => Q'
    BY SMT DEF P, Q, RSDrainAny, RSDrain
<1>4. RSInvariant /\ ~rsStopped /\ P => ENABLED <<RSDrainAny>>_rsVars
    <2>1. SUFFICES ASSUME RSInvariant, ~rsStopped, P PROVE ENABLED <<RSDrainAny>>_rsVars
        OBVIOUS
    <2>2. DEFINE k == IF rsQueue = 0 THEN 0 ELSE 1
    <2>3. k \in 0..RSDrainMax /\ k <= rsQueue /\ (rsQueue > 0 => k > 0)
        BY <2>1, RSLegalInputs, SMT DEF k, RSInvariant, RSType, RSUniqueOwner, RSThread, RSParkedSignal, RSNoLostWake, RSFinishHint, RSUnnotified, RSPhases, RSProducerPhases
    <2>4. ENABLED RSDrainAny
        BY <2>1, <2>3, ExpandENABLED, SMT DEF P, RSDrainAny, RSDrain
    <2>5. RSDrainAny <=> <<RSDrainAny>>_rsVars
        BY <2>1, SMT DEF P, RSDrainAny, RSDrain, rsVars
    <2>6. ENABLED RSDrainAny <=> ENABLED <<RSDrainAny>>_rsVars
        BY <2>5, ENABLEDaxioms
    <2> QED BY <2>4, <2>6, SMT
<1> QED BY RSServiceInvariant, <1>2, <1>3, <1>4, PTL DEF RSServiceSpec, RSFairness, P, Q

THEOREM RSFinishToWait == RSServiceSpec => ((rsQueue > 0 /\ rsPhase = "finish") ~> (rsQueue > 0 /\ rsPhase = "wait"))
<1>1. DEFINE P == rsQueue > 0 /\ rsPhase = "finish"
            Q == rsQueue > 0 /\ rsPhase = "wait"
<1>2. RSInvariant /\ ~rsStopped /\ P /\ [RSWorkNext]_rsVars => RSInvariant' /\ (P' \/ Q')
    BY RSInvariantStep, RSLegalInputs, SMT DEF P, Q, RSNext, RSWorkNext, RSStartPublish, RSPublish, RSRefuse, RSNotify, RSHint, RSBegin, RSDrain, RSFinish, RSPark, RSWake, RSQuiesce, rsVars, RSInvariant, RSType, RSUniqueOwner, RSThread, RSParkedSignal, RSNoLostWake, RSFinishHint, RSUnnotified, RSPhases, RSProducerPhases
<1>3. RSInvariant /\ ~rsStopped /\ P /\ RSFinish => Q'
    BY SMT DEF P, Q, RSFinish
<1>4. RSInvariant /\ ~rsStopped /\ P => ENABLED <<RSFinish>>_rsVars
    BY ExpandENABLED, RSLegalInputs, SMT DEF P, RSFinish, rsVars, RSInvariant, RSType, RSUniqueOwner, RSThread, RSParkedSignal, RSNoLostWake, RSFinishHint, RSUnnotified, RSPhases, RSProducerPhases
<1> QED BY RSServiceInvariant, <1>2, <1>3, <1>4, PTL DEF RSServiceSpec, RSFairness, P, Q

THEOREM RSFinishHintToWait == RSServiceSpec => ((rsPending /\ rsPhase = "finish") ~> (rsPending /\ rsPhase = "wait"))
<1>1. DEFINE P == rsPending /\ rsPhase = "finish"
            Q == rsPending /\ rsPhase = "wait"
<1>2. RSInvariant /\ ~rsStopped /\ P /\ [RSWorkNext]_rsVars => RSInvariant' /\ (P' \/ Q')
    BY RSInvariantStep, RSLegalInputs, SMT DEF P, Q, RSNext, RSWorkNext, RSStartPublish, RSPublish, RSRefuse, RSNotify, RSHint, RSBegin, RSDrain, RSFinish, RSPark, RSWake, RSQuiesce, rsVars, RSInvariant, RSType, RSUniqueOwner, RSThread, RSParkedSignal, RSNoLostWake, RSFinishHint, RSUnnotified, RSPhases, RSProducerPhases
<1>3. RSInvariant /\ ~rsStopped /\ P /\ RSFinish => Q'
    BY SMT DEF P, Q, RSFinish
<1>4. RSInvariant /\ ~rsStopped /\ P => ENABLED <<RSFinish>>_rsVars
    BY ExpandENABLED, RSLegalInputs, SMT DEF P, RSFinish, rsVars, RSInvariant, RSType, RSUniqueOwner, RSThread, RSParkedSignal, RSNoLostWake, RSFinishHint, RSUnnotified, RSPhases, RSProducerPhases
<1> QED BY RSServiceInvariant, <1>2, <1>3, <1>4, PTL DEF RSServiceSpec, RSFairness, P, Q

THEOREM RSServiceProgressProof == RSServiceSpec => RSServiceProgress
<1>1. RSInvariant /\ ~rsStopped /\ rsClaimed # {} /\ rsQueue > 0 =>
    (rsPhase = "awake" \/ rsPhase = "drain" \/ rsPhase = "finish" \/ rsPhase = "wait" \/ rsPhase = "park")
    BY SMT DEF RSInvariant, RSType, RSUniqueOwner, RSThread, RSParkedSignal, RSNoLostWake, RSFinishHint, RSUnnotified, RSPhases, RSProducerPhases
<1>2. rsQueue > 0 /\ rsPhase \in {"wait", "park"} =>
    (rsQueue > 0 /\ rsPhase \in {"wait", "park"} /\ ~rsPending /\ ~rsWake) \/
    (rsQueue > 0 /\ rsPhase \in {"wait", "park"} /\ (rsPending \/ rsWake))
    BY SMT
<1>3. (rsPhase = "wait" \/ rsPhase = "park") => rsPhase \in {"wait", "park"}
    BY SMT
<1> QED BY <1>1, <1>2, <1>3, RSServiceInvariant, RSCallbackProgress, RSWakeProgress,
    RSBeginProgress, RSFinishToWait, PTL DEF RSServiceProgress

THEOREM RSHintProgressProof == RSServiceSpec => RSHintProgress
<1>1. RSInvariant /\ ~rsStopped /\ rsClaimed # {} =>
    (rsPhase = "awake" \/ rsPhase = "drain" \/ rsPhase = "finish" \/ rsPhase \in {"wait", "park"})
    BY SMT DEF RSInvariant, RSType, RSUniqueOwner, RSThread, RSParkedSignal, RSNoLostWake, RSFinishHint, RSUnnotified, RSPhases, RSProducerPhases
<1>2. rsPhase = "wait" => rsPhase \in {"wait", "park"}
    BY SMT
<1> QED BY <1>1, <1>2, RSServiceInvariant, RSFinishHintToWait, RSWakeProgress,
    RSBeginProgress, PTL DEF RSHintProgress

THEOREM RSFinishProgressProof == RSServiceSpec => RSFinishProgress
BY RSDrainProgress DEF RSFinishProgress

THEOREM RSStoppedInvariant == RSStopSpec => [](RSInvariant /\ rsStopped /\ rsClaimed # {})
<1>1. RSStopNext => RSNext BY SMT DEF RSStopNext, RSNext, RSWorkNext
<1>2. RSInvariant /\ rsStopped /\ rsClaimed # {} /\ [RSStopNext]_rsVars =>
    (RSInvariant /\ rsStopped /\ rsClaimed # {})'
    BY <1>1, RSInvariantStep, SMT DEF RSStopNext, RSWake, RSExit, RSQuiesce, rsVars
<1> QED BY <1>2, PTL DEF RSStopSpec

THEOREM RSStopWakeProgress == RSStopSpec => ((rsPhase = "park") ~> (rsPhase = "awake"))
<1>1. RSInvariant /\ rsStopped /\ rsPhase = "park" /\ [RSStopNext]_rsVars =>
    RSInvariant' /\ ((rsPhase = "park")' \/ (rsPhase = "awake")')
    BY RSInvariantStep, SMT DEF RSNext, RSWorkNext, RSStopNext, RSWake, RSExit, RSQuiesce, rsVars
<1>2. rsPhase = "park" /\ RSWake => (rsPhase = "awake")'
    BY SMT DEF RSWake
<1>3. RSInvariant /\ rsStopped /\ rsPhase = "park" => ENABLED <<RSWake>>_rsVars
    BY ExpandENABLED, SMT DEF RSWake, rsVars, RSInvariant, RSType, RSUniqueOwner, RSThread, RSParkedSignal, RSNoLostWake, RSFinishHint, RSUnnotified, RSPhases, RSProducerPhases
<1> QED BY RSStoppedInvariant, <1>1, <1>2, <1>3, PTL DEF RSStopSpec

THEOREM RSExitProgress == RSStopSpec => ((rsPhase \in {"awake", "drain", "finish", "wait"}) ~> (rsPhase = "halted"))
<1>1. DEFINE P == rsPhase \in {"awake", "drain", "finish", "wait"}
<1>2. RSInvariant /\ rsStopped /\ P /\ [RSStopNext]_rsVars =>
    RSInvariant' /\ (P' \/ (rsPhase = "halted")')
    BY RSInvariantStep, SMT DEF P, RSNext, RSWorkNext, RSStopNext, RSWake, RSExit, RSQuiesce, rsVars
<1>3. P /\ RSExit(FALSE) => (rsPhase = "halted")'
    BY SMT DEF RSExit
<1>4. RSInvariant /\ rsStopped /\ P => ENABLED <<RSExit(FALSE)>>_rsVars
    BY ExpandENABLED, SMT DEF P, RSExit, rsVars, RSInvariant, RSType, RSUniqueOwner, RSThread, RSParkedSignal, RSNoLostWake, RSFinishHint, RSUnnotified, RSPhases, RSProducerPhases
<1> QED BY RSStoppedInvariant, <1>2, <1>3, <1>4, PTL DEF RSStopSpec, P

THEOREM RSStopProgressProof == RSStopSpec => RSStopProgress
<1>1. RSInvariant /\ rsClaimed # {} =>
    (rsPhase \in {"awake", "drain", "finish", "wait"} \/ rsPhase = "park" \/ rsPhase = "halted")
    BY SMT DEF RSInvariant, RSType, RSUniqueOwner, RSThread, RSParkedSignal, RSNoLostWake, RSFinishHint, RSUnnotified, RSPhases, RSProducerPhases
<1>2. rsPhase = "awake" => rsPhase \in {"awake", "drain", "finish", "wait"}
    BY SMT
<1> QED BY <1>1, <1>2, RSStoppedInvariant, RSStopWakeProgress, RSExitProgress, PTL DEF RSStopProgress
=============================================================================
