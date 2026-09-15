------------------------ MODULE AnchorBindingProof ------------------------
EXTENDS AnchorBinding, TLAPS
THEOREM AInvariantInit == AInit => AInvariant
BY AInputs, SMT DEF AInit, AInvariant, AType, ASameOpen, APhases, ACapSafety
THEOREM AInvariantStep == ASSUME AInvariant, [ANext]_aVars PROVE AInvariant'
<1>1. ABegin \/ ABase => AInvariant'
    BY AInputs, SMT DEF ABegin, ABase, AInvariant, AType, ASameOpen, APhases, ACapSafety
<1>2. ARecovered \/ ABind => AInvariant'
    BY AInputs, SMT DEF ARecovered, ABind, AInvariant, AType, ASameOpen, APhases, ACapSafety
<1>3. ADecode \/ AFail \/ UNCHANGED aVars => AInvariant'
    BY AInputs, SMT DEF ADecode, AFail, aVars, AInvariant, AType, ASameOpen, APhases, ACapSafety
<1> QED BY <1>1, <1>2, <1>3, SMT DEF ANext
THEOREM AInvariantAlways == ASpec => []AInvariant
BY AInvariantInit, AInvariantStep, PTL DEF ASpec
THEOREM ALocalObservationBindsOneImage == AInvariant /\ aCap # 0 =>
    aCap = aSelected /\ aCap = aBase /\ aCap = aPublication
BY DEF AInvariant, ACapSafety
THEOREM ADecodeCannotMintLocalObservation == ADecode => aCap' = aCap
BY DEF ADecode
THEOREM ALocalObservationRequiresCompletion == AInvariant /\ aCap # 0 =>
    aComplete /\ ARootOK[aCap] /\ AConfigurationOK[aCap] /\ APublicationOK[aCap] /\ AFrameOK[aCap]
BY DEF AInvariant, ACapSafety
THEOREM AEncodedFrameBound ==
    ASSUME NEW r \in Nat, NEW m \in Nat, NEW n1 \in Nat, NEW n2 \in Nat,
           NEW n3 \in Nat, NEW n4 \in Nat, NEW o \in Nat,
           r <= 65536, m <= 1048576, n1 <= 1024, n2 <= 1024,
           n3 <= 1024, n4 <= 1024, o <= 16
    PROVE AFrameSize(r,m,n1,n2,n3,n4,o) <= 1147128
BY SMT DEF AFrameSize
=============================================================================
