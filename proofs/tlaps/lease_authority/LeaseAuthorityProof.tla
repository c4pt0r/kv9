---------------------- MODULE LeaseAuthorityProof ----------------------
EXTENDS LeaseAuthority, TLAPS

THEOREM LAQuorumIntersection ==
    \A a, b \in SUBSET LANodes : LAHasQuorum(a) /\ LAHasQuorum(b) => a \cap b # {}
<1>1. SUFFICES ASSUME NEW a \in SUBSET LANodes, NEW b \in SUBSET LANodes,
                     LAHasQuorum(a), LAHasQuorum(b)
              PROVE a \cap b # {}
    OBVIOUS
<1>2. PICK qa \in LAQuorums : qa \subseteq a
    BY <1>1 DEF LAHasQuorum
<1>3. PICK qb \in LAQuorums : qb \subseteq b
    BY <1>1 DEF LAHasQuorum
<1>4. PICK n \in qa \cap qb : TRUE
    BY <1>2, <1>3, LALegalInputs, SMT
<1> QED BY <1>2, <1>3, <1>4, SMT

THEOREM LAExpirySeparation ==
    \A t, d, x, y \in Nat : t < d /\ d <= x /\ x <= y /\ y <= t => FALSE
BY SMT

THEOREM LARecoveryCover ==
    \A expiry, t, span, quarantine, held \in Nat :
        expiry <= t + span /\ span <= quarantine => expiry <= LAMax(held, t + quarantine)
BY SMT DEF LAMax

THEOREM LAProtectedQuorum ==
    \A ack, votes \in SUBSET LANodes, t, end \in Nat, until \in [LANodes -> Nat] :
        LAHasQuorum(ack) /\ t < end
        /\ (\A q \in ack : end <= until[q])
        /\ (\A q \in ack \cap votes : until[q] <= t)
        => ~LAHasQuorum(votes)
<1>1. SUFFICES ASSUME NEW ack \in SUBSET LANodes, NEW votes \in SUBSET LANodes,
                     NEW t \in Nat, NEW end \in Nat, NEW until \in [LANodes -> Nat],
                     LAHasQuorum(ack), t < end,
                     \A q \in ack : end <= until[q],
                     \A q \in ack \cap votes : until[q] <= t,
                     LAHasQuorum(votes)
              PROVE FALSE
    OBVIOUS
<1>2. ack \cap votes # {} BY <1>1, LAQuorumIntersection, SMT
<1>3. PICK q \in ack \cap votes : TRUE BY <1>2, SMT
<1> QED BY <1>1, <1>3, SMT

THEOREM LAInvariantInit == LAInit => LAInvariant
BY LALegalInputs, SMT DEF LAInit, LAInvariant, LAType, LARecordedGrants, LAPromiseProtection, LAVoteHistory, LAAuthority, LANoReplacementUnderLease, LAReadCoverage, LAUsableRound, LALiveRound, LACanRead, LATicketValid, LAHasQuorum, LADead, LAAdvanceGeneration, LAMax

THEOREM LAAdvanceStep == ASSUME LAInvariant, LAAdvance PROVE LAInvariant'
BY LALegalInputs, SMT DEF LAAdvance, LAInvariant, LAType, LARecordedGrants, LAPromiseProtection, LAVoteHistory, LAAuthority, LANoReplacementUnderLease, LAReadCoverage, LAUsableRound, LALiveRound, LACanRead, LATicketValid, LAHasQuorum, LADead, LAAdvanceGeneration, LAMax

THEOREM LAStartRoundStep == ASSUME LAInvariant, NEW r \in LARounds, LAStartRound(r) PROVE LAInvariant'
BY LALegalInputs, SMT DEF LAStartRound, LAInvariant, LAType, LARecordedGrants, LAPromiseProtection, LAVoteHistory, LAAuthority, LANoReplacementUnderLease, LAReadCoverage, LAUsableRound, LALiveRound, LACanRead, LATicketValid, LAHasQuorum, LADead, LAAdvanceGeneration, LAMax

