------------------------ MODULE CheckpointBaseProof ------------------------
EXTENDS CheckpointBase, TLAPS
THEOREM CBInvariantInit == CBInit => CBInvariant
BY CBInputs, SMT DEF CBInit, CBInvariant, CBType, CBSameImage, CBBaseGate, CBGood, CBPhaseGate
THEOREM CBInvariantStep == ASSUME CBInvariant, [CBNext]_cbVars PROVE CBInvariant'
<1>1. CBAdvance \/ CBFreeze \/ CBMint \/ CBRestore => CBInvariant'
    BY CBInputs, SMT DEF CBAdvance, CBFreeze, CBMint, CBRestore, CBInvariant, CBType, CBSameImage, CBBaseGate, CBGood, CBPhaseGate
<1>2. CBCheck \/ CBReplay \/ CBFinish => CBInvariant'
    BY CBInputs, SMT DEF CBCheck, CBReplay, CBFinish, CBInvariant, CBType, CBSameImage, CBBaseGate, CBGood, CBPhaseGate
<1>3. CBGrant \/ CBFail \/ UNCHANGED cbVars => CBInvariant'
    BY CBInputs, SMT DEF CBGrant, CBFail, cbVars, CBInvariant, CBType, CBSameImage, CBBaseGate, CBGood, CBPhaseGate
<1> QED BY <1>1, <1>2, <1>3, SMT DEF CBNext
THEOREM CBInvariantAlways == CBSpec => []CBInvariant
BY CBInvariantInit, CBInvariantStep, PTL DEF CBSpec
THEOREM CBDescriptorUsesFrozenImage == CBInvariant /\ cbMinted => cbScope = CBImageEpoch[cbFrozen]
BY DEF CBInvariant, CBSameImage
THEOREM CBTailCannotAuthorizeWrongBase == CBInvariant /\ cbChecked =>
    CBImageRoot[cbFrozen] = CBExpectedRoot /\ CBImageValid[cbFrozen]
BY DEF CBInvariant, CBBaseGate, CBGood
THEOREM CBServingRequiresCompletedPublication == CBInvariant /\ cbPhase = "served" =>
    cbComplete /\ cbChecked /\ CBPublication /\ CBGood
BY SMT DEF CBInvariant, CBPhaseGate, CBBaseGate
THEOREM CBFrozenImageSurvivesLaterWrites ==
    ASSUME CBInvariant, cbPhase # "idle", [CBNext]_cbVars PROVE cbFrozen' = cbFrozen
BY SMT DEF CBNext, CBAdvance, CBFreeze, CBMint, CBRestore, CBCheck, CBReplay, CBFinish, CBGrant, CBFail, cbVars
=============================================================================
