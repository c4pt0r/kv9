------------------------- MODULE OwnerPollMC -------------------------
EXTENDS OwnerPoll
OPServiceInit ==
    /\ rsQueue = RSMaxQueue
    /\ rsProducer = [p \in RSProducers |-> IF p = 1 THEN "published" ELSE "idle"]
    /\ rsPending = FALSE /\ rsStopped = FALSE /\ rsPhase = "park"
    /\ rsRedo = FALSE /\ rsWake = FALSE /\ rsClaimed = {1}
    /\ opLeft = 0 /\ opHint = FALSE
OPServiceMC == OPLegalInputs /\ OPServiceInit /\ [][OPWorkNext]_opVars
    /\ RSFairness /\ WF_opVars(OPSpin)
OPVariantAlways == [][OPVariant /\ OPNoFalseAuthority]_opVars
=============================================================================
