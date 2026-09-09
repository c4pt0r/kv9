--------------------------- MODULE ReadAdmissionProof ---------------------------
EXTENDS ReadAdmission, TLAPS
THEOREM RAInvariantInit == RAInit => RAInvariant
BY RALegalInputs, SMT DEF RAInit, RAInvariant, RAType, RABinding, RAPhases
THEOREM RAInvariantStep == ASSUME RAInvariant, [RANext]_raVars PROVE RAInvariant'
<1>1. RAPoll => RAInvariant'
    BY RALegalInputs, SMT DEF RAPoll, RAInvariant, RAType, RABinding, RAPhases, RAEnv, RARequest
<1>2. RACommit => RAInvariant'
    BY RALegalInputs, SMT DEF RACommit, RAInvariant, RAType, RABinding, RAPhases, RAEnv, RARequest
<1>3. (\E l \in BOOLEAN : RAAdvance(l)) => RAInvariant'
    BY RALegalInputs, SMT DEF RAAdvance, RAInvariant, RAType, RABinding, RAPhases, RAEnv, RARequest
<1>4. RALose => RAInvariant'
    BY RALegalInputs, SMT DEF RALose, RAInvariant, RAType, RABinding, RAPhases, RAEnv, RARequest
<1>5. RACertify => RAInvariant'
    BY RALegalInputs, SMT DEF RACertify, RAInvariant, RAType, RABinding, RAPhases, RAEnv, RARequest
<1>6. (\E c \in {RAOwn, RAOther} : RADeliver(c)) => RAInvariant'
    BY RALegalInputs, SMT DEF RADeliver, RAInvariant, RAType, RABinding, RAPhases, RAEnv, RARequest
<1>7. RACatchUp => RAInvariant'
    BY RALegalInputs, SMT DEF RACatchUp, RAInvariant, RAType, RABinding, RAPhases, RAEnv, RARequest
<1>8. RAComplete => RAInvariant'
    BY RALegalInputs, SMT DEF RAComplete, RAInvariant, RAType, RABinding, RAPhases, RAEnv, RARequest
<1>9. RATick => RAInvariant'
    BY RALegalInputs, SMT DEF RATick, RAActive, RAInvariant, RAType, RABinding, RAPhases, RAEnv, RARequest
<1>10. RATimeout => RAInvariant'
    BY RALegalInputs, SMT DEF RATimeout, RAActive, RAInvariant, RAType, RABinding, RAPhases, RAEnv, RARequest