THEOREM LAGrantStep == ASSUME LAInvariant, NEW r \in LARounds, NEW q \in LANodes, NEW d \in LAGrantMin..LAGrantMax, LAGrant(r, q, d) PROVE LAInvariant'
BY LALegalInputs, SMT DEF LAGrant, LAInvariant, LAType, LARecordedGrants, LAPromiseProtection, LAVoteHistory, LAAuthority, LANoReplacementUnderLease, LAReadCoverage, LAUsableRound, LALiveRound, LACanRead, LATicketValid, LAHasQuorum, LADead, LAAdvanceGeneration, LAMax

THEOREM LADeliverStep == ASSUME LAInvariant, NEW r \in LARounds, NEW q \in LANodes, LADeliver(r, q) PROVE LAInvariant'
BY LALegalInputs, SMT DEF LADeliver, LAInvariant, LAType, LARecordedGrants, LAPromiseProtection, LAVoteHistory, LAAuthority, LANoReplacementUnderLease, LAReadCoverage, LAUsableRound, LALiveRound, LACanRead, LATicketValid, LAHasQuorum, LADead, LAAdvanceGeneration, LAMax

THEOREM LAActivateStep == ASSUME LAInvariant, NEW r \in LARounds, LAActivate(r) PROVE LAInvariant'
<1>1. LAType'
    BY LALegalInputs, LARecoveryCover, SMT DEF LAActivate, LAInvariant, LAType, LARecordedGrants, LAPromiseProtection, LAVoteHistory, LAAuthority, LANoReplacementUnderLease, LAReadCoverage, LAUsableRound, LALiveRound, LACanRead, LATicketValid, LAHasQuorum, LADead, LAAdvanceGeneration, LAMax
<1>2. LARecordedGrants'
    BY LALegalInputs, LARecoveryCover, SMT DEF LAActivate, LAInvariant, LAType, LARecordedGrants, LAPromiseProtection, LAVoteHistory, LAAuthority, LANoReplacementUnderLease, LAReadCoverage, LAUsableRound, LALiveRound, LACanRead, LATicketValid, LAHasQuorum, LADead, LAAdvanceGeneration, LAMax
<1>3. LAPromiseProtection'
    BY LALegalInputs, LARecoveryCover, SMT DEF LAActivate, LAInvariant, LAType, LARecordedGrants, LAPromiseProtection, LAVoteHistory, LAAuthority, LANoReplacementUnderLease, LAReadCoverage, LAUsableRound, LALiveRound, LACanRead, LATicketValid, LAHasQuorum, LADead, LAAdvanceGeneration, LAMax
<1>4. LAVoteHistory'
    BY LALegalInputs, LARecoveryCover, SMT DEF LAActivate, LAInvariant, LAType, LARecordedGrants, LAPromiseProtection, LAVoteHistory, LAAuthority, LANoReplacementUnderLease, LAReadCoverage, LAUsableRound, LALiveRound, LACanRead, LATicketValid, LAHasQuorum, LADead, LAAdvanceGeneration, LAMax
<1>5. LAAuthority'
    BY LALegalInputs, LARecoveryCover, SMT DEF LAActivate, LAInvariant, LAType, LARecordedGrants, LAPromiseProtection, LAVoteHistory, LAAuthority, LANoReplacementUnderLease, LAReadCoverage, LAUsableRound, LALiveRound, LACanRead, LATicketValid, LAHasQuorum, LADead, LAAdvanceGeneration, LAMax
<1>6. LAReadCoverage'
    BY LALegalInputs, LARecoveryCover, SMT DEF LAActivate, LAInvariant, LAType, LARecordedGrants, LAPromiseProtection, LAVoteHistory, LAAuthority, LANoReplacementUnderLease, LAReadCoverage, LAUsableRound, LALiveRound, LACanRead, LATicketValid, LAHasQuorum, LADead, LAAdvanceGeneration, LAMax
