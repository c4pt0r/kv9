-------------------------- MODULE NativeBatchProof --------------------------
EXTENDS NativeBatch, NativeBatchAlgebraProof, ClientRetryProof

THEOREM NBProjectionType ==
    ASSUME NEW view \in RMStates, NEW keys \in Seq(RMKeys)
    PROVE NBProjection(view, keys) \in Seq(RMValues \cup {0}) /\
          Len(NBProjection(view, keys)) = Len(keys)
BY SMT DEF NBProjection, RMStates

THEOREM NBProjectionDuplicates ==
    ASSUME NEW view \in RMStates, NEW keys \in Seq(RMKeys),
        NEW i \in 1..Len(keys), NEW j \in 1..Len(keys), keys[i] = keys[j]
    PROVE NBProjection(view, keys)[i] = NBProjection(view, keys)[j]
BY SMT DEF NBProjection

THEOREM NBProjectionAbsence ==
    ASSUME NEW view \in RMStates, NEW keys \in Seq(RMKeys), NEW i \in 1..Len(keys),
        NEW emptyValue \in RMValues
    PROVE (NBProjection(view, keys)[i] = 0 <=> view[keys[i]] = 0) /\
          (NBProjection(view, keys)[i] = emptyValue <=> view[keys[i]] = emptyValue) /\
          emptyValue # 0
BY RMLegalInputs, SMT DEF NBProjection

THEOREM NBRetryStep == NBNext => [CRNext]_crVars
BY SMT DEF NBNext, NBDispatch, NBEffect, NBSuccess, NBControl, NBBackground,
    NBReadStart, NBReadCapture, NBReadItem, NBReadPublish, NBReadFail,
    NBQuiesce, nbVars, nbLocal, CRNext, crVars

THEOREM NBRetryRefinement == NBSpec => CRSpec
<1>1. NBInit => CRInit BY DEF NBInit
<1>2. [NBNext]_nbVars => [CRNext]_crVars
    BY NBRetryStep, SMT DEF nbVars, nbLocal, crVars
<1> QED BY <1>1, <1>2, PTL DEF NBSpec, CRSpec

THEOREM NBRetryUnchanged == CRInvariant /\ UNCHANGED crVars => CRInvariant'
BY SMT DEF CRInvariant, CRType, CRSinglePotential, CRClosedExcludesEffects,
    CRReady, CRFixedBudget, crVars

THEOREM NBInvariantInit == NBInit => NBInvariant
BY CRInvariantInit, NBLegalInputs, SMT DEF NBInit, NBInvariant, NBType,
    NBCommandBinding, NBReadPrefix, NBReadLength, NBWriteBinding, NBAckBinding, CRInit

THEOREM NBEffectCommand ==
    ASSUME NBInvariant, NEW i \in 1..CRMaxAttempts, NBEffect(i)
    PROVE nbCommand = NBWriteBatch
BY SMT DEF NBInvariant, NBCommandBinding, CRInvariant, CRType, NBEffect, CREffect

THEOREM NBDispatchStep ==
    ASSUME NBInvariant, NBDispatch PROVE NBInvariant'
<1>1. CRInvariant' BY NBRetryStep, CRInvariantStep DEF NBInvariant, NBNext
<1> QED BY <1>1, NBLegalInputs, SMT DEF NBInvariant, NBType, NBCommandBinding,
    NBReadPrefix, NBReadLength, NBWriteBinding, NBAckBinding, NBDispatch, CRDispatch,
    CRInvariant, CRClosedExcludesEffects, CRType

THEOREM NBEffectPreserves ==
    ASSUME NBInvariant, NEW i \in 1..CRMaxAttempts, NBEffect(i) PROVE NBInvariant'
<1>1. CRInvariant' BY NBRetryStep, CRInvariantStep DEF NBInvariant, NBNext
<1>2. nbCommand = NBWriteBatch BY NBEffectCommand
<1>3. RMApply(nbStore, nbCommand) \in RMStates
    BY RMApplyType DEF NBInvariant, NBType
