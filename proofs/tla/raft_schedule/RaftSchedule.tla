------------------------- MODULE RaftSchedule -------------------------
EXTENDS Naturals
CONSTANTS RSMaxQueue, RSDrainMax, RSProducers, RSOwners
ASSUME RSLegalInputs ==
    /\ RSMaxQueue \in Nat \ {0} /\ RSDrainMax \in 1..RSMaxQueue
    /\ RSProducers # {} /\ RSProducers \subseteq Nat \ {0}
    /\ RSOwners # {} /\ RSOwners \subseteq Nat \ {0}

\* One runtime-assembled driver/peer lifetime. Producer callbacks publish work
\* before notifying. Queued work is abstract service demand, not process memory
\* or consensus authority. Actual encoded-byte bounds are a separate boundary.
\* wake records delivery to an already parked waiter. Merely finding pending
\* after a non-atomic predicate/park gap does not manufacture a delivered wake.
VARIABLES rsQueue, rsProducer, rsPending, rsStopped, rsPhase, rsRedo, rsWake, rsClaimed
rsVars == <<rsQueue, rsProducer, rsPending, rsStopped, rsPhase, rsRedo, rsWake, rsClaimed>>
RSPhases == {"new", "awake", "drain", "finish", "wait", "park", "halted"}
RSProducerPhases == {"idle", "publishing", "published", "notified"}
RSUnnotified == \E p \in RSProducers : rsProducer[p] = "published"
RSInit ==
    /\ rsQueue = 0 /\ rsProducer = [p \in RSProducers |-> "idle"]
    /\ rsPending = FALSE /\ rsStopped = FALSE /\ rsPhase = "new"
    /\ rsRedo = FALSE /\ rsWake = FALSE /\ rsClaimed = {}

RSStartPublish(p) ==
    /\ p \in RSProducers /\ rsProducer[p] = "idle"
    /\ rsProducer' = [rsProducer EXCEPT ![p] = "publishing"]
    /\ UNCHANGED <<rsQueue, rsPending, rsStopped, rsPhase, rsRedo, rsWake, rsClaimed>>
RSPublish(p) ==
    /\ p \in RSProducers /\ rsProducer[p] = "publishing" /\ rsQueue < RSMaxQueue
    /\ rsProducer' = [rsProducer EXCEPT ![p] = "published"]
    /\ rsQueue' = rsQueue + 1
    /\ UNCHANGED <<rsPending, rsStopped, rsPhase, rsRedo, rsWake, rsClaimed>>
RSRefuse(p) ==
    /\ p \in RSProducers /\ rsProducer[p] = "publishing" /\ rsQueue = RSMaxQueue
    /\ rsProducer' = [rsProducer EXCEPT ![p] = "idle"]
    /\ UNCHANGED <<rsQueue, rsPending, rsStopped, rsPhase, rsRedo, rsWake, rsClaimed>>
RSNotify(p) ==
    /\ p \in RSProducers /\ rsProducer[p] = "published"
    /\ rsProducer' = [rsProducer EXCEPT ![p] = "idle"]
    /\ rsPending' = (IF rsStopped THEN rsPending ELSE TRUE)
    /\ rsWake' = (rsWake \/ (~rsStopped /\ rsPhase = "park"))
    /\ UNCHANGED <<rsQueue, rsStopped, rsPhase, rsRedo, rsClaimed>>
\* Notifications for work outside the tracked bounded inbox (local proposals,
\* successor Ready, assembly) can arrive without creating a tracked queue item.
RSHint ==
    /\ rsPending' = (IF rsStopped THEN rsPending ELSE TRUE)
    /\ rsWake' = (rsWake \/ (~rsStopped /\ rsPhase = "park"))
    /\ UNCHANGED <<rsQueue, rsProducer, rsStopped, rsPhase, rsRedo, rsClaimed>>
RSSpawn(o) ==
    /\ o \in RSOwners /\ rsClaimed = {}
    /\ rsClaimed' = rsClaimed \cup {o}
    /\ rsPhase' = "awake"
    /\ UNCHANGED <<rsQueue, rsProducer, rsPending, rsStopped, rsRedo, rsWake>>
RSBegin ==
    /\ rsPhase = "awake" /\ ~rsStopped
    /\ rsPending' = FALSE
    /\ rsPhase' = "drain"
    /\ UNCHANGED <<rsQueue, rsProducer, rsStopped, rsRedo, rsWake, rsClaimed>>
RSDrain(k) ==
    /\ rsPhase = "drain" /\ k \in 0..RSDrainMax /\ k <= rsQueue
    /\ (rsQueue > 0 => k > 0)
    /\ rsQueue' = rsQueue - k
    /\ rsRedo' = (rsQueue - k > 0)
    /\ rsPhase' = "finish"
    /\ UNCHANGED <<rsProducer, rsPending, rsStopped, rsWake, rsClaimed>>
RSFinish ==
    /\ rsPhase = "finish"
    /\ rsPending' = (IF rsRedo /\ ~rsStopped THEN TRUE ELSE rsPending)
    /\ rsRedo' = FALSE
    /\ rsPhase' = "wait"
    /\ UNCHANGED <<rsQueue, rsProducer, rsStopped, rsWake, rsClaimed>>
RSPark ==
    /\ rsPhase = "wait" /\ ~rsPending /\ ~rsStopped
    /\ rsPhase' = "park"
    /\ rsWake' = FALSE
    /\ UNCHANGED <<rsQueue, rsProducer, rsPending, rsStopped, rsRedo, rsClaimed>>
