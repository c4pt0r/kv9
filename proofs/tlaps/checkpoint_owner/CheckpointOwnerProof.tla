------------------------ MODULE CheckpointOwnerProof ------------------------
EXTENDS CheckpointOwner, TLAPS
THEOREM CInvariantInit == CInit => CInvariant
BY CInputs, SMT DEF CInit, CInvariant, CType, CSlot, CCovered, CProtected, CTransfer, CSettled
THEOREM CInvariantStep == ASSUME CInvariant, [CNext]_cVars PROVE CInvariant'
<1>1. \A o \in COps : CPlan(o) => CInvariant'
    BY CInputs, SMT DEF CPlan, CInvariant, CType, CSlot, CCovered, CProtected, CTransfer, CSettled
<1>2. \A o \in COps : CAcquire(o) \/ CPin(o) => CInvariant'
    BY CInputs, SMT DEF CAcquire, CPin, CInvariant, CType, CSlot, CCovered, CProtected, CTransfer, CSettled
<1>3. \A o \in COps : CPut(o) \/ CVerify(o) \/ CApply(o) => CInvariant'
    BY CInputs, SMT DEF CPut, CVerify, CApply, CInvariant, CType, CSlot, CCovered, CProtected, CTransfer, CSettled
<1>4. \A o \in COps : CPositiveEvidence(o) \/ CNegativeEvidence(o) => CInvariant'
    BY CInputs, SMT DEF CPositiveEvidence, CNegativeEvidence, CInvariant, CType, CSlot, CCovered, CProtected, CTransfer, CSettled
<1>5. \A o \in COps : CShare(o) \/ CPublishVersion(o) => CInvariant'
    BY CInputs, SMT DEF CShare, CPublishVersion, CInvariant, CType, CSlot, CCovered, CProtected, CTransfer, CSettled
<1>6. \A o \in COps : CQuiesce(o) \/ CRelease(o) => CInvariant'
    BY CInputs, SMT DEF CQuiesce, CRelease, CInvariant, CType, CSlot, CCovered, CProtected, CTransfer, CSettled
<1>7. \A o \in COps : CClear(o) => CInvariant'
    BY CInputs, SMT DEF CClear, CInvariant, CType, CSlot, CCovered, CProtected, CTransfer, CSettled
<1>8. UNCHANGED cVars => CInvariant'
    BY DEF cVars, CInvariant, CType, CSlot, CCovered, CProtected, CTransfer, CSettled
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, SMT DEF CNext, CRestart
THEOREM CInvariantAlways == CSpec => []CInvariant
BY CInvariantInit, CInvariantStep, PTL DEF CSpec
THEOREM CRemoteIOCovered == CInvariant => CCovered
BY DEF CInvariant
THEOREM COnlySettledJournalClears == CInvariant => CSettled
BY DEF CInvariant
THEOREM CNoIdentityReplacement ==
    ASSUME NEW o \in COps, CInvariant, CPlan(o) PROVE cJournal = CNone
BY DEF CPlan
THEOREM CCrashKeepsIdentityAndOwners == CRestart => UNCHANGED <<cJournal,cPending,cVersion>>
BY DEF CRestart, cVars
=============================================================================
