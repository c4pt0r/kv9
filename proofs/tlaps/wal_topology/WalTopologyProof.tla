---------------------- MODULE WalTopologyProof ----------------------
EXTENDS WalTopology, TLAPS

THEOREM WTInvariantInit == WTInit => WTInvariant
BY WTLegalInputs, SMT DEF WTInit, WTInitialTopology, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments

THEOREM WTAllAccounted == ASSUME WTInvariant, wtPhase = "ready"
    PROVE \A s \in WTSegments : wtData[s] \subseteq WTContents(wtLocal)
<1>1. SUFFICES ASSUME NEW s \in WTSegments PROVE wtData[s] \subseteq WTContents(wtLocal)
    OBVIOUS
<1>2. CASE s < wtLocal.first
    BY <1>1, <1>2, WTLegalInputs, SMT DEF WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
<1>3. CASE wtLocal.first <= s /\ s <= wtLocal.active
    BY <1>1, <1>3, WTLegalInputs, SMT DEF WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
<1>4. CASE s > wtLocal.active
    BY <1>1, <1>4, WTLegalInputs, SMT DEF WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
<1>5. s \in Nat /\ wtLocal.first \in Nat /\ wtLocal.active \in Nat
    <2>1. wtLocal \in WTTopologies
        BY SMT DEF WTInvariant, WTType
    <2>2. wtLocal.first \in WTSegments /\ wtLocal.active \in WTSegments
        BY <2>1, SMT DEF WTTopologies
    <2> QED BY <1>1, <2>2, WTLegalInputs, SMT DEF WTSegments
<1>6. s < wtLocal.first \/ (wtLocal.first <= s /\ s <= wtLocal.active) \/ s > wtLocal.active
    BY <1>5, SMT
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, SMT

THEOREM WTCheckpointContent == ASSUME WTInvariant, wtPhase = "ready",
    NEW c \in WTAnchors, NEW f \in WTSegments, WTCheckpointChoice(c, f)
    PROVE WTContents([anchor |-> c, first |-> f, active |-> wtLocal.active]) = WTContents(wtLocal)
<1>1. WTContents([anchor |-> c, first |-> f, active |-> wtLocal.active]) \subseteq WTContents(wtLocal)
    BY WTLegalInputs, SMT DEF WTCheckpointChoice, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
<1>2. WTContents(wtLocal) \subseteq WTContents([anchor |-> c, first |-> f, active |-> wtLocal.active])
    BY WTLegalInputs, SMT DEF WTCheckpointChoice, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
<1> QED BY <1>1, <1>2, SMT

THEOREM WTDisjointAppend ==
    ASSUME NEW data \in [WTSegments -> SUBSET WTRecords], NEW a \in WTSegments, NEW r \in WTRecords,
           \A i, j \in WTSegments : i # j => data[i] \cap data[j] = {},
           \A k \in WTSegments : r \notin data[k]
    PROVE \A i, j \in WTSegments : i # j =>
        [data EXCEPT ![a] = @ \cup {r}][i] \cap [data EXCEPT ![a] = @ \cup {r}][j] = {}
<1>1. SUFFICES ASSUME NEW i \in WTSegments, NEW j \in WTSegments, i # j
        PROVE [data EXCEPT ![a] = @ \cup {r}][i] \cap [data EXCEPT ![a] = @ \cup {r}][j] = {}
    OBVIOUS
<1>2. /\ data[i] \cap data[j] = {} /\ r \notin data[i] /\ r \notin data[j]
    BY <1>1, SMT
<1>3. CASE i = a
    BY <1>1, <1>2, <1>3, SMT
<1>4. CASE j = a
    BY <1>1, <1>2, <1>4, SMT
<1>5. CASE i # a /\ j # a
    BY <1>1, <1>2, <1>5, SMT
<1> QED BY <1>3, <1>4, <1>5, SMT

THEOREM WTAckStep == ASSUME WTInvariant, NEW r \in WTRecords, WTAck(r) PROVE WTInvariant'
<1>1. WTType'
    BY WTLegalInputs, SMT DEF WTAck, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
