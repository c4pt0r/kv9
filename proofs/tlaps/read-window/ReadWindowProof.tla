-------------------------- MODULE ReadWindowProof ---------------------------
EXTENDS ReadWindow, TLAPS

THEOREM RWInitialBound == RWInit => RWInvariant
BY SMT DEF RWInit, RWInvariant, RWLegal

THEOREM RWAdmissionBound == ASSUME RWInvariant, RWAdmit PROVE RWInvariant'
BY SMT DEF RWInvariant, RWLegal, RWAdmit

THEOREM RWReleaseBound == ASSUME RWInvariant, RWRelease PROVE RWInvariant'
BY SMT DEF RWInvariant, RWLegal, RWRelease

THEOREM RWResetBound == ASSUME RWInvariant, RWReset PROVE RWInvariant'
BY SMT DEF RWInvariant, RWLegal, RWReset

THEOREM RWStepBound == ASSUME RWInvariant, [RWNext]_rwPending PROVE RWInvariant'
BY RWAdmissionBound, RWReleaseBound, RWResetBound, SMT
   DEF RWNext, RWCancel, RWInvariant, RWLegal

THEOREM RWAlwaysBounded == RWSpec => []RWInvariant
BY RWInitialBound, RWStepBound, PTL DEF RWSpec

THEOREM RWAdmissionRequiresSpace == RWAdmissionEffect
BY SMT DEF RWAdmissionEffect, RWAdmit

THEOREM RWAdmissionAddsAtMostOne ==
    ASSUME RWInvariant, RWAdmit PROVE rwPending' <= rwPending + 1
BY SMT DEF RWInvariant, RWLegal, RWAdmit

THEOREM RWCancellationKeepsOccupancy == RWCancel => UNCHANGED rwPending
BY SMT DEF RWCancel
=============================================================================
