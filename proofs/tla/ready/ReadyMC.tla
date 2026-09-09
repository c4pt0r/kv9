----------------------------- MODULE ReadyMC -----------------------------
EXTENDS ReadyPublication
RPNoLateCommit == rpPhase # "light"
RPNoAckedRestart == ~(rpPhase = "idle" /\ rpAcknowledged > 0 /\ ~rpPublished)
=============================================================================