RSWake ==
    /\ ((rsPhase = "wait" /\ (rsPending \/ rsStopped)) \/ (rsPhase = "park" /\ rsWake))
    /\ rsPhase' = "awake"
    /\ rsWake' = FALSE
    /\ UNCHANGED <<rsQueue, rsProducer, rsPending, rsStopped, rsRedo, rsClaimed>>
\* A deadline expiry is supplied by the independent monotonic deadline proof.
\* It may wake an idle owner; notification-driven progress does not require it.
RSTimeout ==
    /\ rsPhase \in {"wait", "park"}
    /\ rsPhase' = "awake" /\ rsWake' = FALSE
    /\ UNCHANGED <<rsQueue, rsProducer, rsPending, rsStopped, rsRedo, rsClaimed>>
RSStop ==
    /\ rsStopped' = TRUE
    /\ rsWake' = (rsWake \/ (rsPhase = "park"))
    /\ UNCHANGED <<rsQueue, rsProducer, rsPending, rsPhase, rsRedo, rsClaimed>>
RSExit(clear) ==
    /\ rsStopped /\ rsPhase \in {"awake", "drain", "finish", "wait"} /\ clear \in BOOLEAN
    /\ rsPhase' = "halted" /\ rsRedo' = FALSE /\ rsWake' = FALSE
    /\ rsPending' = (IF clear THEN FALSE ELSE rsPending)
    /\ UNCHANGED <<rsQueue, rsProducer, rsStopped, rsClaimed>>
RSQuiesce == UNCHANGED rsVars
RSWorkNext == (\E p \in RSProducers : RSStartPublish(p) \/ RSPublish(p) \/ RSRefuse(p) \/ RSNotify(p))
    \/ RSHint \/ RSBegin \/ (\E k \in 0..RSDrainMax : RSDrain(k)) \/ RSFinish \/ RSPark \/ RSWake \/ RSQuiesce
RSNext == RSWorkNext \/ (\E o \in RSOwners : RSSpawn(o)) \/ RSTimeout \/ RSStop
    \/ (\E clear \in BOOLEAN : RSExit(clear))
RSSpec == RSInit /\ [][RSNext]_rsVars

RSType ==
    /\ rsQueue \in 0..RSMaxQueue /\ rsProducer \in [RSProducers -> RSProducerPhases]
    /\ rsPending \in BOOLEAN /\ rsStopped \in BOOLEAN /\ rsRedo \in BOOLEAN /\ rsWake \in BOOLEAN
    /\ rsPhase \in RSPhases /\ rsClaimed \subseteq RSOwners
RSUniqueOwner == \A a, b \in rsClaimed : a = b
RSThread ==
    /\ ((rsPhase = "new") <=> (rsClaimed = {}))
    /\ (rsPhase = "halted" => rsStopped)
    /\ (rsRedo => rsPhase = "finish") /\ (rsWake => rsPhase = "park")
RSParkedSignal == rsPhase = "park" /\ (rsPending \/ rsStopped) => rsWake
RSNoLostWake == rsQueue > 0 /\ ~rsStopped =>
    /\ (rsPhase = "wait" => rsPending \/ RSUnnotified)
    /\ (rsPhase = "park" => rsWake \/ RSUnnotified)
RSFinishHint == rsPhase = "finish" /\ rsQueue > 0 /\ ~rsStopped => rsRedo \/ rsPending \/ RSUnnotified
RSInvariant == RSType /\ RSUniqueOwner /\ RSThread /\ RSParkedSignal /\ RSNoLostWake /\ RSFinishHint
RSStopMonotonic == rsStopped => rsStopped'
RSHaltTerminal == rsPhase = "halted" => rsPhase' = "halted"
RSNoBeginAfterStop == (rsPhase # "drain" /\ rsPhase' = "drain") => ~rsStopped
RSStopAlways == [][RSStopMonotonic /\ RSHaltTerminal /\ RSNoBeginAfterStop]_rsVars
RSNoWorkWitness == rsQueue = 0
RSNoCoalescingWitness == ~rsPending
RSNoRetainedWitness == ~rsRedo
RSNoParkWitness == rsPhase # "park"
RSNoTerminalWitness == rsPhase # "halted"

RSNotifyAny == \E p \in RSProducers : RSNotify(p)
RSDrainAny == \E k \in 0..RSDrainMax : RSDrain(k)
RSFairness == WF_rsVars(RSNotifyAny) /\ WF_rsVars(RSBegin) /\ WF_rsVars(RSDrainAny)
    /\ WF_rsVars(RSFinish) /\ WF_rsVars(RSWake)
RSServiceSpec == RSInvariant /\ ~rsStopped /\ rsClaimed # {} /\ [][RSWorkNext]_rsVars /\ RSFairness
RSServiceProgress == rsQueue > 0 ~> rsPhase = "drain"
RSFinishProgress == rsPhase = "drain" ~> rsPhase = "finish"
RSHintProgress == rsPending ~> rsPhase = "drain"
RSStopNext == RSWake \/ (\E clear \in BOOLEAN : RSExit(clear)) \/ RSQuiesce
RSStopSpec == RSInvariant /\ rsStopped /\ rsClaimed # {} /\ [][RSStopNext]_rsVars
    /\ WF_rsVars(RSWake) /\ WF_rsVars(RSExit(FALSE))
RSStopProgress == <> (rsPhase = "halted")
=============================================================================
