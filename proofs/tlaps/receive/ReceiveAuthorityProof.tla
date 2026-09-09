----------------------- MODULE ReceiveAuthorityProof -----------------------
EXTENDS ReceiveAuthority, TLAPS
THEOREM GRInvariantInit == GRInit => GRInvariant
BY GRLegalInputs, SMT DEF GRInit, GRInvariant, GRType, GRCredentials,
   GRPermission, GRRouting, GRSafety

THEOREM GRInvariantStep == ASSUME GRInvariant, [GRNext]_grVars PROVE GRInvariant'
<1>1. \A i \in GRIncarnations : GRBind(i) => GRInvariant'
    BY GRLegalInputs, SMT DEF GRBind, GRInvariant, GRType, GRCredentials, GRPermission, GRRouting, GRSafety
<1>2. \A i \in GRIncarnations : GRRoute(i) => GRInvariant'
    BY SMT DEF GRRoute, GRInvariant, GRType, GRCredentials, GRPermission, GRRouting, GRSafety
<1>3. GRReply \/ GRGrant \/ GRStart \/ GRReceive \/ GRCertify => GRInvariant'
    BY GRLegalInputs, SMT DEF GRReply, GRGrant, GRStart, GRReceive, GRCertify, GRInvariant,
       GRType, GRCredentials, GRPermission, GRRouting, GRSafety
<1>4. GRCrash => GRInvariant'
    BY SMT DEF GRCrash, GRInvariant, GRType, GRCredentials, GRPermission, GRRouting, GRSafety
<1>5. \A i \in GRIncarnations : GRRestart(i) => GRInvariant'
    BY GRLegalInputs, SMT DEF GRRestart, GRInvariant, GRType, GRCredentials, GRPermission, GRRouting, GRSafety
<1>6. UNCHANGED grVars => GRInvariant'
    BY SMT DEF grVars, GRInvariant, GRType, GRCredentials, GRPermission, GRRouting, GRSafety
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, SMT DEF GRNext, GRQuiesce

THEOREM GRInvariantAlways == GRSpec => []GRInvariant
BY GRInvariantInit, GRInvariantStep, PTL DEF GRSpec

THEOREM GRReceiveAuthorizedAlways == GRSpec => []GRSafety
<1>1. GRInvariant => GRSafety BY SMT DEF GRInvariant
<1> QED BY GRInvariantAlways, <1>1, PTL

THEOREM GRPermissionAlways == GRSpec => []GRPermission
<1>1. GRInvariant => GRPermission BY SMT DEF GRInvariant
<1> QED BY GRInvariantAlways, <1>1, PTL

THEOREM GRRoutingAlways == GRSpec => []GRRouting
<1>1. GRInvariant => GRRouting BY SMT DEF GRInvariant
<1> QED BY GRInvariantAlways, <1>1, PTL

THEOREM GRBindingStepProof == ASSUME GRInvariant, GRNext PROVE GRBindingStep
BY SMT DEF GRNext, GRBindingStep, GRBind, GRRoute, GRRestart, GRReply,
   GRGrant, GRStart, GRReceive, GRCertify, GRCrash, GRQuiesce, grVars

THEOREM GRBindingProof == GRSpec => GRBindingAlways
<1>1. GRInvariant /\ [GRNext]_grVars => [GRBindingStep]_grVars
    BY GRBindingStepProof, SMT DEF grVars, GRBindingStep
<1> QED BY GRInvariantAlways, <1>1, PTL DEF GRSpec, GRBindingAlways
THEOREM GRStableInvariant == GRFairSpec => []GRInvariant
<1>1. GRStableNext => GRNext BY SMT DEF GRStableNext, GRNext
<1>2. GRInvariant /\ [GRStableNext]_grVars => GRInvariant'
    BY <1>1, GRInvariantStep, SMT
<1> QED BY <1>2, PTL DEF GRFairSpec

THEOREM GRGrantProgress == GRFairSpec => (GRReady ~> GRGated)
<1>1. GRReady /\ [GRStableNext]_grVars => GRReady'
    BY GRLegalInputs, GRInvariantStep, SMT DEF GRReady, GRStableNext, GRNext,
       GRBind, GRRoute, GRReply, GRGrant, GRStart, GRReceive, GRCertify, GRQuiesce, grVars
<1>2. GRReady /\ GRGrant => GRGated'
    BY <1>1, SMT DEF GRGated, GRGrant, GRStableNext, grVars
<1>3. GRReady /\ ~grGate => ENABLED <<GRGrant>>_grVars
    BY ExpandENABLED, SMT DEF GRReady, GRGrant, grVars
<1> QED BY <1>1, <1>2, <1>3, PTL DEF GRFairSpec, GRGated

THEOREM GRStartProgress == GRFairSpec => (GRGated ~> GRRunning)
<1>1. GRGated /\ [GRStableNext]_grVars => GRGated'
    BY GRLegalInputs, GRInvariantStep, SMT DEF GRReady, GRGated, GRStableNext, GRNext,
       GRBind, GRRoute, GRReply, GRGrant, GRStart, GRReceive, GRCertify, GRQuiesce, grVars
<1>2. GRGated /\ GRStart => GRRunning'
    BY <1>1, SMT DEF GRRunning, GRStart, GRStableNext, grVars
<1>3. GRGated /\ ~grOwner => ENABLED <<GRStart>>_grVars
    BY ExpandENABLED, SMT DEF GRGated, GRReady, GRStart, grVars
<1> QED BY <1>1, <1>2, <1>3, PTL DEF GRFairSpec, GRRunning

THEOREM GRDeliveryProgress == GRFairSpec => (GRRunning ~> GRDelivered)
<1>1. GRRunning /\ [GRStableNext]_grVars => GRRunning'
    BY GRLegalInputs, GRInvariantStep, SMT DEF GRReady, GRGated, GRRunning, GRStableNext, GRNext,
       GRBind, GRRoute, GRReply, GRGrant, GRStart, GRReceive, GRCertify, GRQuiesce, grVars
<1>2. GRRunning /\ GRReceive => GRDelivered'
    BY <1>1, SMT DEF GRDelivered, GRReceive, GRStableNext, grVars
<1>3. GRRunning /\ ~(grLocal \in grReceived) => ENABLED <<GRReceive>>_grVars
    BY ExpandENABLED, SMT DEF GRRunning, GRGated, GRReady, GRReceive, grVars
<1> QED BY <1>1, <1>2, <1>3, PTL DEF GRFairSpec, GRDelivered

THEOREM GRProgressProof == GRFairSpec => GRProgress
BY GRStableInvariant, GRGrantProgress, GRStartProgress, GRDeliveryProgress, PTL
   DEF GRProgress, GRReady, GRDelivered, GRRunning, GRGated
=============================================================================
