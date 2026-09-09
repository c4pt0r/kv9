-------------------------- MODULE ClientRetryProof --------------------------
EXTENDS ClientRetry, TLAPS
THEOREM CRInvariantInit == CRInit => CRInvariant
BY CRLegalInputs, SMT DEF CRInit, CRInvariant, CRType, CRSinglePotential,
   CRClosedExcludesEffects, CRReady, CRFixedBudget

THEOREM CRInvariantStep == ASSUME CRInvariant, [CRNext]_crVars PROVE CRInvariant'
<1>1. CRDispatch => CRInvariant'
    BY SMT DEF CRDispatch, CRInvariant, CRType, CRSinglePotential,
       CRClosedExcludesEffects, CRReady, CRFixedBudget
<1>2. CRRefuse => CRInvariant'
    BY SMT DEF CRRefuse, CRInvariant, CRType, CRSinglePotential,
       CRClosedExcludesEffects, CRReady, CRFixedBudget
<1>3. \A i \in 1..CRMaxAttempts : CREffect(i) => CRInvariant'
    BY SMT DEF CREffect, CRInvariant, CRType, CRSinglePotential,
       CRClosedExcludesEffects, CRReady, CRFixedBudget
<1>4. CRSuccess \/ CRUnknown \/ CRStop \/ CRTick => CRInvariant'
    BY SMT DEF CRSuccess, CRUnknown, CRStop, CRTick, CRInvariant, CRType,
       CRSinglePotential, CRClosedExcludesEffects, CRReady, CRFixedBudget
<1>5. UNCHANGED crVars => CRInvariant'
    BY SMT DEF crVars, CRInvariant, CRType, CRSinglePotential,
       CRClosedExcludesEffects, CRReady, CRFixedBudget
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, SMT DEF CRNext, CRQuiesce

THEOREM CRSafetyAlways == CRSpec => []CRAtMostOneEffect
<1>1. CRSpec => []CRInvariant BY CRInvariantInit, CRInvariantStep, PTL DEF CRSpec
<1>2. CRInvariant => CRAtMostOneEffect
    BY SMT DEF CRInvariant, CRType, CRSinglePotential, CRClosedExcludesEffects, CRAtMostOneEffect
<1> QED BY <1>1, <1>2, PTL

THEOREM CRBudgetAlways == CRSpec => []CRFixedBudget
<1>1. CRSpec => []CRInvariant BY CRInvariantInit, CRInvariantStep, PTL DEF CRSpec
<1>2. CRInvariant => CRFixedBudget BY SMT DEF CRInvariant
<1> QED BY <1>1, <1>2, PTL

THEOREM CRTerminalStepProof == ASSUME CRInvariant, CRNext PROVE CRTerminalStep
BY SMT DEF CRNext, CRTerminalStep, CRDispatch, CRRefuse, CREffect, CRSuccess,
   CRUnknown, CRStop, CRTick, CRQuiesce, crVars

THEOREM CRTerminalAlways == CRSpec => CRTerminalFreeze
<1>1. CRSpec => []CRInvariant BY CRInvariantInit, CRInvariantStep, PTL DEF CRSpec
<1>2. CRInvariant /\ [CRNext]_crVars => [CRTerminalStep]_crVars
    BY CRTerminalStepProof, SMT DEF crVars, CRTerminalStep
<1> QED BY <1>1, <1>2, PTL DEF CRSpec, CRTerminalFreeze

THEOREM CRDispatchBudgetStepProof == ASSUME CRInvariant, CRNext PROVE CRDispatchBudgetStep
BY SMT DEF CRInvariant, CRFixedBudget, CRNext, CRDispatchBudgetStep, CRDispatch,
   CRRefuse, CREffect, CRSuccess, CRUnknown, CRStop, CRTick, CRQuiesce, crVars

THEOREM CRDispatchBudgetAlways == CRSpec => CRDispatchBudgetSafety
<1>1. CRSpec => []CRInvariant BY CRInvariantInit, CRInvariantStep, PTL DEF CRSpec
<1>2. CRInvariant /\ [CRNext]_crVars => [CRDispatchBudgetStep]_crVars
    BY CRDispatchBudgetStepProof, SMT DEF crVars, CRDispatchBudgetStep
<1> QED BY <1>1, <1>2, PTL DEF CRSpec, CRDispatchBudgetSafety
=============================================================================
