----------------------- MODULE StoreLifecycleProof -----------------------
EXTENDS StoreLifecycle, TLAPS
THEOREM SLInvariantInit == SLInit => SLInvariant
BY SLLegalInputs, SMT DEF SLInit, SLInvariant, SLType, SLIdentity, SLHistory,
   SLNoRecreatedLog, SLPermission, SLHealthy

THEOREM SLInvariantStep == ASSUME SLInvariant, [SLNext]_slVars PROVE SLInvariant'
<1>1. \A i \in SLIncarnations : SLPrepare(i) => SLInvariant'
    BY SLLegalInputs, SMT DEF SLPrepare, SLInvariant, SLType, SLIdentity, SLHistory,
       SLNoRecreatedLog, SLPermission, SLHealthy
<1>2. SLRootBind \/ SLWriteBinding \/ SLPublish \/ SLCreateLog \/ SLSyncLog \/ SLWriteActivation \/ SLStart \/ SLCertify => SLInvariant'
    BY SLLegalInputs, SMT DEF SLRootBind, SLWriteBinding, SLPublish, SLCreateLog, SLSyncLog,
       SLWriteActivation, SLStart, SLCertify, SLInvariant, SLType, SLIdentity, SLHistory,
       SLNoRecreatedLog, SLPermission, SLHealthy
<1>3. SLFail \/ SLCrash => SLInvariant'
    BY SMT DEF SLFail, SLCrash, SLInvariant, SLType, SLIdentity, SLHistory,
       SLNoRecreatedLog, SLPermission, SLHealthy
<1>4. \A p \in 0..3, l \in SLIncarnations \cup {0} : SLRestart(p, l) => SLInvariant'
    BY SMT DEF SLRestart, SLInvariant, SLType, SLIdentity, SLHistory,
       SLNoRecreatedLog, SLPermission, SLHealthy
<1>5. SLLoseLog \/ SLLoseLifecycle \/ SLWholeLoss \/ SLLegacyWrite => SLInvariant'
    BY SLLegalInputs, SMT DEF SLLoseLog, SLLoseLifecycle, SLWholeLoss, SLLegacyWrite,
       SLInvariant, SLType, SLIdentity, SLHistory, SLNoRecreatedLog, SLPermission, SLHealthy
<1>6. UNCHANGED slVars => SLInvariant'
    BY SMT DEF slVars, SLInvariant, SLType, SLIdentity, SLHistory,
       SLNoRecreatedLog, SLPermission, SLHealthy
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, SMT DEF SLNext, SLQuiesce

THEOREM SLInvariantAlways == SLSpec => []SLInvariant
BY SLInvariantInit, SLInvariantStep, PTL DEF SLSpec
THEOREM SLPermissionAlways == SLSpec => []SLPermission
<1>1. SLInvariant => SLPermission BY SMT DEF SLInvariant
<1> QED BY SLInvariantAlways, <1>1, PTL
THEOREM SLNoRecreatedLogAlways == SLSpec => []SLNoRecreatedLog
<1>1. SLInvariant => SLNoRecreatedLog BY SMT DEF SLInvariant
<1> QED BY SLInvariantAlways, <1>1, PTL
THEOREM SLRootStepProof == ASSUME SLInvariant, SLNext PROVE SLRootStep
BY SMT DEF SLNext, SLRootStep, SLPrepare, SLRootBind, SLWriteBinding, SLPublish,
   SLCreateLog, SLSyncLog, SLWriteActivation, SLStart, SLCertify, SLFail, SLCrash,
   SLRestart, SLLoseLog, SLLoseLifecycle, SLWholeLoss, SLLegacyWrite, SLQuiesce, slVars
THEOREM SLRootProof == SLSpec => SLRootAlways
<1>1. SLInvariant /\ [SLNext]_slVars => [SLRootStep]_slVars
    BY SLRootStepProof, SMT DEF slVars, SLRootStep
<1> QED BY SLInvariantAlways, <1>1, PTL DEF SLSpec, SLRootAlways

THEOREM SLActivationStepProof == ASSUME SLInvariant, SLNext PROVE SLActivationStep
BY SMT DEF SLNext, SLActivationStep, SLPrepare, SLRootBind, SLWriteBinding, SLPublish,
   SLCreateLog, SLSyncLog, SLWriteActivation, SLStart, SLCertify, SLFail, SLCrash,
   SLRestart, SLLoseLog, SLLoseLifecycle, SLWholeLoss, SLLegacyWrite, SLQuiesce, slVars,
   SLInvariant, SLType, SLIdentity
THEOREM SLActivationProof == SLSpec => SLActivationAlways
<1>1. SLInvariant /\ [SLNext]_slVars => [SLActivationStep]_slVars
    BY SLActivationStepProof, SMT DEF slVars, SLActivationStep
<1> QED BY SLInvariantAlways, <1>1, PTL DEF SLSpec, SLActivationAlways

