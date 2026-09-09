----------------------- MODULE EndpointRecoveryProof -----------------------
EXTENDS EndpointRecovery, TLAPS
THEOREM ERHistoryAddress == \A i, j \in 0..ERLimit :
    ERVersion[i] = ERVersion[j] => ERAddress[i] = ERAddress[j]
BY ERLegalInputs, SMT DEF ERLegalInputs
THEOREM ERInvariantInit == ERInit => ERInvariant
BY ERLegalInputs, SMT DEF ERInit, ERInvariant, ERType, ERAuthority
THEOREM ERInvariantStep == ASSUME ERInvariant, [ERNext]_erVars PROVE ERInvariant'
<1>1. ERAdvance => ERInvariant'
    BY ERLegalInputs, SMT DEF ERAdvance, ERInvariant, ERType, ERAuthority, ERRecord
<1>2. ERApply => ERInvariant'
    BY SMT DEF ERApply, ERInvariant, ERType, ERAuthority, ERRecord
<1>3. ASSUME ERConfirm PROVE ERInvariant'
    <2>1. erCommit \in 0..ERLimit /\ erCommit + 1 \in 0..ERLimit
        BY <1>3, ERLegalInputs, SMT DEF ERConfirm, ERInvariant, ERType
    <2>2. ERAddress[erCommit + 1] = erConfigured
        BY <1>3, ERHistoryAddress, <2>1, SMT DEF ERConfirm
    <2> QED BY <1>3, ERLegalInputs, <2>1, <2>2, SMT DEF ERConfirm, ERInvariant, ERType, ERAuthority, ERRecord
<1>4. ERPublish => ERInvariant'
    BY ERLegalInputs, SMT DEF ERPublish, ERInvariant, ERType, ERAuthority
<1>5. ERInitial => ERInvariant'
    BY ERLegalInputs, SMT DEF ERInitial, ERInvariant, ERType, ERAuthority
<1>6. ERServe => ERInvariant'
    BY SMT DEF ERServe, ERSavedUsable, ERInvariant, ERType, ERAuthority, ERRecord
<1>7. ERReject \/ ERForget => ERInvariant'
    BY SMT DEF ERReject, ERForget, ERInvariant, ERType, ERAuthority, ERRecord
<1>8. \A a \in ERAddresses : ERCrash(a) => ERInvariant'
    BY SMT DEF ERCrash, ERInvariant, ERType, ERAuthority, ERRecord
<1>9. UNCHANGED erVars => ERInvariant'
    BY SMT DEF erVars, ERInvariant, ERType, ERAuthority
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, SMT DEF ERNext
THEOREM ERInvariantAlways == ERSpec => []ERInvariant
BY ERInvariantInit, ERInvariantStep, PTL DEF ERSpec
THEOREM ERFreshReceiptProof == ASSUME ERInvariant PROVE ERFreshReceipt
BY SMT DEF ERFreshReceipt, ERConfirm, ERInvariant, ERType
THEOREM ERPublicationProof == ERPublication
BY SMT DEF ERPublication, ERPublish
THEOREM ERAdmissionProof == ERAdmission
BY SMT DEF ERAdmission, ERServe
THEOREM ERNoRollbackProof == ASSUME ERInvariant, ERNext PROVE ERNoRollback
BY SMT DEF ERNoRollback, ERNext, ERAdvance, ERApply, ERConfirm, ERPublish,
   ERInitial, ERServe, ERReject, ERForget, ERCrash, ERRecord, ERInvariant, ERType, ERAuthority
THEOREM EREffectsProof == ERSpec => EREffects
<1>1. ERInvariant /\ [ERNext]_erVars => [ERFreshReceipt /\ ERPublication /\ ERAdmission /\ ERNoRollback]_erVars
    BY ERFreshReceiptProof, ERPublicationProof, ERAdmissionProof, ERNoRollbackProof, SMT
<1> QED BY ERInvariantAlways, <1>1, PTL DEF ERSpec, EREffects
THEOREM ERStableOrder == ERStableSpec => []ERRecoveryOrder
<1>1. ERStableInit => ERRecoveryOrder
    BY ERLegalInputs, SMT DEF ERStableInit, ERRecoveryOrder, ERStable
<1>2. ERRecoveryOrder /\ [ERStableNext]_erVars => ERRecoveryOrder'
    BY ERLegalInputs, ERInvariantStep, SMT DEF ERRecoveryOrder, ERStable, ERStableNext, ERNext,
       ERApply, ERConfirm, ERPublish, ERServe, ERSavedUsable, ERRecord, erVars,
       ERInvariant, ERType, ERAuthority
<1> QED BY <1>1, <1>2, PTL DEF ERStableSpec
THEOREM ERNeedProgress == ERStableSpec => (ERNeed ~> (ERCatching \/ ERObserved \/ ERReady \/ erServing))
<1>1. ERNeed /\ [ERStableNext]_erVars => ERNeed' \/ ERCatching' \/ ERObserved' \/ ERReady' \/ erServing'
    BY ERLegalInputs, SMT DEF ERNeed, ERCatching, ERObserved, ERReady, ERRecoveryOrder, ERStable,
       ERStableNext, ERApply, ERConfirm, ERPublish, ERServe, ERSavedUsable, ERRecord, erVars,
       ERInvariant, ERType, ERAuthority