<1>7. LANoReplacementUnderLease'
    <2>1. SUFFICES ASSUME NEW rr \in LARounds, LAUsableRound(rr)'
                  PROVE ~laReplacement'
        BY SMT DEF LANoReplacementUnderLease
    <2>2. rr \in laSent' /\ rr \in laPublished' /\ laNow' < laEnd'[rr]
          /\ LAHasQuorum(laDelivered'[rr])
        BY <1>1, <1>5, <2>1, SMT DEF LAType, LAAuthority, LAUsableRound, LALiveRound
    <2>3. \A q \in laDelivered'[rr] : laEnd'[rr] <= laUntil'[rr][q]
        BY <1>1, <1>2, <2>2, SMT DEF LAType, LARecordedGrants
    <2>4. \A q \in laDelivered'[rr] \cap laHigherVotes' : laUntil'[rr][q] <= laNow'
        BY <1>1, <1>2, <1>4, <2>2, SMT DEF LAType, LARecordedGrants, LAVoteHistory
    <2>5. ~LAHasQuorum(laHigherVotes')
        BY <1>1, <2>2, <2>3, <2>4, LAProtectedQuorum, SMT DEF LAType
    <2> QED BY <1>4, <2>5, SMT DEF LAVoteHistory
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7 DEF LAInvariant

THEOREM LARevokeStep == ASSUME LAInvariant, LARevoke PROVE LAInvariant'
BY LALegalInputs, SMT DEF LARevoke, LAInvariant, LAType, LARecordedGrants, LAPromiseProtection, LAVoteHistory, LAAuthority, LANoReplacementUnderLease, LAReadCoverage, LAUsableRound, LALiveRound, LACanRead, LATicketValid, LAHasQuorum, LADead, LAAdvanceGeneration, LAMax

THEOREM LARearmStep == ASSUME LAInvariant, LARearm PROVE LAInvariant'
<1>1. LAType'
    BY LALegalInputs, LARecoveryCover, SMT DEF LARearm, LAInvariant, LAType, LARecordedGrants, LAPromiseProtection, LAVoteHistory, LAAuthority, LANoReplacementUnderLease, LAReadCoverage, LAUsableRound, LALiveRound, LACanRead, LATicketValid, LAHasQuorum, LADead, LAAdvanceGeneration, LAMax
<1>2. LARecordedGrants'
    BY LALegalInputs, LARecoveryCover, SMT DEF LARearm, LAInvariant, LAType, LARecordedGrants, LAPromiseProtection, LAVoteHistory, LAAuthority, LANoReplacementUnderLease, LAReadCoverage, LAUsableRound, LALiveRound, LACanRead, LATicketValid, LAHasQuorum, LADead, LAAdvanceGeneration, LAMax
<1>3. LAPromiseProtection'
    BY LALegalInputs, LARecoveryCover, SMT DEF LARearm, LAInvariant, LAType, LARecordedGrants, LAPromiseProtection, LAVoteHistory, LAAuthority, LANoReplacementUnderLease, LAReadCoverage, LAUsableRound, LALiveRound, LACanRead, LATicketValid, LAHasQuorum, LADead, LAAdvanceGeneration, LAMax
<1>4. LAVoteHistory'
    BY LALegalInputs, LARecoveryCover, SMT DEF LARearm, LAInvariant, LAType, LARecordedGrants, LAPromiseProtection, LAVoteHistory, LAAuthority, LANoReplacementUnderLease, LAReadCoverage, LAUsableRound, LALiveRound, LACanRead, LATicketValid, LAHasQuorum, LADead, LAAdvanceGeneration, LAMax
<1>5. LAAuthority'
    BY LALegalInputs, LARecoveryCover, SMT DEF LARearm, LAInvariant, LAType, LARecordedGrants, LAPromiseProtection, LAVoteHistory, LAAuthority, LANoReplacementUnderLease, LAReadCoverage, LAUsableRound, LALiveRound, LACanRead, LATicketValid, LAHasQuorum, LADead, LAAdvanceGeneration, LAMax
<1>6. LAReadCoverage'
    BY LALegalInputs, LARecoveryCover, SMT DEF LARearm, LAInvariant, LAType, LARecordedGrants, LAPromiseProtection, LAVoteHistory, LAAuthority, LANoReplacementUnderLease, LAReadCoverage, LAUsableRound, LALiveRound, LACanRead, LATicketValid, LAHasQuorum, LADead, LAAdvanceGeneration, LAMax
<1>7. LANoReplacementUnderLease'
    <2>1. SUFFICES ASSUME NEW rr \in LARounds, LAUsableRound(rr)'
                  PROVE ~laReplacement'
        BY SMT DEF LANoReplacementUnderLease
    <2>2. rr \in laSent' /\ rr \in laPublished' /\ laNow' < laEnd'[rr]
          /\ LAHasQuorum(laDelivered'[rr])
        BY <1>1, <1>5, <2>1, SMT DEF LAType, LAAuthority, LAUsableRound, LALiveRound
    <2>3. \A q \in laDelivered'[rr] : laEnd'[rr] <= laUntil'[rr][q]
        BY <1>1, <1>2, <2>2, SMT DEF LAType, LARecordedGrants
    <2>4. \A q \in laDelivered'[rr] \cap laHigherVotes' : laUntil'[rr][q] <= laNow'
        BY <1>1, <1>2, <1>4, <2>2, SMT DEF LAType, LARecordedGrants, LAVoteHistory
    <2>5. ~LAHasQuorum(laHigherVotes')
        BY <1>1, <2>2, <2>3, <2>4, LAProtectedQuorum, SMT DEF LAType
    <2> QED BY <1>4, <2>5, SMT DEF LAVoteHistory
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7 DEF LAInvariant

THEOREM LACrashStep == ASSUME LAInvariant, NEW q \in LANodes, LACrash(q) PROVE LAInvariant'
BY LALegalInputs, SMT DEF LACrash, LAInvariant, LAType, LARecordedGrants, LAPromiseProtection, LAVoteHistory, LAAuthority, LANoReplacementUnderLease, LAReadCoverage, LAUsableRound, LALiveRound, LACanRead, LATicketValid, LAHasQuorum, LADead, LAAdvanceGeneration, LAMax

THEOREM LARestartStep == ASSUME LAInvariant, NEW q \in LANodes, LARestart(q) PROVE LAInvariant'
<1>1. LAType'
    BY LALegalInputs, LARecoveryCover, SMT DEF LARestart, LAInvariant, LAType, LARecordedGrants, LAPromiseProtection, LAVoteHistory, LAAuthority, LANoReplacementUnderLease, LAReadCoverage, LAUsableRound, LALiveRound, LACanRead, LATicketValid, LAHasQuorum, LADead, LAAdvanceGeneration, LAMax
<1>2. LARecordedGrants'
    BY LALegalInputs, LARecoveryCover, SMT DEF LARestart, LAInvariant, LAType, LARecordedGrants, LAPromiseProtection, LAVoteHistory, LAAuthority, LANoReplacementUnderLease, LAReadCoverage, LAUsableRound, LALiveRound, LACanRead, LATicketValid, LAHasQuorum, LADead, LAAdvanceGeneration, LAMax
<1>3. LAPromiseProtection'
    BY LALegalInputs, LARecoveryCover, SMT DEF LARestart, LAInvariant, LAType, LARecordedGrants, LAPromiseProtection, LAVoteHistory, LAAuthority, LANoReplacementUnderLease, LAReadCoverage, LAUsableRound, LALiveRound, LACanRead, LATicketValid, LAHasQuorum, LADead, LAAdvanceGeneration, LAMax
<1>4. LAVoteHistory'
    BY LALegalInputs, LARecoveryCover, SMT DEF LARestart, LAInvariant, LAType, LARecordedGrants, LAPromiseProtection, LAVoteHistory, LAAuthority, LANoReplacementUnderLease, LAReadCoverage, LAUsableRound, LALiveRound, LACanRead, LATicketValid, LAHasQuorum, LADead, LAAdvanceGeneration, LAMax
<1>5. LAAuthority'
    BY LALegalInputs, LARecoveryCover, SMT DEF LARestart, LAInvariant, LAType, LARecordedGrants, LAPromiseProtection, LAVoteHistory, LAAuthority, LANoReplacementUnderLease, LAReadCoverage, LAUsableRound, LALiveRound, LACanRead, LATicketValid, LAHasQuorum, LADead, LAAdvanceGeneration, LAMax
<1>6. LAReadCoverage'
    BY LALegalInputs, LARecoveryCover, SMT DEF LARestart, LAInvariant, LAType, LARecordedGrants, LAPromiseProtection, LAVoteHistory, LAAuthority, LANoReplacementUnderLease, LAReadCoverage, LAUsableRound, LALiveRound, LACanRead, LATicketValid, LAHasQuorum, LADead, LAAdvanceGeneration, LAMax
<1>7. LANoReplacementUnderLease'
    <2>1. SUFFICES ASSUME NEW rr \in LARounds, LAUsableRound(rr)'
                  PROVE ~laReplacement'
        BY SMT DEF LANoReplacementUnderLease
    <2>2. rr \in laSent' /\ rr \in laPublished' /\ laNow' < laEnd'[rr]
          /\ LAHasQuorum(laDelivered'[rr])
        BY <1>1, <1>5, <2>1, SMT DEF LAType, LAAuthority, LAUsableRound, LALiveRound
    <2>3. \A voter \in laDelivered'[rr] : laEnd'[rr] <= laUntil'[rr][voter]
        BY <1>1, <1>2, <2>2, SMT DEF LAType, LARecordedGrants
    <2>4. \A voter \in laDelivered'[rr] \cap laHigherVotes' : laUntil'[rr][voter] <= laNow'
        BY <1>1, <1>2, <1>4, <2>2, SMT DEF LAType, LARecordedGrants, LAVoteHistory
    <2>5. ~LAHasQuorum(laHigherVotes')
        BY <1>1, <2>2, <2>3, <2>4, LAProtectedQuorum, SMT DEF LAType
    <2> QED BY <1>4, <2>5, SMT DEF LAVoteHistory
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7 DEF LAInvariant

THEOREM LAVoteStep == ASSUME LAInvariant, NEW q \in LANodes, LAVote(q) PROVE LAInvariant'
BY LALegalInputs, SMT DEF LAVote, LAInvariant, LAType, LARecordedGrants, LAPromiseProtection, LAVoteHistory, LAAuthority, LANoReplacementUnderLease, LAReadCoverage, LAUsableRound, LALiveRound, LACanRead, LATicketValid, LAHasQuorum, LADead, LAAdvanceGeneration, LAMax

THEOREM LAElectStep == ASSUME LAInvariant, LAElect PROVE LAInvariant'
<1>1. LAType'
    BY LALegalInputs, LARecoveryCover, SMT DEF LAElect, LAInvariant, LAType, LARecordedGrants, LAPromiseProtection, LAVoteHistory, LAAuthority, LANoReplacementUnderLease, LAReadCoverage, LAUsableRound, LALiveRound, LACanRead, LATicketValid, LAHasQuorum, LADead, LAAdvanceGeneration, LAMax
<1>2. LARecordedGrants'
    BY LALegalInputs, LARecoveryCover, SMT DEF LAElect, LAInvariant, LAType, LARecordedGrants, LAPromiseProtection, LAVoteHistory, LAAuthority, LANoReplacementUnderLease, LAReadCoverage, LAUsableRound, LALiveRound, LACanRead, LATicketValid, LAHasQuorum, LADead, LAAdvanceGeneration, LAMax
<1>3. LAPromiseProtection'
    BY LALegalInputs, LARecoveryCover, SMT DEF LAElect, LAInvariant, LAType, LARecordedGrants, LAPromiseProtection, LAVoteHistory, LAAuthority, LANoReplacementUnderLease, LAReadCoverage, LAUsableRound, LALiveRound, LACanRead, LATicketValid, LAHasQuorum, LADead, LAAdvanceGeneration, LAMax
<1>4. LAVoteHistory'
    BY LALegalInputs, LARecoveryCover, SMT DEF LAElect, LAInvariant, LAType, LARecordedGrants, LAPromiseProtection, LAVoteHistory, LAAuthority, LANoReplacementUnderLease, LAReadCoverage, LAUsableRound, LALiveRound, LACanRead, LATicketValid, LAHasQuorum, LADead, LAAdvanceGeneration, LAMax
<1>5. LAAuthority'
    BY LALegalInputs, LARecoveryCover, SMT DEF LAElect, LAInvariant, LAType, LARecordedGrants, LAPromiseProtection, LAVoteHistory, LAAuthority, LANoReplacementUnderLease, LAReadCoverage, LAUsableRound, LALiveRound, LACanRead, LATicketValid, LAHasQuorum, LADead, LAAdvanceGeneration, LAMax
<1>6. LAReadCoverage'
    BY LALegalInputs, LARecoveryCover, SMT DEF LAElect, LAInvariant, LAType, LARecordedGrants, LAPromiseProtection, LAVoteHistory, LAAuthority, LANoReplacementUnderLease, LAReadCoverage, LAUsableRound, LALiveRound, LACanRead, LATicketValid, LAHasQuorum, LADead, LAAdvanceGeneration, LAMax
<1>7. LANoReplacementUnderLease'
    <2>1. SUFFICES ASSUME NEW rr \in LARounds, LAUsableRound(rr)'
                  PROVE ~laReplacement'
        BY SMT DEF LANoReplacementUnderLease
    <2>2. rr \in laSent' /\ rr \in laPublished' /\ laNow' < laEnd'[rr]
          /\ LAHasQuorum(laDelivered'[rr])
        BY <1>1, <1>5, <2>1, SMT DEF LAType, LAAuthority, LAUsableRound, LALiveRound
    <2>3. \A q \in laDelivered'[rr] : laEnd'[rr] <= laUntil'[rr][q]
        BY <1>1, <1>2, <2>2, SMT DEF LAType, LARecordedGrants
    <2>4. \A q \in laDelivered'[rr] \cap laHigherVotes' : laUntil'[rr][q] <= laNow'
        BY <1>1, <1>2, <1>4, <2>2, SMT DEF LAType, LARecordedGrants, LAVoteHistory
    <2>5. ~LAHasQuorum(laHigherVotes')
        BY <1>1, <2>2, <2>3, <2>4, LAProtectedQuorum, SMT DEF LAType
    <2> QED BY <1>4, <2>5, SMT DEF LAVoteHistory
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7 DEF LAInvariant

THEOREM LAOldCommitStep == ASSUME LAInvariant, LAOldCommit PROVE LAInvariant'
BY LALegalInputs, SMT DEF LAOldCommit, LAInvariant, LAType, LARecordedGrants, LAPromiseProtection, LAVoteHistory, LAAuthority, LANoReplacementUnderLease, LAReadCoverage, LAUsableRound, LALiveRound, LACanRead, LATicketValid, LAHasQuorum, LADead, LAAdvanceGeneration, LAMax

THEOREM LANewCommitStep == ASSUME LAInvariant, LANewCommit PROVE LAInvariant'
BY LALegalInputs, SMT DEF LANewCommit, LAInvariant, LAType, LARecordedGrants, LAPromiseProtection, LAVoteHistory, LAAuthority, LANoReplacementUnderLease, LAReadCoverage, LAUsableRound, LALiveRound, LACanRead, LATicketValid, LAHasQuorum, LADead, LAAdvanceGeneration, LAMax

THEOREM LAApplyStep == ASSUME LAInvariant, LAApply PROVE LAInvariant'
BY LALegalInputs, SMT DEF LAApply, LAInvariant, LAType, LARecordedGrants, LAPromiseProtection, LAVoteHistory, LAAuthority, LANoReplacementUnderLease, LAReadCoverage, LAUsableRound, LALiveRound, LACanRead, LATicketValid, LAHasQuorum, LADead, LAAdvanceGeneration, LAMax

THEOREM LAReadBeginStep == ASSUME LAInvariant, NEW r \in LAReads, LAReadBegin(r) PROVE LAInvariant'
BY LALegalInputs, SMT DEF LAReadBegin, LAInvariant, LAType, LARecordedGrants, LAPromiseProtection, LAVoteHistory, LAAuthority, LANoReplacementUnderLease, LAReadCoverage, LAUsableRound, LALiveRound, LACanRead, LATicketValid, LAHasQuorum, LADead, LAAdvanceGeneration, LAMax

THEOREM LAReadViewStep == ASSUME LAInvariant, NEW r \in LAReads, LAReadView(r) PROVE LAInvariant'
BY LALegalInputs, SMT DEF LAReadView, LAInvariant, LAType, LARecordedGrants, LAPromiseProtection, LAVoteHistory, LAAuthority, LANoReplacementUnderLease, LAReadCoverage, LAUsableRound, LALiveRound, LACanRead, LATicketValid, LAHasQuorum, LADead, LAAdvanceGeneration, LAMax

THEOREM LAReadFinishStep == ASSUME LAInvariant, NEW r \in LAReads, LAReadFinish(r) PROVE LAInvariant'
BY LALegalInputs, SMT DEF LAReadFinish, LAInvariant, LAType, LARecordedGrants, LAPromiseProtection, LAVoteHistory, LAAuthority, LANoReplacementUnderLease, LAReadCoverage, LAUsableRound, LALiveRound, LACanRead, LATicketValid, LAHasQuorum, LADead, LAAdvanceGeneration, LAMax

THEOREM LAReadFallbackStep == ASSUME LAInvariant, NEW r \in LAReads, LAReadFallback(r) PROVE LAInvariant'
BY LALegalInputs, SMT DEF LAReadFallback, LAInvariant, LAType, LARecordedGrants, LAPromiseProtection, LAVoteHistory, LAAuthority, LANoReplacementUnderLease, LAReadCoverage, LAUsableRound, LALiveRound, LACanRead, LATicketValid, LAHasQuorum, LADead, LAAdvanceGeneration, LAMax

THEOREM LAInvariantStep == ASSUME LAInvariant, [LANext]_laVars PROVE LAInvariant'
BY LAAdvanceStep, LAStartRoundStep, LAGrantStep, LADeliverStep, LAActivateStep, LARevokeStep, LARearmStep, LACrashStep, LARestartStep, LAVoteStep, LAElectStep, LAOldCommitStep, LANewCommitStep, LAApplyStep, LAReadBeginStep, LAReadViewStep, LAReadFinishStep, LAReadFallbackStep, SMT DEF LANext, laVars, LAInvariant, LAType, LARecordedGrants, LAPromiseProtection, LAVoteHistory, LAAuthority, LANoReplacementUnderLease, LAReadCoverage, LAUsableRound, LALiveRound, LACanRead, LATicketValid, LAHasQuorum, LADead, LAAdvanceGeneration, LAMax

THEOREM LAInvariantAlways == LASpec => []LAInvariant
BY LAInvariantInit, LAInvariantStep, PTL DEF LASpec

\* Read steps use a previously published certificate. They neither send a
\* renewal nor collect a grant, ACK, or vote, and do not advance application.
THEOREM LAReadStepsAreLocal ==
    \A r \in LAReads :
        (LAReadBegin(r) \/ LAReadView(r) \/ LAReadFinish(r) \/ LAReadFallback(r)) =>
        UNCHANGED <<laNow, laMode, laGeneration, laLive, laSent, laStart, laEnd,
            laRoundGen, laPending, laActive, laPublished, laGranted, laUntil,
            laDelivered, laHold, laRecovery, laHigherVotes, laReplacement,
            laCommit, laKnown, laApplied>>
BY SMT DEF LAReadBegin, LAReadView, LAReadFinish, LAReadFallback

THEOREM LAReadFinishAuthorized ==
    ASSUME LAInvariant, NEW r \in LAReads, LAReadFinish(r)
    PROVE /\ ~laReplacement
          /\ laReadFloor[r] <= laReadTarget[r]
          /\ laReadTarget[r] <= laReadView[r]
          /\ laNow < laEnd[laReadRound[r]]
          /\ laReadGen[r] = laGeneration
BY SMT DEF LAInvariant, LAReadFinish, LAReadCoverage, LATicketValid,
    LANoReplacementUnderLease, LAUsableRound, LALiveRound
=============================================================================