THEOREM SLStableInvariant == SLFairSpec => []SLInvariant
<1>1. SLStableNext => SLNext BY SMT DEF SLStableNext, SLNext
<1>2. SLInvariant /\ [SLStableNext]_slVars => SLInvariant'
    BY <1>1, SLInvariantStep, SMT
<1> QED BY <1>2, PTL DEF SLFairSpec

THEOREM SLActivationWriteProgress == SLFairSpec => (SLReady ~> SLWrittenReady)
<1>1. SLReady /\ [SLStableNext]_slVars => SLReady'
    BY SLLegalInputs, SLInvariantStep, SMT DEF SLReady, SLStableNext, SLNext,
       SLRootBind, SLWriteBinding, SLPublish, SLCreateLog, SLSyncLog,
       SLWriteActivation, SLStart, SLCertify, SLQuiesce, slVars, SLHealthy, SLInvariant, SLType
<1>2. SLReady /\ SLWriteActivation => SLWrittenReady'
    BY <1>1, SMT DEF SLWrittenReady, SLReady, SLWriteActivation, SLStableNext, slVars
<1>3. SLReady /\ ~SLWrittenReady => ENABLED <<SLWriteActivation>>_slVars
    BY ExpandENABLED, SMT DEF SLReady, SLWrittenReady, SLInvariant, SLType, SLWriteActivation, SLHealthy, slVars
<1> QED BY <1>1, <1>2, <1>3, PTL DEF SLFairSpec, SLWrittenReady

THEOREM SLActivationPublishProgress == SLFairSpec => (SLWrittenReady ~> SLActiveReady)
<1>1. SLWrittenReady /\ [SLStableNext]_slVars => SLWrittenReady'
    BY SLLegalInputs, SLInvariantStep, SMT DEF SLReady, SLWrittenReady, SLStableNext, SLNext,
       SLRootBind, SLWriteBinding, SLPublish, SLCreateLog, SLSyncLog,
       SLWriteActivation, SLStart, SLCertify, SLQuiesce, slVars, SLHealthy, SLInvariant, SLType
<1>2. SLWrittenReady /\ SLPublish => SLActiveReady'
    BY <1>1, SMT DEF SLActiveReady, SLWrittenReady, SLReady, SLPublish, SLStableNext, slVars
<1>3. SLWrittenReady /\ ~SLActiveReady => ENABLED <<SLPublish>>_slVars
    BY ExpandENABLED, SMT DEF SLWrittenReady, SLActiveReady, SLReady, SLPublish, SLHealthy, slVars
<1> QED BY <1>1, <1>2, <1>3, PTL DEF SLFairSpec, SLActiveReady

THEOREM SLStartProgress == SLFairSpec => (SLActiveReady ~> SLStarted)
<1>1. SLActiveReady /\ [SLStableNext]_slVars => SLActiveReady'
    BY SLLegalInputs, SLInvariantStep, SMT DEF SLReady, SLActiveReady, SLStableNext, SLNext,
       SLRootBind, SLWriteBinding, SLPublish, SLCreateLog, SLSyncLog,
       SLWriteActivation, SLStart, SLCertify, SLQuiesce, slVars, SLHealthy, SLInvariant, SLType
<1>2. SLActiveReady /\ SLStart => SLStarted'
    BY <1>1, SMT DEF SLStarted, SLStart, SLStableNext, slVars
<1>3. ASSUME SLActiveReady, ~slOwner PROVE ENABLED <<SLStart>>_slVars
    <2>1. SLHealthy /\ slPhase = 3 /\ slLog = slInc /\ slInc = slRoot /\ slInc # 0
        BY <1>3, SMT DEF SLActiveReady, SLReady
    <2>2. slOwner = FALSE BY <1>3, SMT DEF SLActiveReady, SLReady, SLInvariant, SLType
    (* Separate the next-state witness from nonstuttering: starting changes
       slOwner from FALSE to TRUE, so SLStart already changes the full frame.
       Lift the proved action equivalence using TLAPS's checked ENABLED rule. *)
    <2>3. ENABLED SLStart
        BY <2>1, <2>2, ExpandENABLED, SMT DEF SLStart
    <2>4. SLStart <=> <<SLStart>>_slVars
        BY <2>2, SMT DEF SLStart, slVars
    <2>5. ENABLED SLStart <=> ENABLED <<SLStart>>_slVars
        BY <2>4, ENABLEDaxioms
    <2> QED BY <2>3, <2>5, SMT
<1> QED BY <1>1, <1>2, <1>3, PTL DEF SLFairSpec, SLStarted

THEOREM SLProgressProof == SLFairSpec => SLProgress
BY SLStableInvariant, SLActivationWriteProgress, SLActivationPublishProgress, SLStartProgress, PTL
   DEF SLProgress, SLReady, SLWrittenReady, SLActiveReady, SLStarted
=============================================================================