<1>2. ERNeed /\ ERConfirm => ERCatching' \/ ERObserved' \/ ERReady' \/ erServing'
    BY ERLegalInputs, SMT DEF ERNeed, ERCatching, ERObserved, ERReady, ERRecoveryOrder, ERStable,
       ERConfirm, ERSavedUsable, ERRecord, ERInvariant, ERType, ERAuthority
<1>3. ASSUME ERNeed PROVE ENABLED <<ERConfirm>>_erVars
    <2>1. erCommit = ERLimit - 1 /\ erCommit + 1 = ERLimit
        BY <1>3, ERLegalInputs, SMT DEF ERNeed, ERRecoveryOrder, ERStable, ERInvariant, ERType
    <2>2. ERAddress[erCommit] = erConfigured
        BY <1>3, ERHistoryAddress, ERLegalInputs, <2>1, SMT DEF ERNeed, ERRecoveryOrder, ERStable
    <2> QED BY <1>3, ERLegalInputs, <2>1, <2>2, ExpandENABLED, SMT DEF ERNeed, ERRecoveryOrder, ERStable,
       ERConfirm, ERRecord, erVars, ERInvariant, ERType
<1> QED BY <1>1, <1>2, <1>3, PTL DEF ERStableSpec, ERFair
THEOREM ERCatchingProgress == ERStableSpec => (ERCatching ~> (ERObserved \/ ERReady \/ erServing))
<1>1. ERCatching /\ [ERStableNext]_erVars => ERCatching' \/ ERObserved' \/ ERReady' \/ erServing'
    BY ERLegalInputs, SMT DEF ERCatching, ERObserved, ERReady, ERRecoveryOrder, ERStable,
       ERStableNext, ERApply, ERConfirm, ERPublish, ERServe, ERSavedUsable, ERRecord, erVars,
       ERInvariant, ERType, ERAuthority
<1>2. ERCatching /\ ERApply => ERObserved' \/ ERReady' \/ erServing'
    BY ERLegalInputs, SMT DEF ERCatching, ERObserved, ERReady, ERRecoveryOrder, ERStable,
       ERApply, ERSavedUsable, ERRecord, ERInvariant, ERType, ERAuthority
<1>3. ERCatching => ENABLED <<ERApply>>_erVars
    BY ExpandENABLED, SMT DEF ERCatching, ERRecoveryOrder, ERStable,
       ERApply, ERRecord, erVars, ERInvariant, ERType, ERAuthority
<1> QED BY <1>1, <1>2, <1>3, PTL DEF ERStableSpec, ERFair
THEOREM ERObservedProgress == ERStableSpec => (ERObserved ~> (ERReady \/ erServing))
<1>1. ERObserved /\ [ERStableNext]_erVars => ERObserved' \/ ERReady' \/ erServing'
    BY ERLegalInputs, SMT DEF ERObserved, ERReady, ERRecoveryOrder, ERStable,
       ERStableNext, ERApply, ERConfirm, ERPublish, ERServe, ERSavedUsable, ERRecord, erVars,
       ERInvariant, ERType, ERAuthority
<1>2. ERObserved /\ ERPublish => ERReady' \/ erServing'
    BY ERLegalInputs, SMT DEF ERObserved, ERReady, ERRecoveryOrder, ERStable,
       ERPublish, ERSavedUsable, ERRecord, ERInvariant, ERType, ERAuthority
<1>3. ERObserved => ENABLED <<ERPublish>>_erVars
    BY ERLegalInputs, ExpandENABLED, SMT DEF ERObserved, ERRecoveryOrder, ERStable,
       ERPublish, ERSavedUsable, ERRecord, erVars, ERInvariant, ERType, ERAuthority
<1> QED BY <1>1, <1>2, <1>3, PTL DEF ERStableSpec, ERFair
THEOREM ERReadyProgress == ERStableSpec => (ERReady ~> erServing)
<1>1. ERReady /\ [ERStableNext]_erVars => ERReady' \/ erServing'
    BY ERLegalInputs, SMT DEF ERReady, ERRecoveryOrder, ERStable,
       ERStableNext, ERApply, ERConfirm, ERPublish, ERServe, ERSavedUsable, ERRecord, erVars,
       ERInvariant, ERType, ERAuthority
<1>2. ERReady /\ ERServe => erServing'
    BY SMT DEF ERServe
<1>3. ERReady => ENABLED <<ERServe>>_erVars
    BY ExpandENABLED, SMT DEF ERReady, ERServe, ERRecord, erVars
<1> QED BY <1>1, <1>2, <1>3, PTL DEF ERStableSpec, ERFair
THEOREM ERConvergenceProof == ERStableSpec => ERConverges
<1>1. ERRecoveryOrder => ERNeed \/ ERCatching \/ ERObserved \/ ERReady \/ erServing
    BY ERLegalInputs, SMT DEF ERRecoveryOrder, ERStable, ERNeed, ERCatching, ERObserved, ERReady,
       ERInvariant, ERType
<1> QED BY <1>1, ERStableOrder, ERNeedProgress, ERCatchingProgress,
           ERObservedProgress, ERReadyProgress, PTL DEF ERConverges
=============================================================================