<1> QED BY <1>1, <1>2, <1>3, SMT DEF NBInvariant, NBType, NBCommandBinding,
    NBReadPrefix, NBReadLength, NBWriteBinding, NBAckBinding, NBEffect, CREffect

THEOREM NBSuccessStep ==
    ASSUME NBInvariant, NBSuccess PROVE NBInvariant'
<1>1. CRInvariant' BY NBRetryStep, CRInvariantStep DEF NBInvariant, NBNext
<1> QED BY <1>1, SMT DEF NBInvariant, NBType, NBCommandBinding,
    NBReadPrefix, NBReadLength, NBWriteBinding, NBAckBinding, NBSuccess, CRSuccess

THEOREM NBControlStep ==
    ASSUME NBInvariant, NBControl PROVE NBInvariant'
<1>1. CRInvariant' BY NBRetryStep, CRInvariantStep DEF NBInvariant, NBNext
<1> QED BY <1>1, SMT DEF NBInvariant, NBType, NBCommandBinding,
    NBReadPrefix, NBReadLength, NBWriteBinding, NBAckBinding, NBControl, nbLocal,
    CRRefuse, CRUnknown, CRStop, CRTick

THEOREM NBBackgroundStep ==
    ASSUME NBInvariant, NEW batch \in NBBackgroundBatches, NBBackground(batch)
    PROVE NBInvariant'
<1>1. RMApply(nbStore, batch) \in RMStates BY NBLegalInputs, RMApplyType, SMT DEF NBInvariant, NBType
<1>2. CRInvariant' BY NBRetryUnchanged, SMT DEF NBInvariant, NBBackground, crVars
<1>3. NBType' BY <1>1, NBLegalInputs, SMT DEF NBInvariant, NBType, NBBackground, crVars
<1> QED BY <1>1, <1>2, <1>3, SMT DEF NBInvariant, NBCommandBinding,
    NBReadPrefix, NBReadLength, NBWriteBinding, NBAckBinding, NBBackground, crVars

THEOREM NBReadStartStep ==
    ASSUME NBInvariant, NBReadStart PROVE NBInvariant'
BY NBRetryUnchanged, SMT DEF NBInvariant, NBType, NBCommandBinding, NBReadPrefix, NBReadLength,
    NBWriteBinding, NBAckBinding, NBReadStart, crVars

THEOREM NBReadCaptureStep ==
    ASSUME NBInvariant, NBReadCapture PROVE NBInvariant'
<1>1. CRInvariant' BY NBRetryUnchanged, SMT DEF NBInvariant, NBReadCapture, crVars
<1>2. NBType' BY NBLegalInputs, SMT DEF NBInvariant, NBType, NBReadCapture, crVars
<1> QED BY <1>1, <1>2, SMT DEF NBInvariant, NBCommandBinding, NBReadPrefix, NBReadLength,
    NBWriteBinding, NBAckBinding, NBReadCapture, crVars

THEOREM NBReadItemStep ==
    ASSUME NBInvariant, NBReadItem PROVE NBInvariant'
BY NBLegalInputs, NBRetryUnchanged, SMT DEF NBInvariant, NBType, NBCommandBinding, NBReadPrefix, NBReadLength,
    NBWriteBinding, NBAckBinding, NBReadItem, RMStates, crVars

THEOREM NBReadPublishStep ==
    ASSUME NBInvariant, NBReadPublish PROVE NBInvariant'
BY NBRetryUnchanged, SMT DEF NBInvariant, NBType, NBCommandBinding, NBReadPrefix, NBReadLength,
    NBWriteBinding, NBAckBinding, NBReadPublish, crVars

THEOREM NBReadFailStep ==
    ASSUME NBInvariant, NBReadFail PROVE NBInvariant'
