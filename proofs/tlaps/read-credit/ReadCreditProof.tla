--------------------------- MODULE ReadCreditProof ---------------------------
EXTENDS ReadCredit, ReadAdmissionProof, TLAPS

THEOREM RCPollProjection == RCPoll => [RANext]_raVars
BY SMT DEF RCPoll, RANext

THEOREM RCNextProjection == [RCNext]_rcVars => [RANext]_raVars
BY RCPollProjection, SMT DEF RCNext, RCReadStep, RCReset,
   RCOtherAdmission, RCRelease, RCCancel, rcVars, RANext

THEOREM RCProjection == RCSpec => RASpec
<1>1. RCInit => RAInit
    BY SMT DEF RCInit
<1> QED BY <1>1, RCNextProjection, PTL DEF RCSpec, RASpec

THEOREM RCCreditSafety == RCSpec => []RASafeReturn
BY RCProjection, RASafeReturnProof, PTL

THEOREM RCInvariantInit == RCInit => RCInvariant
BY RAInvariantInit, SMT DEF RCInit, RCInvariant

THEOREM RCInvariantStep == ASSUME RCInvariant, [RCNext]_rcVars PROVE RCInvariant'
<1>1. RAInvariant'
    BY RCNextProjection, RAInvariantStep, SMT DEF RCInvariant
<1> QED BY <1>1, SMT DEF RCInvariant, RCNext, RCPoll, RCReadStep, RCReset,
    RCOtherAdmission, RCRelease, RCCancel, rcVars

THEOREM RCInvariantAlways == RCSpec => []RCInvariant
BY RCInvariantInit, RCInvariantStep, PTL DEF RCSpec

THEOREM RCAdmissionCreditProof == RCAdmissionHasCredit
BY SMT DEF RCAdmissionHasCredit, RCPoll, RAPoll, raVars, RAEnv, RARequest

THEOREM RCCancellationKeepsCredit == RCCancel => UNCHANGED rcOccupied
BY SMT DEF RCCancel

THEOREM RCTimeoutKeepsCredit == (RCReadStep /\ RATimeout) => UNCHANGED rcOccupied
BY SMT DEF RCReadStep

THEOREM RCReleaseProgress == RCProgressSpec => (RCBlocked ~> RCFree)
<1>1. RCBlocked /\ [RCProgressNext]_rcVars => RCBlocked' \/ RCFree'
    BY SMT DEF RCBlocked, RCFree, RCWaiting, RCInvariant,
       RCProgressNext, RCPoll, RCRelease, rcVars, raVars, RAEnv, RARequest,
       RAInvariant, RAType, RABinding, RAPhases
<1>2. RCBlocked /\ RCRelease => RCFree'
    BY SMT DEF RCBlocked, RCFree, RCWaiting, RCInvariant, RCRelease,
       raVars, RAEnv, RARequest, RAInvariant, RAType, RABinding, RAPhases
<1>3. RCBlocked => ENABLED <<RCRelease>>_rcVars
    BY ExpandENABLED, SMT DEF RCBlocked, RCWaiting, RCInvariant, RCRelease,
       rcVars, raVars, RAEnv, RARequest, RAInvariant, RAType, RABinding, RAPhases
<1> QED BY <1>1, <1>2, <1>3, PTL DEF RCProgressSpec, RCProgressFairness

THEOREM RCAdmissionProgress == RCProgressSpec => (RCFree ~> RCAdmitted)
<1>1. RCFree /\ [RCProgressNext]_rcVars => RCFree' \/ RCAdmitted'
    BY RALegalInputs, SMT DEF RCFree, RCAdmitted, RCWaiting, RCInvariant,
       RCProgressNext, RCPoll, RCRelease, RAPoll, rcVars,
       raVars, RAEnv, RARequest, RAInvariant, RAType, RABinding, RAPhases
<1>2. RCFree /\ RCPoll => RCAdmitted'
    BY RALegalInputs, SMT DEF RCFree, RCAdmitted, RCWaiting, RCInvariant,
       RCPoll, RAPoll, RAInvariant, RAType, RABinding
<1>3. RCFree => ENABLED <<RCPoll>>_rcVars
    BY ExpandENABLED, RALegalInputs, SMT DEF RCFree, RCWaiting, RCInvariant,
       RCPoll, RAPoll, RAInvariant, RAType, RABinding, RAPhases,
       RAEnv, RARequest, raVars, rcVars
<1> QED BY <1>1, <1>2, <1>3, PTL DEF RCProgressSpec, RCProgressFairness

THEOREM RCStableAdmission == RCProgressSpec => <>RCAdmitted
<1>1. RCWaiting => RCBlocked \/ RCFree
    BY SMT DEF RCWaiting, RCBlocked, RCFree
<1> QED BY <1>1, RCReleaseProgress, RCAdmissionProgress, PTL DEF RCProgressSpec
=============================================================================