<1>2. WTLayouts'
    <2>1. \A s \in WTSegments : r \notin wtData[s]
        BY WTAllAccounted, SMT DEF WTAck
    <2>2. \A i, j \in WTSegments : i # j => wtData'[i] \cap wtData'[j] = {}
        BY <2>1, WTDisjointAppend, WTLegalInputs, SMT DEF WTAck, WTInvariant, WTType, WTLayouts, WTAuthority, WTTopologies, WTSegments
    <2>3. WTValid(wtVisible)' /\ WTValid(wtDurable)' /\ WTValid(wtCandidate)'
        BY <2>1, WTLegalInputs, SMT DEF WTAck, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
    <2>4. WTAvailable(wtVisible)' /\ WTAvailable(wtDurable)'
        BY <2>1, WTLegalInputs, SMT DEF WTAck, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
    <2>5. wtVisible'.active >= wtDurable'.active /\ wtCandidate'.active >= wtDurable'.active /\ (\A s \in WTSegments : s > wtDurable'.active => wtData'[s] = {})
        BY <2>1, WTLegalInputs, SMT DEF WTAck, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
    <2>6. \A t \in {wtVisible', wtDurable', wtCandidate'} : t.anchor # 0 => WTContents(t)' \cap WTUnpositioned = {}
        BY <2>1, WTLegalInputs, SMT DEF WTAck, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
    <2> QED BY <2>2, <2>3, <2>4, <2>5, <2>6 DEF WTLayouts
<1>3. WTAckRecoverable'
    BY WTLegalInputs, SMT DEF WTAck, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
<1>4. WTCandidateSafe'
    BY WTLegalInputs, SMT DEF WTAck, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
<1>5. WTAuthority'
    BY WTLegalInputs, SMT DEF WTAck, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
<1>6. WTPlanAuthority'
    BY WTLegalInputs, SMT DEF WTAck, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
<1>7. WTStage'
    BY WTLegalInputs, SMT DEF WTAck, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7 DEF WTInvariant

THEOREM WTUnknownStep == ASSUME WTInvariant, NEW r \in WTRecords, WTUnknown(r) PROVE WTInvariant'
<1>1. WTType'
    BY WTLegalInputs, SMT DEF WTUnknown, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
<1>2. WTLayouts'
    <2>1. \A s \in WTSegments : r \notin wtData[s]
        BY WTAllAccounted, SMT DEF WTUnknown
    <2>2. \A i, j \in WTSegments : i # j => wtData'[i] \cap wtData'[j] = {}
        <3>1. /\ wtData \in [WTSegments -> SUBSET WTRecords] /\ wtWriter \in WTSegments /\ r \in WTRecords
            <4>1. wtLocal \in WTTopologies /\ wtData \in [WTSegments -> SUBSET WTRecords]
                BY SMT DEF WTInvariant, WTType
            <4>2. wtWriter = wtLocal.active /\ r \in WTRecords
                BY SMT DEF WTUnknown, WTInvariant, WTAuthority
            <4>3. wtLocal.active \in WTSegments
                BY <4>1, SMT DEF WTTopologies
            <4> QED BY <4>1, <4>2, <4>3
        <3>2. \A i, j \in WTSegments : i # j => wtData[i] \cap wtData[j] = {}
            BY SMT DEF WTInvariant, WTLayouts
        <3>3. \A i, j \in WTSegments : i # j =>
            [wtData EXCEPT ![wtWriter] = @ \cup {r}][i] \cap [wtData EXCEPT ![wtWriter] = @ \cup {r}][j] = {}
            BY <2>1, <3>1, <3>2, WTDisjointAppend, SMT
        <3> QED BY <3>3, SMT DEF WTUnknown
    <2>3. WTValid(wtVisible)' /\ WTValid(wtDurable)' /\ WTValid(wtCandidate)'
        BY <2>1, WTLegalInputs, SMT DEF WTUnknown, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
    <2>4. WTAvailable(wtVisible)' /\ WTAvailable(wtDurable)'
        BY <2>1, WTLegalInputs, SMT DEF WTUnknown, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
    <2>5. wtVisible'.active >= wtDurable'.active /\ wtCandidate'.active >= wtDurable'.active /\ (\A s \in WTSegments : s > wtDurable'.active => wtData'[s] = {})
        BY <2>1, WTLegalInputs, SMT DEF WTUnknown, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
    <2>6. \A t \in {wtVisible', wtDurable', wtCandidate'} : t.anchor # 0 => WTContents(t)' \cap WTUnpositioned = {}
        BY <2>1, WTLegalInputs, SMT DEF WTUnknown, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
    <2> QED BY <2>2, <2>3, <2>4, <2>5, <2>6 DEF WTLayouts
<1>3. WTAckRecoverable'
    BY WTLegalInputs, SMT DEF WTUnknown, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
<1>4. WTCandidateSafe'
    BY WTLegalInputs, SMT DEF WTUnknown, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
<1>5. WTAuthority'
    BY WTLegalInputs, SMT DEF WTUnknown, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
<1>6. WTPlanAuthority'
    BY WTLegalInputs, SMT DEF WTUnknown, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
<1>7. WTStage'
    BY WTLegalInputs, SMT DEF WTUnknown, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7 DEF WTInvariant

THEOREM WTRotateStep == ASSUME WTInvariant PROVE WTRotate => WTInvariant'
BY WTLegalInputs, SMT DEF WTRotate, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments

THEOREM WTCreateStep == ASSUME WTInvariant, WTCreate PROVE WTInvariant'
<1>1. WTType'
    BY WTLegalInputs, SMT DEF WTCreate, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
<1>2. WTLayouts'
    BY WTLegalInputs, SMT DEF WTCreate, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
<1>3. WTAckRecoverable'
    BY WTLegalInputs, SMT DEF WTCreate, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
<1>4. WTCandidateSafe'
    BY WTLegalInputs, SMT DEF WTCreate, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
<1>5. WTAuthority'
    BY WTLegalInputs, SMT DEF WTCreate, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
<1>6. WTPlanAuthority'
    BY WTLegalInputs, SMT DEF WTCreate, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
<1>7. WTStage'
    <2>1. WTSelected(wtCandidate) \subseteq WTSelected(wtDurable) \cup {wtCandidate.active}
        <3>1. /\ wtCandidate.first = wtDurable.first /\ wtCandidate.active = wtDurable.active + 1
               /\ wtDurable.first \in Nat /\ wtDurable.active \in Nat
            BY WTLegalInputs, SMT DEF WTCreate, WTInvariant, WTType, WTStage, WTTopologies, WTSegments
        <3>2. \A k \in WTSelected(wtCandidate) : k \in WTSelected(wtDurable) \/ k = wtCandidate.active
            BY <3>1, SMT DEF WTSelected
        <3> QED BY <3>2, SMT
    <2>2. WTAvailable(wtCandidate)'
        BY <2>1, WTLegalInputs, SMT DEF WTCreate, WTInvariant, WTLayouts, WTAvailable
    <2> QED BY <2>2, WTLegalInputs, SMT DEF WTCreate, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7 DEF WTInvariant

THEOREM WTCheckpointStep == ASSUME WTInvariant, NEW c \in WTAnchors, NEW f \in WTSegments, WTCheckpoint(c, f) PROVE WTInvariant'
<1>1. WTType'
    BY WTLegalInputs, SMT DEF WTCheckpoint, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments, WTCheckpointChoice
<1>2. WTLayouts'
    BY WTLegalInputs, SMT DEF WTCheckpoint, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments, WTCheckpointChoice
<1>3. WTAckRecoverable'
    BY WTLegalInputs, SMT DEF WTCheckpoint, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments, WTCheckpointChoice
<1>4. WTCandidateSafe'
    BY WTLegalInputs, SMT DEF WTCheckpoint, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments, WTCheckpointChoice
<1>5. WTAuthority'
    BY WTLegalInputs, SMT DEF WTCheckpoint, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments, WTCheckpointChoice
<1>6. WTPlanAuthority'
    BY WTLegalInputs, SMT DEF WTCheckpoint, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments, WTCheckpointChoice
<1>7. WTStage'
    <2>1. WTContents(wtCandidate)' = WTContents(wtLocal)
        BY WTCheckpointContent, WTLegalInputs, SMT DEF WTCheckpoint, WTContents, WTSelected
    <2> QED BY <2>1, WTLegalInputs, SMT DEF WTCheckpoint, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments, WTCheckpointChoice
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7 DEF WTInvariant

THEOREM WTTemporarySyncStep == ASSUME WTInvariant PROVE WTTemporarySync => WTInvariant'
BY WTLegalInputs, SMT DEF WTTemporarySync, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments

THEOREM WTRenameStep == ASSUME WTInvariant PROVE WTRename => WTInvariant'
BY WTLegalInputs, SMT DEF WTRename, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments

THEOREM WTParentSyncStep == ASSUME WTInvariant PROVE WTParentSync => WTInvariant'
BY WTLegalInputs, SMT DEF WTParentSync, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments

THEOREM WTInstallStep == ASSUME WTInvariant PROVE WTInstall => WTInvariant'
BY WTLegalInputs, SMT DEF WTInstall, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments

THEOREM WTFailStep == ASSUME WTInvariant PROVE WTFail => WTInvariant'
BY WTLegalInputs, SMT DEF WTFail, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments

THEOREM WTCrashStep == ASSUME WTInvariant, NEW t \in WTTopologies, NEW files \in SUBSET WTSegments PROVE WTCrash(t, files) => WTInvariant'
BY WTLegalInputs, SMT DEF WTCrash, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments

THEOREM WTReadStep == ASSUME WTInvariant PROVE WTRead => WTInvariant'
BY WTLegalInputs, SMT DEF WTRead, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments

THEOREM WTRestoreStep == ASSUME WTInvariant PROVE WTRestore => WTInvariant'
BY WTLegalInputs, SMT DEF WTRestore, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments

THEOREM WTRefuseStep == ASSUME WTInvariant PROVE WTRefuse => WTInvariant'
BY WTLegalInputs, SMT DEF WTRefuse, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments

THEOREM WTRecoverySyncStep == ASSUME WTInvariant, WTRecoverySync PROVE WTInvariant'
<1>1. WTType'
    BY WTLegalInputs, SMT DEF WTRecoverySync, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
<1>2. WTLayouts'
    BY WTLegalInputs, SMT DEF WTRecoverySync, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
<1>3. WTAckRecoverable'
    BY WTLegalInputs, SMT DEF WTRecoverySync, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
<1>4. WTCandidateSafe'
    BY WTLegalInputs, SMT DEF WTRecoverySync, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
<1>5. WTAuthority'
    BY WTLegalInputs, SMT DEF WTRecoverySync, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
<1>6. WTPlanAuthority'
    BY WTLegalInputs, SMT DEF WTRecoverySync, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
<1>7. WTStage'
    BY WTLegalInputs, SMT DEF WTRecoverySync, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7 DEF WTInvariant

THEOREM WTReplayStep == ASSUME WTInvariant PROVE WTReplay => WTInvariant'
BY WTLegalInputs, SMT DEF WTReplay, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments, WTTail

THEOREM WTUnlinkStep == ASSUME WTInvariant, NEW s \in WTSegments PROVE WTUnlink(s) => WTInvariant'
BY WTLegalInputs, SMT DEF WTUnlink, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments

THEOREM WTCleanupSyncStep == ASSUME WTInvariant PROVE WTCleanupSync => WTInvariant'
BY WTLegalInputs, SMT DEF WTCleanupSync, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments

THEOREM WTInvariantStep == ASSUME WTInvariant, [WTNext]_wtVars PROVE WTInvariant'
BY WTAckStep, WTUnknownStep, WTRotateStep, WTCreateStep, WTCheckpointStep, WTTemporarySyncStep, WTRenameStep, WTParentSyncStep, WTInstallStep, WTFailStep, WTCrashStep, WTReadStep, WTRestoreStep, WTRefuseStep, WTRecoverySyncStep, WTReplayStep, WTUnlinkStep, WTCleanupSyncStep, SMT DEF WTNext, WTQuiesce, wtVars, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments

THEOREM WTInvariantAlways == WTSpec => []WTInvariant
BY WTInvariantInit, WTInvariantStep, PTL DEF WTSpec

THEOREM WTAcknowledgedRecovery == WTSpec => []WTAckRecoverable
<1>1. WTInvariant => WTAckRecoverable BY DEF WTInvariant
<1> QED BY <1>1, WTInvariantAlways, PTL

THEOREM WTWriterAuthority == WTSpec => []WTAuthority
<1>1. WTInvariant => WTAuthority BY DEF WTInvariant
<1> QED BY <1>1, WTInvariantAlways, PTL

THEOREM WTOrphanExclusion == WTSpec => []WTPlanAuthority
<1>1. WTInvariant => WTPlanAuthority BY DEF WTInvariant
<1> QED BY <1>1, WTInvariantAlways, PTL

THEOREM WTDeletionStep == ASSUME WTInvariant, WTNext PROVE WTDeleteSafe
BY WTLegalInputs, SMT DEF WTDeleteSafe, WTNext, WTAck, WTUnknown, WTRotate, WTCreate, WTCheckpoint, WTTemporarySync, WTRename, WTParentSync, WTInstall, WTFail, WTCrash, WTRead, WTRestore, WTRefuse, WTRecoverySync, WTReplay, WTUnlink, WTCleanupSync, WTQuiesce, wtVars, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments, WTCheckpointChoice, WTTail

THEOREM WTDeletionAlways == WTSpec => [][WTDeleteSafe]_wtVars
<1>1. WTInvariant /\ [WTNext]_wtVars => [WTDeleteSafe]_wtVars
    BY WTDeletionStep, SMT DEF wtVars
<1> QED BY <1>1, WTInvariantAlways, PTL DEF WTSpec

THEOREM WTClosedContentsStep == ASSUME WTInvariant, WTNext PROVE WTClosedContentsSafe
BY WTLegalInputs, SMT DEF WTClosedContentsSafe, WTClosed, WTNext, WTAck, WTUnknown, WTRotate, WTCreate, WTCheckpoint, WTTemporarySync, WTRename, WTParentSync, WTInstall, WTFail, WTCrash, WTRead, WTRestore, WTRefuse, WTRecoverySync, WTReplay, WTUnlink, WTCleanupSync, WTQuiesce, wtVars, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments, WTCheckpointChoice, WTTail

THEOREM WTClosedContentsPreserved == WTSpec => [][WTClosedContentsSafe]_wtVars
<1>1. WTInvariant /\ [WTNext]_wtVars => [WTClosedContentsSafe]_wtVars
    BY WTClosedContentsStep, SMT DEF wtVars
<1> QED BY <1>1, WTInvariantAlways, PTL DEF WTSpec

THEOREM WTAckMonotonicStep == ASSUME WTInvariant, WTNext PROVE WTAckMonotonic
BY WTLegalInputs, SMT DEF WTAckMonotonic, WTClosed, WTNext, WTAck, WTUnknown, WTRotate, WTCreate, WTCheckpoint, WTTemporarySync, WTRename, WTParentSync, WTInstall, WTFail, WTCrash, WTRead, WTRestore, WTRefuse, WTRecoverySync, WTReplay, WTUnlink, WTCleanupSync, WTQuiesce, wtVars, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments, WTCheckpointChoice, WTTail

THEOREM WTAckMonotonicPreserved == WTSpec => [][WTAckMonotonic]_wtVars
<1>1. WTInvariant /\ [WTNext]_wtVars => [WTAckMonotonic]_wtVars
    BY WTAckMonotonicStep, SMT DEF wtVars
<1> QED BY <1>1, WTInvariantAlways, PTL DEF WTSpec

THEOREM WTAcknowledgementStep == ASSUME WTInvariant, WTNext PROVE WTNoEarlyAck
BY WTLegalInputs, SMT DEF WTNoEarlyAck, WTNext, WTAck, WTUnknown, WTRotate, WTCreate, WTCheckpoint, WTTemporarySync, WTRename, WTParentSync, WTInstall, WTFail, WTCrash, WTRead, WTRestore, WTRefuse, WTRecoverySync, WTReplay, WTUnlink, WTCleanupSync, WTQuiesce, wtVars, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments, WTCheckpointChoice, WTTail

THEOREM WTAcknowledgementAlways == WTSpec => [][WTNoEarlyAck]_wtVars
<1>1. WTInvariant /\ [WTNext]_wtVars => [WTNoEarlyAck]_wtVars
    BY WTAcknowledgementStep, SMT DEF wtVars
<1> QED BY <1>1, WTInvariantAlways, PTL DEF WTSpec

THEOREM WTFencingStep == ASSUME WTInvariant, WTNext PROVE WTFenceStep
BY WTLegalInputs, SMT DEF WTFenceStep, WTNext, WTAck, WTUnknown, WTRotate, WTCreate, WTCheckpoint, WTTemporarySync, WTRename, WTParentSync, WTInstall, WTFail, WTCrash, WTRead, WTRestore, WTRefuse, WTRecoverySync, WTReplay, WTUnlink, WTCleanupSync, WTQuiesce, wtVars, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments, WTCheckpointChoice, WTTail

THEOREM WTFencingAlways == WTSpec => [][WTFenceStep]_wtVars
<1>1. WTInvariant /\ [WTNext]_wtVars => [WTFenceStep]_wtVars
    BY WTFencingStep, SMT DEF wtVars
<1> QED BY <1>1, WTInvariantAlways, PTL DEF WTSpec

THEOREM WTReplayExact == ASSUME WTInvariant, WTReplay PROVE wtView' = WTContents(wtPlan)
BY WTLegalInputs, SMT DEF WTReplay, WTTail, WTContents, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments

THEOREM WTStableInvariant == WTFairSpec => []WTInvariant
<1>1. WTStableNext => WTNext BY SMT DEF WTStableNext, WTNext
<1>2. WTInvariant /\ [WTStableNext]_wtVars => WTInvariant'
    BY <1>1, WTInvariantStep, SMT
<1> QED BY <1>2, PTL DEF WTFairSpec

THEOREM WTCreateProgress == WTFairSpec => ((wtPhase = "create") ~> (wtPhase = "write_top"))
<1>1. WTInvariant /\ (wtPhase = "create") /\ [WTStableNext]_wtVars =>
          WTInvariant' /\ ((wtPhase = "create")' \/ (wtPhase = "write_top")')
    BY WTInvariantStep, WTLegalInputs, SMT DEF WTNext, WTStableNext, WTQuiesce, wtVars, WTCreate, WTTemporarySync, WTRename, WTParentSync, WTInstall, WTRestore, WTRefuse, WTRecoverySync, WTReplay
<1>2. WTInvariant /\ (wtPhase = "create") /\ WTCreate => (wtPhase = "write_top")'
    BY SMT DEF WTCreate
<1>3. WTInvariant /\ (wtPhase = "create") => ENABLED <<WTCreate>>_wtVars
    BY ExpandENABLED, WTLegalInputs, Isa DEF WTCreate, wtVars
<1> QED BY WTStableInvariant, <1>1, <1>2, <1>3, PTL DEF WTFairSpec

THEOREM WTTemporarySyncProgress == WTFairSpec => ((wtPhase = "write_top") ~> (wtPhase = "rename"))
<1>1. WTInvariant /\ (wtPhase = "write_top") /\ [WTStableNext]_wtVars =>
          WTInvariant' /\ ((wtPhase = "write_top")' \/ (wtPhase = "rename")')
    BY WTInvariantStep, WTLegalInputs, SMT DEF WTNext, WTStableNext, WTQuiesce, wtVars, WTCreate, WTTemporarySync, WTRename, WTParentSync, WTInstall, WTRestore, WTRefuse, WTRecoverySync, WTReplay
<1>2. WTInvariant /\ (wtPhase = "write_top") /\ WTTemporarySync => (wtPhase = "rename")'
    BY SMT DEF WTTemporarySync
<1>3. WTInvariant /\ (wtPhase = "write_top") => ENABLED <<WTTemporarySync>>_wtVars
    BY ExpandENABLED, WTLegalInputs, SMT DEF WTTemporarySync, wtVars, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
<1> QED BY WTStableInvariant, <1>1, <1>2, <1>3, PTL DEF WTFairSpec

THEOREM WTRenameProgress == WTFairSpec => ((wtPhase = "rename") ~> (wtPhase = "sync_top"))
<1>1. WTInvariant /\ (wtPhase = "rename") /\ [WTStableNext]_wtVars =>
          WTInvariant' /\ ((wtPhase = "rename")' \/ (wtPhase = "sync_top")')
    BY WTInvariantStep, WTLegalInputs, SMT DEF WTNext, WTStableNext, WTQuiesce, wtVars, WTCreate, WTTemporarySync, WTRename, WTParentSync, WTInstall, WTRestore, WTRefuse, WTRecoverySync, WTReplay
<1>2. WTInvariant /\ (wtPhase = "rename") /\ WTRename => (wtPhase = "sync_top")'
    BY SMT DEF WTRename
<1>3. WTInvariant /\ (wtPhase = "rename") => ENABLED <<WTRename>>_wtVars
    BY ExpandENABLED, WTLegalInputs, SMT DEF WTRename, wtVars, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
<1> QED BY WTStableInvariant, <1>1, <1>2, <1>3, PTL DEF WTFairSpec

THEOREM WTParentSyncProgress == WTFairSpec => ((wtPhase = "sync_top") ~> (wtPhase = "install"))
<1>1. WTInvariant /\ (wtPhase = "sync_top") /\ [WTStableNext]_wtVars =>
          WTInvariant' /\ ((wtPhase = "sync_top")' \/ (wtPhase = "install")')
    BY WTInvariantStep, WTLegalInputs, SMT DEF WTNext, WTStableNext, WTQuiesce, wtVars, WTCreate, WTTemporarySync, WTRename, WTParentSync, WTInstall, WTRestore, WTRefuse, WTRecoverySync, WTReplay
<1>2. WTInvariant /\ (wtPhase = "sync_top") /\ WTParentSync => (wtPhase = "install")'
    BY SMT DEF WTParentSync
<1>3. WTInvariant /\ (wtPhase = "sync_top") => ENABLED <<WTParentSync>>_wtVars
    BY ExpandENABLED, WTLegalInputs, SMT DEF WTParentSync, wtVars, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
<1> QED BY WTStableInvariant, <1>1, <1>2, <1>3, PTL DEF WTFairSpec

THEOREM WTInstallProgress == WTFairSpec => ((wtPhase = "install") ~> (wtPhase = "ready"))
<1>1. WTInvariant /\ (wtPhase = "install") /\ [WTStableNext]_wtVars =>
          WTInvariant' /\ ((wtPhase = "install")' \/ (wtPhase = "ready")')
    BY WTInvariantStep, WTLegalInputs, SMT DEF WTNext, WTStableNext, WTQuiesce, wtVars, WTCreate, WTTemporarySync, WTRename, WTParentSync, WTInstall, WTRestore, WTRefuse, WTRecoverySync, WTReplay
<1>2. WTInvariant /\ (wtPhase = "install") /\ WTInstall => (wtPhase = "ready")'
    BY SMT DEF WTInstall
<1>3. WTInvariant /\ (wtPhase = "install") => ENABLED <<WTInstall>>_wtVars
    BY ExpandENABLED, WTLegalInputs, SMT DEF WTInstall, wtVars, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
<1> QED BY WTStableInvariant, <1>1, <1>2, <1>3, PTL DEF WTFairSpec

THEOREM WTRestoreProgress == WTFairSpec => ((wtPhase = "restore") ~> (wtPhase = "stabilize" \/ wtPhase = "rejected"))
<1>1. WTInvariant /\ (wtPhase = "restore") /\ [WTStableNext]_wtVars =>
          WTInvariant' /\ ((wtPhase = "restore")' \/ (wtPhase = "stabilize" \/ wtPhase = "rejected")')
    BY WTInvariantStep, WTLegalInputs, SMT DEF WTNext, WTStableNext, WTQuiesce, wtVars, WTCreate, WTTemporarySync, WTRename, WTParentSync, WTInstall, WTRestore, WTRefuse, WTRecoverySync, WTReplay
<1>2. WTInvariant /\ (wtPhase = "restore") /\ WTRestore => (wtPhase = "stabilize" \/ wtPhase = "rejected")'
    BY SMT DEF WTRestore
<1>3. WTInvariant /\ (wtPhase = "restore") => ENABLED <<WTRestore>>_wtVars
    BY ExpandENABLED, WTLegalInputs, SMT DEF WTRestore, wtVars, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
<1> QED BY WTStableInvariant, <1>1, <1>2, <1>3, PTL DEF WTFairSpec

THEOREM WTRecoverySyncProgress == WTFairSpec => ((wtPhase = "stabilize") ~> (wtPhase = "replay" \/ wtPhase = "rejected"))
<1>1. WTInvariant /\ (wtPhase = "stabilize") /\ [WTStableNext]_wtVars =>
          WTInvariant' /\ ((wtPhase = "stabilize")' \/ (wtPhase = "replay" \/ wtPhase = "rejected")')
    BY WTInvariantStep, WTLegalInputs, SMT DEF WTNext, WTStableNext, WTQuiesce, wtVars, WTCreate, WTTemporarySync, WTRename, WTParentSync, WTInstall, WTRestore, WTRefuse, WTRecoverySync, WTReplay
<1>2. WTInvariant /\ (wtPhase = "stabilize") /\ WTRecoverySync => (wtPhase = "replay" \/ wtPhase = "rejected")'
    BY SMT DEF WTRecoverySync
<1>3. WTInvariant /\ (wtPhase = "stabilize") => ENABLED <<WTRecoverySync>>_wtVars
    BY ExpandENABLED, WTLegalInputs, SMT DEF WTRecoverySync, wtVars, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments
<1> QED BY WTStableInvariant, <1>1, <1>2, <1>3, PTL DEF WTFairSpec

THEOREM WTReplayProgress == WTFairSpec => ((wtPhase = "replay") ~> (wtPhase = "ready" \/ wtPhase = "rejected"))
<1>1. WTInvariant /\ (wtPhase = "replay") /\ [WTStableNext]_wtVars =>
          WTInvariant' /\ ((wtPhase = "replay")' \/ (wtPhase = "ready" \/ wtPhase = "rejected")')
    BY WTInvariantStep, WTLegalInputs, SMT DEF WTNext, WTStableNext, WTQuiesce, wtVars, WTCreate, WTTemporarySync, WTRename, WTParentSync, WTInstall, WTRestore, WTRefuse, WTRecoverySync, WTReplay
<1>2. WTInvariant /\ (wtPhase = "replay") /\ WTReplay => (wtPhase = "ready" \/ wtPhase = "rejected")'
    BY SMT DEF WTReplay
<1>3. WTInvariant /\ (wtPhase = "replay") => ENABLED <<WTReplay>>_wtVars
    <2>1. WTInvariant /\ wtPhase = "replay" => wtRestored /\ WTAvailable(wtPlan)
        BY SMT DEF WTInvariant, WTLayouts, WTPlanAuthority, WTStage
    <2> QED BY <2>1, ExpandENABLED, Isa DEF WTReplay, wtVars
<1> QED BY WTStableInvariant, <1>1, <1>2, <1>3, PTL DEF WTFairSpec

THEOREM WTProgressProof == WTFairSpec => WTProgress
<1>1. WTInFlight <=> (wtPhase = "create" \/ wtPhase = "write_top" \/ wtPhase = "rename"
          \/ wtPhase = "sync_top" \/ wtPhase = "install" \/ wtPhase = "restore"
          \/ wtPhase = "stabilize" \/ wtPhase = "replay")
    BY SMT DEF WTInFlight
<1>2. (wtPhase = "ready" \/ wtPhase = "rejected") => WTSettled
    BY SMT DEF WTSettled
<1> QED BY <1>1, <1>2, WTCreateProgress, WTTemporarySyncProgress, WTRenameProgress, WTParentSyncProgress, WTInstallProgress, WTRestoreProgress, WTRecoverySyncProgress, WTReplayProgress, PTL DEF WTProgress

THEOREM WTCleanupInvariant == WTCleanupSpec => [](WTInvariant /\ wtPhase = "ready")
<1>1. DEFINE P == WTInvariant /\ wtPhase = "ready"
<1>2. WTCleanupNext => WTNext BY SMT DEF WTCleanupNext, WTNext
<1>3. P /\ [WTCleanupNext]_wtVars => P'
    BY <1>2, WTInvariantStep, SMT DEF P, WTCleanupNext, WTUnlink, WTCleanupSync, WTQuiesce, wtVars
<1> QED BY <1>3, PTL DEF WTCleanupSpec, WTCleanupFairness, P

THEOREM WTUnlinkProgress == WTCleanupSpec => (WTGarbage(WTReclaimTarget) ~> (WTReclaimTarget \notin wtFiles))
<1>1. WTInvariant /\ WTGarbage(WTReclaimTarget) /\ [WTCleanupNext]_wtVars =>
          WTInvariant' /\ (WTGarbage(WTReclaimTarget)' \/ (WTReclaimTarget \notin wtFiles)')
    BY WTInvariantStep, WTLegalInputs, SMT DEF WTNext, WTCleanupNext, WTGarbage, WTUnlink, WTCleanupSync, WTQuiesce, wtVars
<1>2. WTInvariant /\ WTGarbage(WTReclaimTarget) /\ WTUnlink(WTReclaimTarget) => (WTReclaimTarget \notin wtFiles)'
    BY SMT DEF WTUnlink
<1>3. WTInvariant /\ WTGarbage(WTReclaimTarget) => ENABLED <<WTUnlink(WTReclaimTarget)>>_wtVars
    <2>1. WTGarbage(WTReclaimTarget) => wtFiles \ {WTReclaimTarget} # wtFiles
        BY SMT DEF WTGarbage
    <2> QED BY <2>1, ExpandENABLED, SMT DEF WTGarbage, WTUnlink, wtVars
<1>4. WTCleanupSpec => WF_wtVars(WTUnlink(WTReclaimTarget))
    BY PTL DEF WTCleanupSpec, WTCleanupFairness
<1> QED BY WTCleanupInvariant, <1>1, <1>2, <1>3, <1>4, PTL DEF WTCleanupSpec, WTCleanupFairness

THEOREM WTDurableUnlinkProgress == WTCleanupSpec => ((WTReclaimTarget \notin wtFiles) ~> (WTReclaimTarget \notin wtDiskFiles))
<1>1. WTInvariant /\ (WTReclaimTarget \notin wtFiles /\ WTReclaimTarget \in wtDiskFiles) /\ [WTCleanupNext]_wtVars =>
          WTInvariant' /\ ((WTReclaimTarget \notin wtFiles /\ WTReclaimTarget \in wtDiskFiles)' \/ (WTReclaimTarget \notin wtDiskFiles)')
    BY WTInvariantStep, WTLegalInputs, SMT DEF WTNext, WTCleanupNext, WTUnlink, WTCleanupSync, WTQuiesce, wtVars
<1>2. WTInvariant /\ (WTReclaimTarget \notin wtFiles /\ WTReclaimTarget \in wtDiskFiles) /\ WTCleanupSync => (WTReclaimTarget \notin wtDiskFiles)'
    BY SMT DEF WTCleanupSync
<1>3. WTInvariant /\ wtPhase = "ready" /\ WTReclaimTarget \notin wtFiles /\ WTReclaimTarget \in wtDiskFiles =>
          ENABLED <<WTCleanupSync>>_wtVars
    BY ExpandENABLED, SMT DEF WTCleanupSync, wtVars
<1>4. (WTReclaimTarget \notin wtFiles) <=> ((WTReclaimTarget \notin wtFiles /\ WTReclaimTarget \in wtDiskFiles) \/ (WTReclaimTarget \notin wtFiles /\ WTReclaimTarget \notin wtDiskFiles))
    BY SMT
<1> QED BY WTCleanupInvariant, <1>1, <1>2, <1>3, <1>4, PTL DEF WTCleanupSpec, WTCleanupFairness

THEOREM WTReclaimProgress == WTCleanupSpec => (WTGarbage(WTReclaimTarget) ~> (WTReclaimTarget \notin wtDiskFiles))
BY WTUnlinkProgress, WTDurableUnlinkProgress, PTL

=============================================================================