BY NBRetryUnchanged, SMT DEF NBInvariant, NBType, NBCommandBinding, NBReadPrefix, NBReadLength,
    NBWriteBinding, NBAckBinding, NBReadFail, crVars

THEOREM NBStutterStep ==
    ASSUME NBInvariant, UNCHANGED nbVars PROVE NBInvariant'
BY NBRetryUnchanged, SMT DEF NBInvariant, NBType, NBCommandBinding, NBReadPrefix, NBReadLength,
    NBWriteBinding, NBAckBinding, nbVars, nbLocal, crVars

THEOREM NBInvariantStep ==
    ASSUME NBInvariant, [NBNext]_nbVars PROVE NBInvariant'
BY NBDispatchStep, NBEffectPreserves, NBSuccessStep, NBControlStep, NBBackgroundStep,
    NBReadStartStep, NBReadCaptureStep, NBReadItemStep, NBReadPublishStep, NBReadFailStep,
    NBStutterStep, SMT DEF NBNext, NBQuiesce

THEOREM NBInvariantAlways == NBSpec => []NBInvariant
BY NBInvariantInit, NBInvariantStep, PTL DEF NBSpec

THEOREM NBReadAtomicFromInvariant == NBInvariant => NBReadAtomic /\ NBReadDuplicates
BY NBLegalInputs, SMT DEF NBInvariant, NBType, NBReadPrefix, NBReadLength,
    NBReadAtomic, NBReadDuplicates, NBProjection

THEOREM NBReadAtomicAlways == NBSpec => [](NBReadAtomic /\ NBReadDuplicates)
BY NBInvariantAlways, NBReadAtomicFromInvariant, PTL

THEOREM NBOrderedImageFromInvariant == NBInvariant => NBOrderedImage
BY NBLegalInputs, NBApplyLastPair, NBApplyUntouched, SMT
    DEF NBInvariant, NBType, NBWriteBinding, NBOrderedImage

THEOREM NBOrderedImageAlways == NBSpec => []NBOrderedImage
BY NBInvariantAlways, NBOrderedImageFromInvariant, PTL

THEOREM NBSystemImageFromInvariant == NBInvariant => NBSystemImage
BY NBLegalInputs, RMApplySystem, SMT DEF NBInvariant, NBType, NBWriteBinding, NBSystemImage

THEOREM NBSystemImageAlways == NBSpec => []NBSystemImage
BY NBInvariantAlways, NBSystemImageFromInvariant, PTL

THEOREM NBWholeEffectStepProof ==
    ASSUME NBInvariant, NBNext PROVE NBWholeEffectStep
BY NBEffectCommand, SMT DEF NBNext, NBWholeEffectStep, NBDispatch, NBEffect,
    NBSuccess, NBControl, NBBackground, NBReadStart, NBReadCapture, NBReadItem,
    NBReadPublish, NBReadFail, NBQuiesce, nbVars, nbLocal, crVars, CRDispatch,
    CREffect, CRSuccess, CRRefuse, CRUnknown, CRStop, CRTick

THEOREM NBWholeEffectAlways == NBSpec => NBWholeEffectSafety
<1>1. NBInvariant /\ [NBNext]_nbVars => [NBWholeEffectStep]_nbVars
    BY NBWholeEffectStepProof, SMT DEF NBWholeEffectStep, nbVars, nbLocal, crVars
<1> QED BY <1>1, NBInvariantAlways, PTL DEF NBSpec, NBWholeEffectSafety

THEOREM NBAcknowledgementAlways == NBSpec => [](NBAckBinding /\ NBWriteBinding)
BY NBInvariantAlways, PTL DEF NBInvariant

THEOREM NBRetrySafetyAlways ==
    NBSpec => []CRAtMostOneEffect /\ CRDispatchBudgetSafety /\ CRTerminalFreeze
BY NBRetryRefinement, CRSafetyAlways, CRDispatchBudgetAlways, CRTerminalAlways, PTL
=============================================================================
