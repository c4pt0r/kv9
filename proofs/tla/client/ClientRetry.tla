--------------------------- MODULE ClientRetry ---------------------------
EXTENDS Naturals
CONSTANTS CRMaxAttempts, CRBudget
ASSUME CRLegalInputs == CRMaxAttempts \in Nat \ {0} /\ CRBudget \in Nat \ {0}

\* One immutable logical point write, not an idempotency-key protocol. Each
\* dispatched RPC can have at most one server effect (the underlying server
\* contract). A trusted refusal excludes both past AND future effects from that
\* attempt. Lost responses/timeouts do not exclude late effects.
VARIABLES crSent, crClosed, crApplied, crPhase, crNow, crDeadline
crVars == <<crSent, crClosed, crApplied, crPhase, crNow, crDeadline>>
CRInit == /\ crSent = 0 /\ crClosed = 0 /\ crApplied = {}
          /\ crPhase = "ready" /\ crNow = 0 /\ crDeadline = CRBudget

CRDispatch ==
    /\ crPhase = "ready" /\ crSent = crClosed
    /\ crSent < CRMaxAttempts /\ crNow < crDeadline
    /\ crSent' = crSent + 1 /\ crPhase' = "pending"
    /\ UNCHANGED <<crClosed, crApplied, crNow, crDeadline>>
CRRefuse ==
    /\ crPhase = "pending" /\ crApplied = {}
    /\ crClosed' = crSent /\ crPhase' = "ready"
    /\ UNCHANGED <<crSent, crApplied, crNow, crDeadline>>
CREffect(i) ==
    /\ i \in (crClosed + 1)..crSent /\ i \notin crApplied
    /\ crApplied' = crApplied \cup {i}
    /\ UNCHANGED <<crSent, crClosed, crPhase, crNow, crDeadline>>
CRSuccess ==
    /\ crPhase = "pending" /\ crSent \in crApplied
    /\ crPhase' = "terminal"
    /\ UNCHANGED <<crSent, crClosed, crApplied, crNow, crDeadline>>
CRUnknown ==
    /\ crPhase = "pending" /\ crPhase' = "unknown"
    /\ UNCHANGED <<crSent, crClosed, crApplied, crNow, crDeadline>>
CRStop ==
    /\ crPhase = "ready" /\ (crNow >= crDeadline \/ crSent = CRMaxAttempts)
    /\ crPhase' = "terminal"
    /\ UNCHANGED <<crSent, crClosed, crApplied, crNow, crDeadline>>
CRTick ==
    /\ crNow < 2 * CRBudget /\ crNow' = crNow + 1
    /\ UNCHANGED <<crSent, crClosed, crApplied, crPhase, crDeadline>>
CRQuiesce == crPhase \in {"terminal", "unknown"} /\ UNCHANGED crVars
CRNext == CRDispatch \/ CRRefuse \/ (\E i \in 1..CRMaxAttempts : CREffect(i))
          \/ CRSuccess \/ CRUnknown \/ CRStop \/ CRTick \/ CRQuiesce
CRSpec == CRInit /\ [][CRNext]_crVars

CRType == /\ CRMaxAttempts \in Nat \ {0} /\ CRBudget \in Nat \ {0}
          /\ crSent \in 0..CRMaxAttempts /\ crClosed \in 0..crSent
          /\ crApplied \subseteq 1..crSent
          /\ crNow \in 0..(2 * CRBudget) /\ crDeadline \in Nat
          /\ crPhase \in {"ready", "pending", "terminal", "unknown"}
CRSinglePotential == crSent <= crClosed + 1
CRClosedExcludesEffects == crApplied \subseteq (crClosed + 1)..crSent
CRReady == crPhase = "ready" => crSent = crClosed
CRFixedBudget == crDeadline = CRBudget
CRInvariant == CRType /\ CRSinglePotential /\ CRClosedExcludesEffects /\ CRReady /\ CRFixedBudget
CRAtMostOneEffect == \A a, b \in crApplied : a = b
CRTerminalStep == crPhase \in {"terminal", "unknown"} => crPhase' = crPhase /\ crSent' = crSent
CRTerminalFreeze == [][CRTerminalStep]_crVars
CRDispatchBudgetStep == crSent' > crSent => crNow < CRBudget
CRDispatchBudgetSafety == [][CRDispatchBudgetStep]_crVars
=============================================================================
