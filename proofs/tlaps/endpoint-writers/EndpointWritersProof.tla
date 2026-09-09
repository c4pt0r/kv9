----------------------- MODULE EndpointWritersProof -----------------------
EXTENDS EndpointWriters, TLAPS
THEOREM EWInvariantInit == EWInit => EWInvariant
BY EWLegalInputs, SMT DEF EWInit, EWInvariant, EWType, EWOrder
THEOREM EWInvariantStep == ASSUME EWInvariant, [EWNext]_ewVars PROVE EWInvariant'
<1>1. \A a \in EWAddresses : EWNewAdmission(a) => EWInvariant'
    BY SMT DEF EWNewAdmission, EWInvariant, EWType, EWOrder, EWDirectory, EWInstalled, EWCapture
<1>2. \A a \in EWAddresses, registration, local \in BOOLEAN : EWChange(a, registration, local) => EWInvariant'
    BY SMT DEF EWChange, EWDelta, EWInvariant, EWType, EWOrder, EWInstalled
<1>3. EWStart => EWInvariant'
    BY SMT DEF EWStart, EWInvariant, EWType, EWOrder, EWDirectory, EWAuthority, EWInstalled
<1>4. EWInstall => EWInvariant'
    BY SMT DEF EWInstall, EWInvariant, EWType, EWOrder, EWDirectory, EWAuthority
<1>5. UNCHANGED ewVars => EWInvariant'
    BY SMT DEF ewVars, EWDirectory, EWAuthority, EWInstalled, EWCapture, EWInvariant, EWType, EWOrder
<1>6. \A a \in EWAddresses : EWEager(a) => EWInvariant'
    BY EWLegalInputs, SMT DEF EWEager, EWCapture, EWInvariant, EWType, EWOrder
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, SMT DEF EWNext
THEOREM EWInvariantAlways == EWSpec => []EWInvariant
BY EWInvariantInit, EWInvariantStep, PTL DEF EWSpec
THEOREM EWVersionProof == ASSUME EWInvariant, EWNext PROVE EWVersion
BY SMT DEF EWVersion, EWNext, EWNewAdmission, EWChange, EWDelta, EWStart, EWInstall, EWEager,
   EWDirectory, EWAuthority, EWInstalled, EWCapture, EWInvariant, EWType, EWOrder
THEOREM EWInstallOrderProof == ASSUME EWInvariant, EWNext PROVE EWInstallOrder
BY SMT DEF EWInstallOrder, EWNext, EWNewAdmission, EWChange, EWDelta, EWStart, EWInstall, EWEager,
   EWDirectory, EWAuthority, EWInstalled, EWCapture, EWInvariant, EWType, EWOrder
THEOREM EWRevocationProof == EWRevocation
BY SMT DEF EWRevocation, EWChange, EWEager
THEOREM EWRegistrationProof == EWRegistration
BY SMT DEF EWRegistration, EWChange
THEOREM EWEffectsProof == EWSpec => EWEffects
<1>1. EWInvariant /\ [EWNext]_ewVars => [EWStepEffects]_ewVars
    BY EWVersionProof, EWInstallOrderProof, EWRevocationProof, EWRegistrationProof, SMT DEF EWStepEffects
<1> QED BY EWInvariantAlways, <1>1, PTL DEF EWSpec, EWEffects
THEOREM EWStableInvariant == EWStableSpec => []EWInvariant
<1>1. EWInvariant /\ [EWStableNext]_ewVars => EWInvariant'
    BY EWInvariantStep, SMT DEF EWStableNext, EWNext
<1> QED BY <1>1, PTL DEF EWStableSpec
THEOREM EWOldProgress == EWStableSpec => (EWOld ~> EWWaiting)
<1>1. EWOld /\ [EWStableNext]_ewVars => EWOld' \/ EWWaiting'
    BY EWInvariantStep, SMT DEF EWOld, EWWaiting, EWStableNext, EWNext, EWStart, EWInstall,
       EWDirectory, EWAuthority, EWInstalled, EWCapture, ewVars, EWInvariant, EWType, EWOrder
<1>2. EWOld /\ EWInstall => EWWaiting'
    BY EWInvariantStep, SMT DEF EWOld, EWWaiting, EWNext, EWInstall, EWDirectory, EWAuthority
<1>3. EWOld => ENABLED <<EWInstall>>_ewVars
    BY ExpandENABLED, SMT DEF EWOld, EWInstall, EWDirectory, EWAuthority, EWInstalled, EWCapture, ewVars
<1> QED BY <1>1, <1>2, <1>3, PTL DEF EWStableSpec, EWStableFairness
THEOREM EWWaitingProgress == EWStableSpec => (EWWaiting ~> EWFresh)
<1>1. EWWaiting /\ [EWStableNext]_ewVars => EWWaiting' \/ EWFresh'
    BY EWInvariantStep, SMT DEF EWWaiting, EWFresh, EWStableNext, EWNext, EWStart, EWInstall,
       EWDirectory, EWAuthority, EWInstalled, EWCapture, ewVars, EWInvariant, EWType, EWOrder
<1>2. EWWaiting /\ EWStart => EWFresh'
    BY EWInvariantStep, SMT DEF EWWaiting, EWFresh, EWNext, EWStart, EWDirectory, EWAuthority, EWInstalled
<1>3. EWWaiting => ENABLED <<EWStart>>_ewVars
    BY ExpandENABLED, SMT DEF EWWaiting, EWStart, EWDirectory, EWAuthority, EWInstalled, EWCapture, ewVars
<1> QED BY <1>1, <1>2, <1>3, PTL DEF EWStableSpec, EWStableFairness
THEOREM EWFreshProgress == EWStableSpec => (EWFresh ~> EWCurrent)
<1>1. EWFresh /\ [EWStableNext]_ewVars => EWFresh' \/ EWCurrent'
    BY EWInvariantStep, SMT DEF EWFresh, EWCurrent, EWStableNext, EWNext, EWStart, EWInstall,
       EWDirectory, EWAuthority, EWInstalled, EWCapture, ewVars, EWInvariant, EWType, EWOrder
<1>2. EWFresh /\ EWInstall => EWCurrent'
    BY EWInvariantStep, SMT DEF EWFresh, EWCurrent, EWNext, EWInstall, EWDirectory, EWAuthority,
       EWInvariant, EWType, EWOrder
<1>3. EWFresh => ENABLED <<EWInstall>>_ewVars
    BY ExpandENABLED, SMT DEF EWFresh, EWInstall, EWDirectory, EWAuthority, EWInstalled, EWCapture, ewVars
<1> QED BY <1>1, <1>2, <1>3, PTL DEF EWStableSpec, EWStableFairness
THEOREM EWConvergenceProof == EWStableSpec => EWConverges
<1>1. EWInvariant => EWOld \/ EWWaiting \/ EWFresh \/ EWCurrent
    BY SMT DEF EWInvariant, EWType, EWOrder, EWOld, EWWaiting, EWFresh, EWCurrent
<1> QED BY <1>1, EWOldProgress, EWWaitingProgress, EWFreshProgress, PTL DEF EWStableSpec, EWConverges
=============================================================================
