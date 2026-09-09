----------------------------- MODULE ClientMC -----------------------------
EXTENDS ClientRetry
CRNoRetriedSuccess == ~(crSent > 1 /\ crPhase = "terminal" /\ crApplied # {})
CRNoLateEffect == ~(crPhase = "unknown" /\ crApplied = {} /\ ENABLED (\E i \in 1..CRMaxAttempts : CREffect(i)))
=============================================================================
