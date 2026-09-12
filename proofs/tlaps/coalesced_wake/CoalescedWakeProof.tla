------------------------- MODULE CoalescedWakeProof -------------------------
EXTENDS CoalescedWake, RaftScheduleProof

THEOREM CWWakeEquivalent == ASSUME rsPending \in BOOLEAN,
    rsStopped \in BOOLEAN, rsWake \in BOOLEAN, RSParkedSignal
    PROVE CWDelivered = (rsWake \/ (~rsStopped /\ rsPhase = "park"))
BY SMT DEF CWDelivered, RSParkedSignal

THEOREM CWPendingEquivalent == ASSUME rsPending \in BOOLEAN,
    rsStopped \in BOOLEAN PROVE
    (IF ~rsStopped /\ ~rsPending THEN TRUE ELSE rsPending) =
    (IF rsStopped THEN rsPending ELSE TRUE)
BY SMT

THEOREM CWNotifyEquivalent == ASSUME RSInvariant, NEW p
    PROVE CWNotify(p) <=> RSNotify(p)
BY CWWakeEquivalent, CWPendingEquivalent, SMT DEF CWNotify, RSNotify,
    RSInvariant, RSType, RSUniqueOwner, RSThread, RSParkedSignal,
    RSNoLostWake, RSFinishHint, RSUnnotified, RSPhases, RSProducerPhases

THEOREM CWHintEquivalent == ASSUME RSInvariant PROVE CWHint <=> RSHint
BY CWWakeEquivalent, CWPendingEquivalent, SMT DEF CWHint, RSHint,
    RSInvariant, RSType, RSUniqueOwner, RSThread, RSParkedSignal,
    RSNoLostWake, RSFinishHint, RSUnnotified, RSPhases, RSProducerPhases

THEOREM CWWorkNextEquivalent == ASSUME RSInvariant
    PROVE CWWorkNext <=> RSWorkNext
BY CWNotifyEquivalent, CWHintEquivalent, SMT DEF CWWorkNext, RSWorkNext

THEOREM CWNextEquivalent == ASSUME RSInvariant PROVE CWNext <=> RSNext
BY CWWorkNextEquivalent, SMT DEF CWNext, RSNext

THEOREM CWInvariantStep == ASSUME RSInvariant, CWNext PROVE RSInvariant'
BY CWNextEquivalent, RSInvariantStep, SMT

THEOREM CWInvariantAlways == CWSpec => []RSInvariant
<1>1. RSInvariant /\ [CWNext]_rsVars => RSInvariant'
    BY CWNextEquivalent, RSInvariantStep, SMT DEF RSNext, rsVars
<1> QED BY RSInvariantInit, <1>1, PTL DEF CWSpec

THEOREM CWSpecRefines == CWSpec => RSSpec
<1>1. RSInvariant /\ [CWNext]_rsVars => [RSNext]_rsVars
    BY CWNextEquivalent, SMT DEF rsVars
<1> QED BY <1>1, CWInvariantAlways, PTL DEF CWSpec, RSSpec

THEOREM CWTerminalAlways == CWSpec => RSStopAlways
BY CWSpecRefines, RSTerminalAlways, PTL

THEOREM CWServiceInvariant == CWServiceSpec => []RSInvariant
<1>1. RSInvariant /\ [CWWorkNext]_rsVars => RSInvariant'
    BY CWWorkNextEquivalent, RSInvariantStep, SMT DEF RSNext, rsVars
<1> QED BY <1>1, PTL DEF CWServiceSpec

THEOREM CWServiceRefines == CWServiceSpec => RSServiceSpec
<1>1. RSInvariant /\ [CWWorkNext]_rsVars => [RSWorkNext]_rsVars
    BY CWWorkNextEquivalent, SMT DEF rsVars
<1> QED BY <1>1, CWServiceInvariant, PTL DEF CWServiceSpec, RSServiceSpec

THEOREM CWServiceProgress == CWServiceSpec => RSServiceProgress
BY CWServiceRefines, RSServiceProgressProof, PTL

THEOREM CWHintProgress == CWServiceSpec => RSHintProgress
BY CWServiceRefines, RSHintProgressProof, PTL
=============================================================================
