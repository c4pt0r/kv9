------------------------- MODULE CoalescedWake -------------------------
EXTENDS RaftSchedule

\* A notification already pending under the signal mutex must not issue another
\* physical wake. Wake delivery is modeled separately from the pending bit.
CWDelivered == rsWake \/ (~rsStopped /\ ~rsPending /\ rsPhase = "park")
CWNotify(p) ==
    /\ p \in RSProducers /\ rsProducer[p] = "published"
    /\ rsProducer' = [rsProducer EXCEPT ![p] = "idle"]
    /\ rsPending' = (IF ~rsStopped /\ ~rsPending THEN TRUE ELSE rsPending)
    /\ rsWake' = CWDelivered
    /\ UNCHANGED <<rsQueue, rsStopped, rsPhase, rsRedo, rsClaimed>>
CWHint ==
    /\ rsPending' = (IF ~rsStopped /\ ~rsPending THEN TRUE ELSE rsPending)
    /\ rsWake' = CWDelivered
    /\ UNCHANGED <<rsQueue, rsProducer, rsStopped, rsPhase, rsRedo, rsClaimed>>
CWWorkNext == (\E p \in RSProducers : RSStartPublish(p) \/ RSPublish(p) \/ RSRefuse(p) \/ CWNotify(p))
    \/ CWHint \/ RSBegin \/ (\E k \in 0..RSDrainMax : RSDrain(k)) \/ RSFinish \/ RSPark \/ RSWake \/ RSQuiesce
CWNext == CWWorkNext \/ (\E o \in RSOwners : RSSpawn(o)) \/ RSTimeout \/ RSStop
    \/ (\E clear \in BOOLEAN : RSExit(clear))
CWSpec == RSInit /\ [][CWNext]_rsVars
\* Fairness remains the original semantic producer/owner-service premise.
\* The proof equates CWNotify with RSNotify on every invariant state; no new
\* fairness, timeout, repeated-notification or scheduler-timing assumption is added.
CWServiceSpec == RSInvariant /\ ~rsStopped /\ rsClaimed # {}
    /\ [][CWWorkNext]_rsVars /\ RSFairness
=============================================================================
