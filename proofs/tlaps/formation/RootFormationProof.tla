------------------------ MODULE RootFormationProof ------------------------
EXTENDS RootFormation, TLAPS
THEOREM RFInvariantInit == RFInit => RFInvariant
BY RFLegalInputs, SMT DEF RFInit, RFInvariant, RFType, RFHistory, RFAuthority, RFCuts, RFFreshPlan
THEOREM RFInvariantStep == ASSUME RFInvariant, [RFNext]_rfVars PROVE RFInvariant'
<1>1. \A local \in RFRoots, owned \in BOOLEAN : RFRecover(local, owned) => RFInvariant'
    BY RFLegalInputs, SMT DEF RFRecover, RFInvariant, RFType, RFHistory, RFAuthority, RFCuts, RFFreshPlan
<1>2. \A keep \in {rfCommitted, rfLog} : RFElect(keep) => RFInvariant'
    BY RFLegalInputs, SMT DEF RFElect, RFInvariant, RFType, RFHistory, RFAuthority, RFCuts, RFFreshPlan
<1>3. RFDrain \/ RFPlan \/ RFAppend => RFInvariant'
    BY RFLegalInputs, SMT DEF RFDrain, RFPlan, RFAppend, RFInvariant, RFType, RFHistory, RFAuthority, RFCuts, RFFreshPlan
<1>4. RFCrash \/ RFCommit \/ RFMark \/ RFWriteUser => RFInvariant'
    BY RFLegalInputs, SMT DEF RFCrash, RFCommit, RFMark, RFWriteUser, RFInvariant, RFType, RFHistory, RFAuthority, RFCuts, RFFreshPlan
<1>5. UNCHANGED rfVars => RFInvariant'
    BY SMT DEF rfVars, RFInvariant, RFType, RFHistory, RFAuthority, RFCuts, RFFreshPlan
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, SMT DEF RFNext, RFElectAny, RFQuiesce
THEOREM RFInvariantAlways == RFSpec => []RFInvariant
BY RFInvariantInit, RFInvariantStep, PTL DEF RFSpec
THEOREM RFStepSafetyProof == ASSUME RFInvariant, RFNext PROVE RFStepSafety
BY RFLegalInputs, SMT DEF RFNext, RFElectAny, RFStepSafety, RFAppendFresh, RFUserPreserved,
   RFRecover, RFElect, RFDrain, RFPlan, RFAppend, RFCrash, RFCommit, RFMark, RFWriteUser, RFQuiesce,
   rfVars, RFInvariant, RFType, RFHistory, RFAuthority, RFCuts, RFFreshPlan
THEOREM RFSafetyProof == RFSpec => RFSafety
<1>1. RFInvariant /\ [RFNext]_rfVars => [RFStepSafety]_rfVars
    BY RFStepSafetyProof, SMT DEF rfVars, RFStepSafety, RFAppendFresh, RFUserPreserved, RFAppend
<1> QED BY RFInvariantAlways, <1>1, PTL DEF RFSpec, RFSafety

THEOREM RFStableInvariant == RFFairSpec => []RFInvariant
<1>1. RFStableNext => RFNext BY RFLegalInputs, SMT DEF RFStableNext, RFNext, RFRestore
<1>2. RFInvariant /\ [RFStableNext]_rfVars => RFInvariant'
    BY <1>1, RFInvariantStep, SMT
<1> QED BY <1>2, PTL DEF RFFairSpec, RFCrashed

THEOREM RFRecoveryProgress == RFFairSpec => (RFCrashed ~> RFReady)
<1>1. RFCrashed /\ [RFStableNext]_rfVars => RFCrashed' \/ RFReady'
    BY RFLegalInputs, RFInvariantStep, SMT DEF RFCrashed, RFReady, RFRestore, RFStableNext, RFNext, RFRecover, RFDrain, RFPlan, RFAppend,
       RFCommit, RFMark, RFWriteUser, RFQuiesce, rfVars,
       RFInvariant, RFType, RFHistory, RFAuthority, RFCuts, RFFreshPlan
<1>2. RFCrashed /\ RFRestore => RFReady'
    BY RFLegalInputs, RFInvariantStep, SMT DEF RFCrashed, RFReady, RFRestore, RFStableNext, RFNext, RFRecover, RFDrain, RFPlan, RFAppend,
       RFCommit, RFMark, RFWriteUser, RFQuiesce, rfVars,
       RFInvariant, RFType, RFHistory, RFAuthority, RFCuts, RFFreshPlan
<1>3. RFCrashed => ENABLED <<RFRestore>>_rfVars
    BY RFLegalInputs, ExpandENABLED, SMT DEF RFCrashed, RFInvariant, RFType, RFRestore, RFRecover, rfVars
<1> QED BY <1>1, <1>2, <1>3, PTL DEF RFFairSpec

THEOREM RFReadyStable == RFReady /\ [RFStableNext]_rfVars => RFReady'
BY RFLegalInputs, RFInvariantStep, SMT DEF RFReady, RFRestore, RFStableNext, RFNext, RFRecover, RFDrain, RFPlan, RFAppend,
       RFCommit, RFMark, RFWriteUser, RFQuiesce, rfVars,
       RFInvariant, RFType, RFHistory, RFAuthority, RFCuts, RFFreshPlan

THEOREM RFBarrierProgress == RFFairSpec => (RFReady ~> RFBarrierReady)
<1>1. RFReady /\ RFDrain => RFBarrierReady'
    BY RFReadyStable, SMT DEF RFBarrierReady, RFDrain, RFStableNext, rfVars
