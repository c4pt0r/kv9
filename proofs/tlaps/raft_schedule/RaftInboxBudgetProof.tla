------------------------- MODULE RaftInboxBudgetProof -------------------------
EXTENDS RaftInboxBudget, TLAPS
THEOREM RBInvariantInit == RBInit => RBInvariant
BY RBLegalInputs, SMT DEF RBInit, RBInvariant, RBType
THEOREM RBAdmitStep == ASSUME RBInvariant, NEW w \in Nat, RBAdmit(w) PROVE RBInvariant'
BY RBLegalInputs, SMT DEF RBAdmit, RBInvariant, RBType
THEOREM RBBeginStep == ASSUME RBInvariant, RBBegin PROVE RBInvariant'
BY RBLegalInputs, SMT DEF RBBegin, RBInvariant, RBType
THEOREM RBPopStep == ASSUME RBInvariant, NEW w \in Nat, RBPop(w) PROVE RBInvariant'
BY RBLegalInputs, SMT DEF RBPop, RBCanPop, RBInvariant, RBType
THEOREM RBFinishStep == ASSUME RBInvariant, RBFinish PROVE RBInvariant'
BY RBLegalInputs, SMT DEF RBFinish, RBCanPop, RBInvariant, RBType
THEOREM RBInvariantStep == ASSUME RBInvariant, [RBNext]_rbVars PROVE RBInvariant'
BY RBAdmitStep, RBBeginStep, RBPopStep, RBFinishStep, SMT DEF RBNext, RBQuiesce, rbVars, RBInvariant, RBType
THEOREM RBInvariantAlways == RBSpec => []RBInvariant
BY RBInvariantInit, RBInvariantStep, PTL DEF RBSpec
THEOREM RBBoundedPopStep == ASSUME RBInvariant, RBNext PROVE RBBoundedPop
BY RBLegalInputs, SMT DEF RBBoundedPop, RBNext, RBAdmit, RBBegin, RBPop, RBFinish, RBQuiesce, RBCanPop, rbVars, RBInvariant, RBType
THEOREM RBBoundedPopAlways == RBSpec => RBSafety
<1>1. RBInvariant /\ [RBNext]_rbVars => [RBBoundedPop]_rbVars
    BY RBBoundedPopStep, SMT DEF rbVars
<1> QED BY <1>1, RBInvariantAlways, PTL DEF RBSpec, RBSafety
THEOREM RBPopStrictVariant == ASSUME RBInvariant, NEW w \in Nat, RBPop(w)
    PROVE rbCount' < rbCount /\ rbCount' \in Nat /\ rbRemovedCount' <= RBDrainMessages
BY RBLegalInputs, SMT DEF RBPop, RBCanPop, RBInvariant, RBType
THEOREM RBOvershootBoundary == ASSUME RBInvariant, NEW w \in Nat, RBPop(w)
    PROVE rbRemovedBytes' < RBDrainBytes + w
BY RBLegalInputs, SMT DEF RBPop, RBCanPop, RBInvariant, RBType
THEOREM RBRefusalPure == ASSUME RBInvariant, NEW w \in Nat,
    rbCount = RBMaxMessages \/ w > RBMaxBytes - rbBytes
    PROVE ~ENABLED RBAdmit(w)
BY ExpandENABLED, RBLegalInputs, SMT DEF RBAdmit, RBInvariant, RBType
=============================================================================
