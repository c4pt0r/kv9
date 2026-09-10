------------------------- MODULE RaftScheduleMC -------------------------
EXTENDS RaftSchedule
RSServiceInit ==
    /\ rsQueue = RSMaxQueue
    /\ rsProducer = [p \in RSProducers |-> IF p = 1 THEN "published" ELSE "idle"]
    /\ rsPending = FALSE /\ rsStopped = FALSE /\ rsPhase = "park"
    /\ rsRedo = FALSE /\ rsWake = FALSE /\ rsClaimed = {1}
RSServiceMC == RSServiceInit /\ [][RSWorkNext]_rsVars /\ RSFairness
RSStoppedInit ==
    /\ rsQueue = 0 /\ rsProducer = [p \in RSProducers |-> "idle"]
    /\ rsPending = FALSE /\ rsStopped = TRUE /\ rsPhase = "park"
    /\ rsRedo = FALSE /\ rsWake = TRUE /\ rsClaimed = {1}
RSStopMC == RSStoppedInit /\ [][RSStopNext]_rsVars
    /\ WF_rsVars(RSWake) /\ WF_rsVars(RSExit(FALSE))
=============================================================================
