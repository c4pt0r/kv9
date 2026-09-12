------------------------- MODULE OwnerPoll -------------------------
EXTENDS RaftSchedule
CONSTANT OPPollSteps
VARIABLES opLeft, opHint
opVars == <<rsVars, opLeft, opHint>>
OPLegalInputs == OPPollSteps \in Nat
OPType == opLeft \in 0..OPPollSteps /\ opHint \in BOOLEAN
    /\ (rsPhase # "wait" => opLeft = 0)
OPInit == RSInit /\ opLeft = 0 /\ opHint = FALSE

\* The hint is deliberately allowed to be stale in either direction. It only
\* shortens optional polling; the original mutex predicate authorizes parking.
OPTransfer ==
    /\ (rsPhase = "wait" /\ rsPhase' = "park" => opLeft = 0)
    /\ opLeft' = IF rsPhase # "wait" /\ rsPhase' = "wait" THEN OPPollSteps
                  ELSE IF rsPhase' # "wait" THEN 0 ELSE opLeft
    /\ opHint' \in BOOLEAN
OPSpin ==
    /\ rsPhase = "wait" /\ opLeft > 0
    /\ opLeft' = opLeft - 1
    /\ UNCHANGED <<rsVars, opHint>>
OPHintReady ==
    /\ rsPhase = "wait" /\ opLeft > 0 /\ opHint
    /\ opLeft' = 0
    /\ UNCHANGED <<rsVars, opHint>>
OPLocalNext == OPSpin \/ OPHintReady
OPNext == (RSNext /\ OPTransfer) \/ OPLocalNext
OPWorkNext == (RSWorkNext /\ OPTransfer) \/ OPLocalNext
OPSpec == OPLegalInputs /\ OPInit /\ [][OPNext]_opVars
OPInvariant == RSInvariant /\ OPType

\* Service fairness remains the original abstract owner/producer fairness.
\* OPSpin models advancement of the bounded local monotonic time budget, not
\* one CPU instruction. Repeated equal clock readings are stuttering steps.
\* No wall-clock scheduling bound or clock-derived consensus lease is assumed.
OPServiceSpec == OPLegalInputs /\ OPInvariant /\ ~rsStopped /\ rsClaimed # {}
    /\ [][OPWorkNext]_opVars /\ RSFairness /\ WF_opVars(OPSpin)
OPVariant == OPLocalNext => opLeft' < opLeft
OPNoFalseAuthority == OPLocalNext => UNCHANGED rsVars
OPNoPollWitness == opLeft = 0
OPNoStaleTrueWitness == ~(opHint /\ ~rsPending /\ ~rsStopped)
=============================================================================