<1>11. UNCHANGED raVars => RAInvariant'
    BY RALegalInputs, SMT DEF raVars, RAInvariant, RAType, RABinding, RAPhases, RAEnv, RARequest
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, SMT DEF RANext
THEOREM RAInvariantAlways == RASpec => []RAInvariant
BY RAInvariantInit, RAInvariantStep, PTL DEF RASpec
THEOREM RAAdmissionProof == RAInvariant => RAAdmission
BY SMT DEF RAAdmission, RAPoll, RAEnv, RARequest
THEOREM RAEffectsProof == RASpec => RAEffects
<1>1. RAInvariant /\ [RANext]_raVars => [RAAdmission /\ UNCHANGED raContext /\ raBudget' <= raBudget]_raVars
    BY RAAdmissionProof, RALegalInputs, SMT DEF RANext, RAPoll, RACommit, RAAdvance, RALose,
       RACertify, RADeliver, RACatchUp, RAComplete, RATick, RATimeout, RAActive,
       RAEnv, RARequest, raVars, RAInvariant, RAType, RABinding, RAPhases
<1> QED BY RAInvariantAlways, <1>1, PTL DEF RASpec, RAEffects
THEOREM RASafeReturnProof == RASpec => []RASafeReturn
<1>1. RAInvariant => RASafeReturn
    BY SMT DEF RAInvariant, RABinding, RASafeReturn
<1> QED BY RAInvariantAlways, <1>1, PTL
THEOREM RAStableInvariantStep == ASSUME RAStableInvariant, [RAStableNext]_raVars PROVE RAStableInvariant'
<1>1. RAInvariant'
    BY RAInvariantStep, SMT DEF RAStableInvariant, RAStableNext, RANext
<1> QED BY <1>1, SMT DEF RAStableInvariant, RAStableNext, RAPoll, RACommit,
    RACertify, RADeliver, RACatchUp, RAComplete, RAEnv, RARequest, raVars
THEOREM RAStableInvariantAlways == RAStableSpec => []RAStableInvariant
<1>1. RAInit => RAStableInvariant
    BY RAInvariantInit, SMT DEF RAInit, RAStableInvariant
<1> QED BY <1>1, RAStableInvariantStep, PTL DEF RAStableSpec, RAStableFairness
THEOREM RAProgress0 == RAStableSpec => (RAStage0 ~> RAStage1)
<1>1. RAStage0 /\ [RAStableNext]_raVars => RAStage0' \/ RAStage1'
    BY RAStableInvariantStep, RALegalInputs, SMT DEF RAStage0, RAStage1, RAStableNext, RAPoll, RACommit, RACertify, RADeliver, RACatchUp, RAComplete, RAEnv, RARequest, raVars, RAStableInvariant, RAInvariant, RAType, RABinding, RAPhases
<1>2. RAStage0 /\ RACommit => RAStage1'
    BY RAStableInvariantStep, RALegalInputs, SMT DEF RAStage0, RAStage1, RAStableNext, RAPoll, RACommit, RACertify, RADeliver, RACatchUp, RAComplete, RAEnv, RARequest, raVars, RAStableInvariant, RAInvariant, RAType, RABinding, RAPhases
<1>3. RAStage0 => ENABLED <<RACommit>>_raVars
    BY ExpandENABLED, RALegalInputs, SMT DEF RAStage0, RAStage1, RAStableNext, RAPoll, RACommit, RACertify, RADeliver, RACatchUp, RAComplete, RAEnv, RARequest, raVars, RAStableInvariant, RAInvariant, RAType, RABinding, RAPhases
<1> QED BY <1>1, <1>2, <1>3, PTL DEF RAStableSpec, RAStableFairness
THEOREM RAProgress1 == RAStableSpec => (RAStage1 ~> RAStage2)
<1>1. RAStage1 /\ [RAStableNext]_raVars => RAStage1' \/ RAStage2'
    BY RAStableInvariantStep, RALegalInputs, SMT DEF RAStage1, RAStage2, RAStableNext, RAPoll, RACommit, RACertify, RADeliver, RACatchUp, RAComplete, RAEnv, RARequest, raVars, RAStableInvariant, RAInvariant, RAType, RABinding, RAPhases
<1>2. RAStage1 /\ RAPoll => RAStage2'
    BY RAStableInvariantStep, RALegalInputs, SMT DEF RAStage1, RAStage2, RAStableNext, RAPoll, RACommit, RACertify, RADeliver, RACatchUp, RAComplete, RAEnv, RARequest, raVars, RAStableInvariant, RAInvariant, RAType, RABinding, RAPhases
<1>3. RAStage1 => ENABLED <<RAPoll>>_raVars
    BY ExpandENABLED, RALegalInputs, SMT DEF RAStage1, RAStage2, RAStableNext, RAPoll, RACommit, RACertify, RADeliver, RACatchUp, RAComplete, RAEnv, RARequest, raVars, RAStableInvariant, RAInvariant, RAType, RABinding, RAPhases
<1> QED BY <1>1, <1>2, <1>3, PTL DEF RAStableSpec, RAStableFairness
THEOREM RAProgress2 == RAStableSpec => (RAStage2 ~> RAStage3)
<1>1. RAStage2 /\ [RAStableNext]_raVars => RAStage2' \/ RAStage3'
    BY RAStableInvariantStep, RALegalInputs, SMT DEF RAStage2, RAStage3, RAStableNext, RAPoll, RACommit, RACertify, RADeliver, RACatchUp, RAComplete, RAEnv, RARequest, raVars, RAStableInvariant, RAInvariant, RAType, RABinding, RAPhases
<1>2. RAStage2 /\ RACertify => RAStage3'
    BY RAStableInvariantStep, RALegalInputs, SMT DEF RAStage2, RAStage3, RAStableNext, RAPoll, RACommit, RACertify, RADeliver, RACatchUp, RAComplete, RAEnv, RARequest, raVars, RAStableInvariant, RAInvariant, RAType, RABinding, RAPhases
<1>3. RAStage2 => ENABLED <<RACertify>>_raVars
    BY ExpandENABLED, RALegalInputs, SMT DEF RAStage2, RAStage3, RAStableNext, RAPoll, RACommit, RACertify, RADeliver, RACatchUp, RAComplete, RAEnv, RARequest, raVars, RAStableInvariant, RAInvariant, RAType, RABinding, RAPhases
<1> QED BY <1>1, <1>2, <1>3, PTL DEF RAStableSpec, RAStableFairness
THEOREM RAProgress3 == RAStableSpec => (RAStage3 ~> RAStage4)
<1>1. RAStage3 /\ [RAStableNext]_raVars => RAStage3' \/ RAStage4'
    BY RAStableInvariantStep, RALegalInputs, SMT DEF RAStage3, RAStage4, RAStableNext, RAPoll, RACommit, RACertify, RADeliver, RACatchUp, RAComplete, RAEnv, RARequest, raVars, RAStableInvariant, RAInvariant, RAType, RABinding, RAPhases
<1>2. RAStage3 /\ RADeliver(RAOwn) => RAStage4'
    BY RAStableInvariantStep, RALegalInputs, SMT DEF RAStage3, RAStage4, RAStableNext, RAPoll, RACommit, RACertify, RADeliver, RACatchUp, RAComplete, RAEnv, RARequest, raVars, RAStableInvariant, RAInvariant, RAType, RABinding, RAPhases
<1>3. RAStage3 => ENABLED <<RADeliver(RAOwn)>>_raVars
    BY ExpandENABLED, RALegalInputs, SMT DEF RAStage3, RAStage4, RAStableNext, RAPoll, RACommit, RACertify, RADeliver, RACatchUp, RAComplete, RAEnv, RARequest, raVars, RAStableInvariant, RAInvariant, RAType, RABinding, RAPhases
<1> QED BY <1>1, <1>2, <1>3, PTL DEF RAStableSpec, RAStableFairness
THEOREM RAProgress4 == RAStableSpec => (RAStage4 ~> RAStage5)
<1>1. RAStage4 /\ [RAStableNext]_raVars => RAStage4' \/ RAStage5'
    BY RAStableInvariantStep, RALegalInputs, SMT DEF RAStage4, RAStage5, RAStableNext, RAPoll, RACommit, RACertify, RADeliver, RACatchUp, RAComplete, RAEnv, RARequest, raVars, RAStableInvariant, RAInvariant, RAType, RABinding, RAPhases
<1>2. RAStage4 /\ RACatchUp => RAStage5'
    BY RAStableInvariantStep, RALegalInputs, SMT DEF RAStage4, RAStage5, RAStableNext, RAPoll, RACommit, RACertify, RADeliver, RACatchUp, RAComplete, RAEnv, RARequest, raVars, RAStableInvariant, RAInvariant, RAType, RABinding, RAPhases
<1>3. RAStage4 => ENABLED <<RACatchUp>>_raVars
    BY ExpandENABLED, RALegalInputs, SMT DEF RAStage4, RAStage5, RAStableNext, RAPoll, RACommit, RACertify, RADeliver, RACatchUp, RAComplete, RAEnv, RARequest, raVars, RAStableInvariant, RAInvariant, RAType, RABinding, RAPhases
<1> QED BY <1>1, <1>2, <1>3, PTL DEF RAStableSpec, RAStableFairness
THEOREM RAProgress5 == RAStableSpec => (RAStage5 ~> RAStage6)
<1>1. RAStage5 /\ [RAStableNext]_raVars => RAStage5' \/ RAStage6'
    BY RAStableInvariantStep, RALegalInputs, SMT DEF RAStage5, RAStage6, RAStableNext, RAPoll, RACommit, RACertify, RADeliver, RACatchUp, RAComplete, RAEnv, RARequest, raVars, RAStableInvariant, RAInvariant, RAType, RABinding, RAPhases
<1>2. RAStage5 /\ RAComplete => RAStage6'
    BY RAStableInvariantStep, RALegalInputs, SMT DEF RAStage5, RAStage6, RAStableNext, RAPoll, RACommit, RACertify, RADeliver, RACatchUp, RAComplete, RAEnv, RARequest, raVars, RAStableInvariant, RAInvariant, RAType, RABinding, RAPhases
<1>3. RAStage5 => ENABLED <<RAComplete>>_raVars
    BY ExpandENABLED, RALegalInputs, SMT DEF RAStage5, RAStage6, RAStableNext, RAPoll, RACommit, RACertify, RADeliver, RACatchUp, RAComplete, RAEnv, RARequest, raVars, RAStableInvariant, RAInvariant, RAType, RABinding, RAPhases
<1> QED BY <1>1, <1>2, <1>3, PTL DEF RAStableSpec, RAStableFairness
THEOREM RAStableSuccessProof == RAStableSpec => RASuccess
<1>1. RAInit => RAStage0
    BY RAInvariantInit, SMT DEF RAInit, RAStage0, RAStableInvariant
<1> QED BY <1>1, RAProgress0, RAProgress1, RAProgress2, RAProgress3, RAProgress4, RAProgress5,
          PTL DEF RAStableSpec, RASuccess, RAStage6
=============================================================================
