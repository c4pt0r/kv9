------------------------- MODULE OwnerPollProof -------------------------
EXTENDS OwnerPoll, RaftScheduleProof

THEOREM OPLocalStutters == OPLocalNext => UNCHANGED rsVars
BY SMT DEF OPLocalNext, OPSpin, OPHintReady

THEOREM OPNextRefines == [OPNext]_opVars => [RSNext]_rsVars
BY OPLocalStutters, SMT DEF OPNext, opVars

THEOREM OPWorkRefines == [OPWorkNext]_opVars => [RSWorkNext]_rsVars
BY OPLocalStutters, SMT DEF OPWorkNext, opVars

THEOREM OPInitType == OPLegalInputs /\ OPInit => OPType
BY SMT DEF OPLegalInputs, OPInit, OPType, RSInit

THEOREM OPTypeStep == ASSUME OPLegalInputs, OPType, [OPNext]_opVars
    PROVE OPType'
BY SMT DEF OPType, OPNext, OPTransfer, OPLocalNext, OPSpin, OPHintReady,
    OPLegalInputs, opVars, rsVars

THEOREM OPTypeAlways == OPSpec => []OPType
<1>1. OPLegalInputs /\ OPInit => OPLegalInputs /\ OPType
    BY OPInitType, SMT
<1>2. OPLegalInputs /\ OPType /\ [OPNext]_opVars =>
    (OPLegalInputs /\ OPType)'
    BY OPTypeStep, SMT DEF OPLegalInputs
<1> QED BY <1>1, <1>2, PTL DEF OPSpec

THEOREM OPSpecRefines == OPSpec => RSSpec
BY OPNextRefines, PTL DEF OPSpec, OPInit, RSSpec

THEOREM OPInvariantAlways == OPSpec => []OPInvariant
BY OPSpecRefines, RSInvariantAlways, OPTypeAlways, PTL DEF OPInvariant

THEOREM OPTerminalAlways == OPSpec => RSStopAlways
BY OPSpecRefines, RSTerminalAlways, PTL

THEOREM OPStrictVariant == ASSUME OPType, OPLocalNext
    PROVE opLeft' < opLeft /\ opLeft' \in Nat
BY SMT DEF OPType, OPLocalNext, OPSpin, OPHintReady

THEOREM OPAtomicPark == ASSUME RSInvariant, OPNext,
    rsPhase = "wait", rsPhase' = "park"
    PROVE ~rsPending /\ ~rsStopped /\ opLeft = 0
BY SMT DEF OPNext, OPTransfer, OPLocalNext, OPSpin, OPHintReady,
    RSNext, RSWorkNext, RSStartPublish, RSPublish, RSRefuse, RSNotify,
    RSHint, RSSpawn, RSBegin, RSDrain, RSFinish, RSPark, RSWake,
    RSQuiesce, RSTimeout, RSStop, RSExit, rsVars

THEOREM OPServiceRefines == OPServiceSpec => RSServiceSpec
BY OPWorkRefines, PTL DEF OPServiceSpec, OPInvariant, RSServiceSpec

THEOREM OPServiceProgress == OPServiceSpec => RSServiceProgress
BY OPServiceRefines, RSServiceProgressProof, PTL

THEOREM OPHintProgress == OPServiceSpec => RSHintProgress
BY OPServiceRefines, RSHintProgressProof, PTL
=============================================================================