<1>2. RFReady /\ ~rfBarrier => ENABLED <<RFDrain>>_rfVars
    BY ExpandENABLED, SMT DEF RFReady, RFDrain, rfVars
<1> QED BY RFReadyStable, <1>1, <1>2, PTL DEF RFFairSpec, RFBarrierReady

THEOREM RFPlanningProgress == RFFairSpec => (RFBarrierReady ~> RFSeedReady)
<1>1. RFBarrierReady /\ [RFStableNext]_rfVars => RFBarrierReady'
    BY RFLegalInputs, RFReadyStable, SMT DEF RFBarrierReady, RFReady, RFRestore, RFStableNext, RFNext, RFRecover, RFDrain, RFPlan, RFAppend,
       RFCommit, RFMark, RFWriteUser, RFQuiesce, rfVars,
       RFInvariant, RFType, RFHistory, RFAuthority, RFCuts, RFFreshPlan
<1>2. RFBarrierReady /\ RFPlan => RFSeedReady'
    BY <1>1, SMT DEF RFSeedReady, RFPlan, RFStableNext, rfVars
<1>3. RFBarrierReady /\ ~RFSeedReady => ENABLED <<RFPlan>>_rfVars
    BY RFLegalInputs, ExpandENABLED, SMT DEF RFSeedReady, RFBarrierReady, RFReady, RFPlan,
       RFInvariant, RFType, RFHistory, RFAuthority, RFCuts, RFFreshPlan, rfVars
<1> QED BY <1>1, <1>2, <1>3, PTL DEF RFFairSpec, RFSeedReady

THEOREM RFAppendProgress == RFFairSpec => (RFSeedReady ~> RFLogged)
<1>1. RFSeedReady /\ [RFStableNext]_rfVars => RFSeedReady'
    BY RFLegalInputs, RFReadyStable, SMT DEF RFSeedReady, RFBarrierReady, RFReady, RFRestore, RFStableNext, RFNext, RFRecover, RFDrain, RFPlan, RFAppend,
       RFCommit, RFMark, RFWriteUser, RFQuiesce, rfVars,
       RFInvariant, RFType, RFHistory, RFAuthority, RFCuts, RFFreshPlan
<1>2. RFSeedReady /\ RFAppend => RFLogged'
    BY <1>1, RFLegalInputs, SMT DEF RFLogged, RFSeedReady, RFBarrierReady, RFReady, RFAppend,
       RFInvariant, RFType, RFAuthority, RFStableNext, rfVars
<1>3. RFSeedReady /\ ~RFLogged => ENABLED <<RFAppend>>_rfVars
    BY RFLegalInputs, ExpandENABLED, SMT DEF RFLogged, RFSeedReady, RFBarrierReady, RFReady, RFAppend,
       RFInvariant, RFType, RFHistory, RFAuthority, RFCuts, RFFreshPlan, rfVars
<1> QED BY <1>1, <1>2, <1>3, PTL DEF RFFairSpec, RFLogged, RFSeedReady, RFBarrierReady

THEOREM RFApplyProgress == RFFairSpec => (RFLogged ~> RFApplied)
<1>1. RFLogged /\ [RFStableNext]_rfVars => RFLogged'
    BY RFLegalInputs, RFReadyStable, SMT DEF RFLogged, RFReady, RFRestore, RFStableNext, RFNext, RFRecover, RFDrain, RFPlan, RFAppend,
       RFCommit, RFMark, RFWriteUser, RFQuiesce, rfVars,
       RFInvariant, RFType, RFHistory, RFAuthority, RFCuts, RFFreshPlan
<1>2. RFLogged /\ RFDrain => RFApplied'
    BY <1>1, SMT DEF RFApplied, RFLogged, RFDrain, RFStableNext, rfVars
<1>3. RFLogged /\ ~RFApplied => ENABLED <<RFDrain>>_rfVars
    BY RFLegalInputs, ExpandENABLED, SMT DEF RFApplied, RFLogged, RFReady, RFDrain,
       RFInvariant, RFType, RFHistory, RFAuthority, RFCuts, RFFreshPlan, rfVars
<1> QED BY <1>1, <1>2, <1>3, PTL DEF RFFairSpec, RFApplied

THEOREM RFServingProgress == RFFairSpec => (RFApplied ~> RFFinished)
<1>1. RFApplied /\ [RFStableNext]_rfVars => RFApplied'
    BY RFLegalInputs, RFReadyStable, SMT DEF RFApplied, RFLogged, RFReady, RFRestore, RFStableNext, RFNext, RFRecover, RFDrain, RFPlan, RFAppend,
       RFCommit, RFMark, RFWriteUser, RFQuiesce, rfVars,
       RFInvariant, RFType, RFHistory, RFAuthority, RFCuts, RFFreshPlan
<1>2. RFApplied /\ RFMark => RFFinished'
    BY <1>1, SMT DEF RFFinished, RFMark, RFStableNext, rfVars
<1>3. RFApplied /\ ~rfMarker => ENABLED <<RFMark>>_rfVars
    BY ExpandENABLED, SMT DEF RFApplied, RFLogged, RFReady, RFMark, rfVars
<1> QED BY <1>1, <1>2, <1>3, PTL DEF RFFairSpec, RFFinished

THEOREM RFProgressProof == RFFairSpec => RFProgress
BY RFStableInvariant, RFRecoveryProgress, RFBarrierProgress, RFPlanningProgress,
   RFAppendProgress, RFApplyProgress, RFServingProgress, PTL
   DEF RFProgress, RFFairSpec, RFServing, RFFinished, RFApplied, RFLogged, RFReady
=============================================================================
