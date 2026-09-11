------------------------- MODULE OwnerReadPump -------------------------
EXTENDS Naturals
\* A single owner transaction. External publishers may notify between any
\* two actions. "returned" and "failed" mean a normal or typed-error return
\* from the token helper, not successful completion of a client read.
\* Pump invocation does not imply quorum confirmation, persistence or apply.
VARIABLES orPhase, orAdmitted, orPumped, orExternal, orPending
orVars == <<orPhase, orAdmitted, orPumped, orExternal, orPending>>
ORInit ==
    /\ orPhase = "submit"
    /\ orAdmitted = FALSE /\ orPumped = FALSE
    /\ orExternal = FALSE /\ orPending = FALSE
ORNotify ==
    /\ orExternal' = TRUE /\ orPending' = TRUE
    /\ UNCHANGED <<orPhase, orAdmitted, orPumped>>
ORSubmit(ok) ==
    /\ orPhase = "submit" /\ ok \in BOOLEAN
    /\ orAdmitted' = ok /\ orPhase' = "pump"
    /\ UNCHANGED <<orPumped, orExternal, orPending>>
ORPump(ok) ==
    /\ orPhase = "pump" /\ ok \in BOOLEAN
    /\ orPumped' = TRUE
    /\ orPhase' = IF ok THEN "returned" ELSE "failed"
    /\ UNCHANGED <<orAdmitted, orExternal, orPending>>
\* An unwind/abort is deliberately outside the ordinary-return guarantee.
ORAbort ==
    /\ orPhase \in {"submit", "pump"}
    /\ orPhase' = "aborted"
    /\ UNCHANGED <<orAdmitted, orPumped, orExternal, orPending>>
ORNext == ORNotify \/ (\E ok \in BOOLEAN : ORSubmit(ok) \/ ORPump(ok)) \/ ORAbort
ORSpec == ORInit /\ [][ORNext]_orVars
ORType ==
    /\ orPhase \in {"submit", "pump", "returned", "failed", "aborted"}
    /\ orAdmitted \in BOOLEAN /\ orPumped \in BOOLEAN
    /\ orExternal \in BOOLEAN /\ orPending \in BOOLEAN
OROrder ==
    /\ (orPumped <=> orPhase \in {"returned", "failed"})
    /\ (orPhase = "submit" => ~orAdmitted)
ORNoLostNotification == orExternal => orPending
ORInvariant == ORType /\ OROrder /\ ORNoLostNotification
=============================================================================
