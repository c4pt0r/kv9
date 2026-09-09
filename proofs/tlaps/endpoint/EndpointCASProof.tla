-------------------------- MODULE EndpointCASProof --------------------------
EXTENDS EndpointCAS, TLAPS
THEOREM EPInvariantInit == EPInit => EPInvariant
BY EPLegalInputs, SMT DEF EPInit, EPInvariant, EPType, EPBinding, EPStates
THEOREM EPInvariantStep == ASSUME EPInvariant, [EPNext]_epVars PROVE EPInvariant'
<1>1. \A g \in 0..EPLimit, o, n \in EPAddresses, s \in EPStores : EPIssue(g, o, n, s) => EPInvariant'
    BY EPLegalInputs, SMT DEF EPIssue, EPInvariant, EPType, EPBinding, EPStates, EPDirectory
<1>2. EPEvaluate => EPInvariant'
    BY EPLegalInputs, SMT DEF EPEvaluate, EPMatch, EPReplay, EPInvariant, EPType, EPBinding,
       EPStates, EPDirectory, EPRequests
<1>3. \A a \in EPAddresses : EPInterfere(a) => EPInvariant'
    BY EPLegalInputs, SMT DEF EPInterfere, EPInvariant, EPType, EPBinding, EPStates,
       EPDirectory, EPRequests, EPResults
<1>4. EPRetry \/ UNCHANGED epVars => EPInvariant'
    BY EPLegalInputs, SMT DEF EPRetry, EPInvariant, EPType, EPBinding, EPStates, epVars,
       EPDirectory, EPRequests, EPResults
<1> QED BY <1>1, <1>2, <1>3, <1>4, SMT DEF EPNext
THEOREM EPInvariantAlways == EPSpec => []EPInvariant
BY EPInvariantInit, EPInvariantStep, PTL DEF EPSpec
THEOREM EPAtomicVersionProof == ASSUME EPInvariant, EPNext PROVE EPAtomicVersion
BY EPLegalInputs, SMT DEF EPAtomicVersion, EPNext, EPIssue, EPEvaluate, EPMatch, EPReplay,
   EPInterfere, EPRetry, EPDirectory, EPRequests, EPResults, EPInvariant, EPType, EPBinding, EPStates
THEOREM EPChangePreconditionProof == EPInvariant => EPChangePrecondition
BY SMT DEF EPChangePrecondition, EPEvaluate, EPMatch, EPReplay, EPDirectory, EPRequests
THEOREM EPConfirmationProof == EPInvariant => EPConfirmation
BY SMT DEF EPConfirmation, EPEvaluate, EPMatch, EPReplay, EPDirectory, EPRequests
THEOREM EPNoRefusalWriteProof == EPInvariant => EPNoRefusalWrite
BY SMT DEF EPNoRefusalWrite, EPEvaluate, EPMatch, EPReplay, EPDirectory, EPRequests
THEOREM EPEffectsProof == EPSpec => EPEffects
<1>1. EPInvariant /\ [EPNext]_epVars => [EPStepEffects]_epVars
    BY EPAtomicVersionProof, EPChangePreconditionProof, EPConfirmationProof,
       EPNoRefusalWriteProof, SMT DEF EPStepEffects, epVars
<1> QED BY EPInvariantAlways, <1>1, PTL DEF EPSpec, EPEffects
THEOREM EPFairInvariant == EPFairSpec => []EPInvariant
BY EPInvariantStep, PTL DEF EPFairSpec
THEOREM EPResultProgress == EPFairSpec => EPProgress
<1>1. EPPending /\ [EPNext]_epVars => EPPending' \/ EPDone'
    BY EPInvariantStep, SMT DEF EPPending, EPDone
<1>2. EPPending /\ EPEvaluate => EPDone'
    BY EPInvariantStep, SMT DEF EPDone, EPPending, EPNext, EPEvaluate, EPMatch,
       EPReplay, EPDirectory, EPRequests
<1>3. EPPending => ENABLED <<EPEvaluate>>_epVars
    BY ExpandENABLED, SMT DEF EPPending, EPEvaluate, EPMatch, EPReplay,
       EPInvariant, EPType, EPBinding, EPStates, epVars, EPDirectory, EPRequests
<1>4. EPFairSpec => (EPPending ~> EPDone)
    BY <1>1, <1>2, <1>3, PTL DEF EPFairSpec
<1> QED BY EPFairInvariant, <1>4, PTL DEF EPProgress, EPPending, EPDone
THEOREM EPStableSuccessProof == EPStableSpec => EPSuccess
<1>1. EPEligible /\ [EPEvaluate]_epVars => EPEligible' \/ EPChanged'
    BY SMT DEF EPEligible, EPChanged, EPPending, EPEvaluate, EPMatch, EPReplay,
       epVars, EPDirectory, EPRequests, EPInvariant, EPType, EPBinding, EPStates
<1>2. EPEligible /\ EPEvaluate => EPChanged'
    BY SMT DEF EPEligible, EPChanged, EPEvaluate, EPMatch, EPReplay, EPDirectory, EPRequests
<1>3. EPEligible => ENABLED <<EPEvaluate>>_epVars
    BY ExpandENABLED, SMT DEF EPEligible, EPPending, EPEvaluate, EPMatch, EPReplay,
       EPInvariant, EPType, EPBinding, EPStates, epVars, EPDirectory, EPRequests
<1>4. EPStableSpec => (EPEligible ~> EPChanged)
    BY <1>1, <1>2, <1>3, PTL DEF EPStableSpec
<1> QED BY <1>4, PTL DEF EPStableSpec, EPSuccess, EPChanged
=============================================================================
