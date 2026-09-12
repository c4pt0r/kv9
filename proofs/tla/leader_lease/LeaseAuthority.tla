-------------------------- MODULE LeaseAuthority --------------------------
EXTENDS Naturals

\* One established Raft leader/term and a fixed configuration. A replacement
\* is a higher-term candidate; all votes below are actual votes for that
\* candidate. Raft log agreement and current-term commitment are composition
\* premises, not election/log algorithms reimplemented by this module.
CONSTANTS LANodes, LALeader, LAQuorums, LARounds, LAReads,
          LAWindow, LAGrantMin, LAGrantMax, LARecoveryWindow, LAMaxGeneration

ASSUME LALegalInputs ==
    /\ LANodes # {} /\ LALeader \in LANodes
    /\ LAQuorums \subseteq SUBSET LANodes /\ LAQuorums # {}
    /\ \A q \in LAQuorums : q # {}
    /\ \A q1, q2 \in LAQuorums : q1 \cap q2 # {}
    /\ LARounds \subseteq Nat \ {0} /\ LARounds # {}
    /\ LAReads \subseteq Nat \ {0} /\ LAReads # {}
    /\ LAWindow \in Nat \ {0}
    /\ LAGrantMin \in Nat /\ LAWindow <= LAGrantMin
    /\ LAGrantMax \in Nat /\ LAGrantMin <= LAGrantMax
    /\ LARecoveryWindow \in Nat /\ LAGrantMax <= LARecoveryWindow
    /\ LAMaxGeneration \in Nat \ {0}

LADead == LAMaxGeneration + 1
LAAdvanceGeneration(g) == IF g < LAMaxGeneration THEN g + 1 ELSE LADead
LAMax(a, b) == IF a >= b THEN a ELSE b
LAHasQuorum(nodes) == \E q \in LAQuorums : q \subseteq nodes

VARIABLES laNow, laMode, laGeneration, laLive, laSent, laStart, laEnd,
          laRoundGen, laPending, laActive, laPublished, laGranted, laUntil, laDelivered,
          laHold, laRecovery, laHigherVotes, laReplacement,
          laCommit, laKnown, laApplied, laReadPhase, laReadFloor,
          laReadTarget, laReadView, laReadRound, laReadGen

laVars == <<laNow, laMode, laGeneration, laLive, laSent, laStart, laEnd,
            laRoundGen, laPending, laActive, laPublished, laGranted, laUntil, laDelivered,
            laHold, laRecovery, laHigherVotes, laReplacement,
            laCommit, laKnown, laApplied, laReadPhase, laReadFloor,
            laReadTarget, laReadView, laReadRound, laReadGen>>

LAType ==
    /\ laNow \in Nat /\ laMode \in {"leader", "revoked", "fenced"}
    /\ laGeneration \in 0..LADead /\ laLive \subseteq LANodes
    /\ laSent \subseteq LARounds
    /\ laStart \in [LARounds -> Nat] /\ laEnd \in [LARounds -> Nat]
    /\ laRoundGen \in [LARounds -> 0..LADead]
    /\ laPending \in LARounds \cup {0} /\ laActive \in LARounds \cup {0}
    /\ laPublished \subseteq LARounds
    /\ laGranted \in [LARounds -> SUBSET LANodes]
    /\ laUntil \in [LARounds -> [LANodes -> Nat]]
    /\ laDelivered \in [LARounds -> SUBSET LANodes]
    /\ laHold \in [LANodes -> Nat] /\ laRecovery \in [LANodes -> Nat]
    /\ laHigherVotes \subseteq LANodes
    /\ laReplacement \in BOOLEAN
    /\ laCommit \in Nat /\ laKnown \in Nat /\ laApplied \in Nat
    /\ laReadPhase \in [LAReads -> {"idle", "wait", "view", "success", "fallback"}]
    /\ laReadFloor \in [LAReads -> Nat] /\ laReadTarget \in [LAReads -> Nat]
    /\ laReadView \in [LAReads -> Nat]
    /\ laReadRound \in [LAReads -> LARounds \cup {0}]
    /\ laReadGen \in [LAReads -> 0..LADead]

LALiveRound(r) ==
    /\ r \in laSent /\ laMode = "leader" /\ LALeader \in laLive
    /\ laGeneration # LADead /\ laRoundGen[r] = laGeneration
    /\ laNow < laEnd[r]

LAUsableRound(r) == r \in laPublished /\ LALiveRound(r)

LACanRead == laActive \in LARounds /\ LAUsableRound(laActive)
LATicketValid(r) ==
    /\ laReadRound[r] \in LARounds /\ LAUsableRound(laReadRound[r])
    /\ laReadGen[r] = laGeneration

\* Ghost grant deadlines retain promises across crashes even when volatile
\* hold state is erased. Restart must cover those obligations conservatively.
LARecordedGrants ==
    /\ \A r \in LARounds :
        /\ laDelivered[r] \subseteq laGranted[r]
        /\ laGranted[r] # {} => r \in laSent
    /\ \A r \in laSent :
        /\ laStart[r] <= laNow /\ laEnd[r] = laStart[r] + LAWindow
        /\ laRoundGen[r] <= laGeneration
        /\ \A q \in laGranted[r] :
            laEnd[r] <= laUntil[r][q] /\ laUntil[r][q] <= laNow + LAGrantMax

LAPromiseProtection ==
    \A q \in laLive : \A r \in LARounds : q \in laGranted[r] =>
        laUntil[r][q] <= LAMax(laHold[q], laRecovery[q])

LAVoteHistory ==
    /\ \A q \in laHigherVotes : \A r \in LARounds :
        q \in laGranted[r] => laUntil[r][q] <= laNow
    /\ laReplacement => LAHasQuorum(laHigherVotes)

LAAuthority ==
    /\ laGeneration = LADead => laMode = "fenced"
    /\ laPending # 0 => laPending \in laSent
    /\ laPublished \subseteq laSent
    /\ \A r \in laPublished : LAHasQuorum(laDelivered[r])
    /\ laActive # 0 => laActive \in laPublished

LANoReplacementUnderLease == \A r \in LARounds : LAUsableRound(r) => ~laReplacement

LAReadCoverage ==
    /\ laApplied <= laKnown /\ laKnown <= laCommit
    /\ ~laReplacement => laKnown = laCommit
    /\ \A r \in LAReads : laReadPhase[r] \in {"wait", "view", "success"} =>
        /\ laReadFloor[r] <= laReadTarget[r] /\ laReadTarget[r] <= laKnown
        /\ laReadRound[r] \in laSent
    /\ \A r \in LAReads : laReadPhase[r] \in {"view", "success"} =>
        laReadTarget[r] <= laReadView[r] /\ laReadView[r] <= laCommit

LAInvariant == LAType /\ LARecordedGrants /\ LAPromiseProtection /\ LAVoteHistory
               /\ LAAuthority /\ LANoReplacementUnderLease /\ LAReadCoverage

LAInit ==
    /\ laNow = 0 /\ laMode = "leader" /\ laGeneration = 0 /\ laLive = LANodes
    /\ laSent = {} /\ laStart = [r \in LARounds |-> 0]
    /\ laEnd = [r \in LARounds |-> 0] /\ laRoundGen = [r \in LARounds |-> 0]
    /\ laPending = 0 /\ laActive = 0 /\ laPublished = {}
    /\ laGranted = [r \in LARounds |-> {}]
    /\ laUntil = [r \in LARounds |-> [q \in LANodes |-> 0]]
    /\ laDelivered = [r \in LARounds |-> {}]
    /\ laHold = [q \in LANodes |-> 0] /\ laRecovery = [q \in LANodes |-> 0]
    /\ laHigherVotes = {}
    /\ laReplacement = FALSE
    \* The tracked term's initial no-op has already committed and applied.
    /\ laCommit = 1 /\ laKnown = 1 /\ laApplied = 1
    /\ laReadPhase = [r \in LAReads |-> "idle"]
    /\ laReadFloor = [r \in LAReads |-> 0] /\ laReadTarget = [r \in LAReads |-> 0]
    /\ laReadView = [r \in LAReads |-> 0] /\ laReadRound = [r \in LAReads |-> 0]
    /\ laReadGen = [r \in LAReads |-> 0]

LAAdvance == laNow' = laNow + 1 /\ UNCHANGED <<laMode, laGeneration, laLive, laSent, laStart, laEnd, laRoundGen, laPending, laActive,
        laGranted, laUntil, laDelivered, laHold, laRecovery, laHigherVotes, laReplacement,
        laCommit, laKnown, laApplied, laReadPhase, laReadFloor, laReadTarget, laReadView,
        laReadRound, laReadGen, laPublished>>

LAStartRound(r) ==
    /\ laMode = "leader" /\ LALeader \in laLive /\ laGeneration # LADead
    /\ r \notin laSent
    /\ laSent' = laSent \cup {r} /\ laPending' = r
    /\ laStart' = [laStart EXCEPT ![r] = laNow]
    /\ laEnd' = [laEnd EXCEPT ![r] = laNow + LAWindow]
    /\ laRoundGen' = [laRoundGen EXCEPT ![r] = laGeneration]
    /\ UNCHANGED <<laNow, laMode, laGeneration, laLive, laActive, laGranted, laUntil, laDelivered,
        laHold, laRecovery, laHigherVotes, laReplacement, laCommit, laKnown, laApplied,
        laReadPhase, laReadFloor, laReadTarget, laReadView, laReadRound, laReadGen, laPublished>>

LAGrant(r, q, duration) ==
    /\ r \in laSent /\ q \in laLive /\ q \notin laGranted[r]
    /\ q \notin laHigherVotes /\ laNow >= laRecovery[q]
    /\ laGranted' = [laGranted EXCEPT ![r] = @ \cup {q}]
    /\ laUntil' = [laUntil EXCEPT ![r][q] = laNow + duration]
    /\ laHold' = [laHold EXCEPT ![q] = LAMax(@, laNow + duration)]
    /\ UNCHANGED <<laNow, laMode, laGeneration, laLive, laSent, laStart, laEnd, laRoundGen, laPending,
        laActive, laDelivered, laRecovery, laHigherVotes, laReplacement, laCommit, laKnown,
        laApplied, laReadPhase, laReadFloor, laReadTarget, laReadView, laReadRound, laReadGen,
        laPublished>>

LADeliver(r, q) ==
    /\ LALeader \in laLive /\ q \in laGranted[r] /\ q \notin laDelivered[r]
    /\ laDelivered' = [laDelivered EXCEPT ![r] = @ \cup {q}]
    /\ UNCHANGED <<laNow, laMode, laGeneration, laLive, laSent, laStart, laEnd, laRoundGen, laPending,
        laActive, laGranted, laUntil, laHold, laRecovery, laHigherVotes, laReplacement, laCommit,
        laKnown, laApplied, laReadPhase, laReadFloor, laReadTarget, laReadView, laReadRound,
        laReadGen, laPublished>>

\* Publication is a separate successful-pump action, not an ACK receipt.
LAActivate(r) ==
    /\ laPending = r /\ LALiveRound(r) /\ LAHasQuorum(laDelivered[r])
    /\ laActive' = r /\ laPending' = 0
    /\ laPublished' = laPublished \cup {r}
    /\ UNCHANGED <<laNow, laMode, laGeneration, laLive, laSent, laStart, laEnd, laRoundGen, laGranted,
        laUntil, laDelivered, laHold, laRecovery, laHigherVotes, laReplacement, laCommit, laKnown,
        laApplied, laReadPhase, laReadFloor, laReadTarget, laReadView, laReadRound, laReadGen>>

LARevoke ==
    /\ laMode = "leader"
    /\ laGeneration' = LAAdvanceGeneration(laGeneration)
    /\ laMode' = IF laGeneration' = LADead THEN "fenced" ELSE "revoked"
    /\ laPending' = 0 /\ laActive' = 0
    /\ UNCHANGED <<laNow, laLive, laSent, laStart, laEnd, laRoundGen, laGranted, laUntil, laDelivered,
        laHold, laRecovery, laHigherVotes, laReplacement, laCommit, laKnown, laApplied,
        laReadPhase, laReadFloor, laReadTarget, laReadView, laReadRound, laReadGen, laPublished>>

\* Rearming a locally retained role does not itself renew any certificate.
LARearm ==
    /\ laMode = "revoked" /\ LALeader \in laLive /\ LALeader \notin laHigherVotes
    /\ laMode' = "leader"
    /\ UNCHANGED <<laNow, laGeneration, laLive, laSent, laStart, laEnd, laRoundGen, laPending, laActive,
        laGranted, laUntil, laDelivered, laHold, laRecovery, laHigherVotes, laReplacement,
        laCommit, laKnown, laApplied, laReadPhase, laReadFloor, laReadTarget, laReadView,
        laReadRound, laReadGen, laPublished>>

LACrash(q) ==
    /\ q \in laLive /\ laLive' = laLive \ {q}
    /\ laHold' = [laHold EXCEPT ![q] = 0]
    /\ laRecovery' = [laRecovery EXCEPT ![q] = 0]
    /\ laMode' = IF q = LALeader THEN "fenced" ELSE laMode
    /\ laGeneration' = IF q = LALeader THEN LADead ELSE laGeneration
    /\ laPending' = IF q = LALeader THEN 0 ELSE laPending
    /\ laActive' = IF q = LALeader THEN 0 ELSE laActive
    /\ UNCHANGED <<laNow, laSent, laStart, laEnd, laRoundGen, laGranted, laUntil, laDelivered,
        laHigherVotes, laReplacement, laCommit, laKnown, laApplied, laReadPhase, laReadFloor,
        laReadTarget, laReadView, laReadRound, laReadGen, laPublished>>

LARestart(q) ==
    /\ q \notin laLive /\ laLive' = laLive \cup {q}
    /\ laRecovery' = [laRecovery EXCEPT ![q] = laNow + LARecoveryWindow]
    /\ UNCHANGED <<laNow, laMode, laGeneration, laSent, laStart, laEnd, laRoundGen, laPending, laActive,
        laGranted, laUntil, laDelivered, laHold, laHigherVotes, laReplacement, laCommit, laKnown,
        laApplied, laReadPhase, laReadFloor, laReadTarget, laReadView, laReadRound, laReadGen,
        laPublished>>

LAVote(q) ==
    /\ q \in laLive /\ q \notin laHigherVotes
    /\ laNow >= laHold[q] /\ laNow >= laRecovery[q]
    /\ laHigherVotes' = laHigherVotes \cup {q}
    /\ laMode' = IF q = LALeader THEN "fenced" ELSE laMode
    /\ laGeneration' = IF q = LALeader THEN LADead ELSE laGeneration
    /\ laPending' = IF q = LALeader THEN 0 ELSE laPending
    /\ laActive' = IF q = LALeader THEN 0 ELSE laActive
    /\ UNCHANGED <<laNow, laLive, laSent, laStart, laEnd, laRoundGen, laGranted, laUntil, laDelivered,
        laHold, laRecovery, laReplacement, laCommit, laKnown, laApplied, laReadPhase, laReadFloor,
        laReadTarget, laReadView, laReadRound, laReadGen, laPublished>>

LAElect ==
    /\ ~laReplacement /\ LAHasQuorum(laHigherVotes) /\ laReplacement' = TRUE
    /\ UNCHANGED <<laNow, laMode, laGeneration, laLive, laSent, laStart, laEnd, laRoundGen, laPending,
        laActive, laPublished, laGranted, laUntil, laDelivered, laHold, laRecovery, laHigherVotes,
        laCommit, laKnown, laApplied, laReadPhase, laReadFloor, laReadTarget, laReadView,
        laReadRound, laReadGen>>

\* These two actions abstract Raft's agreed committed-prefix publication.
LAOldCommit ==
    /\ laMode = "leader" /\ LALeader \in laLive /\ ~laReplacement
    /\ laCommit' = laCommit + 1 /\ laKnown' = laCommit'
    /\ UNCHANGED <<laNow, laMode, laGeneration, laLive, laSent, laStart, laEnd, laRoundGen, laPending,
        laActive, laPublished, laGranted, laUntil, laDelivered, laHold, laRecovery, laHigherVotes,
        laReplacement, laApplied, laReadPhase, laReadFloor, laReadTarget, laReadView, laReadRound,
        laReadGen>>

LANewCommit ==
    /\ laReplacement /\ laCommit' = laCommit + 1
    /\ UNCHANGED <<laNow, laMode, laGeneration, laLive, laSent, laStart, laEnd, laRoundGen, laPending,
        laActive, laPublished, laGranted, laUntil, laDelivered, laHold, laRecovery, laHigherVotes,
        laReplacement, laKnown, laApplied, laReadPhase, laReadFloor, laReadTarget, laReadView,
        laReadRound, laReadGen>>

LAApply ==
    /\ LALeader \in laLive /\ laApplied < laKnown /\ laApplied' = laApplied + 1
    /\ UNCHANGED <<laNow, laMode, laGeneration, laLive, laSent, laStart, laEnd, laRoundGen, laPending,
        laActive, laPublished, laGranted, laUntil, laDelivered, laHold, laRecovery, laHigherVotes,
        laReplacement, laCommit, laKnown, laReadPhase, laReadFloor, laReadTarget, laReadView,
        laReadRound, laReadGen>>

LAReadBegin(r) ==
    /\ laReadPhase[r] = "idle" /\ LACanRead
    /\ laReadPhase' = [laReadPhase EXCEPT ![r] = "wait"]
    /\ laReadFloor' = [laReadFloor EXCEPT ![r] = laCommit]
    /\ laReadTarget' = [laReadTarget EXCEPT ![r] = laKnown]
    /\ laReadRound' = [laReadRound EXCEPT ![r] = laActive]
    /\ laReadGen' = [laReadGen EXCEPT ![r] = laGeneration]
    /\ UNCHANGED <<laNow, laMode, laGeneration, laLive, laSent, laStart, laEnd, laRoundGen, laPending,
        laActive, laPublished, laGranted, laUntil, laDelivered, laHold, laRecovery, laHigherVotes,
        laReplacement, laCommit, laKnown, laApplied, laReadView>>

LAReadView(r) ==
    /\ laReadPhase[r] = "wait" /\ laApplied >= laReadTarget[r]
    /\ laReadPhase' = [laReadPhase EXCEPT ![r] = "view"]
    /\ laReadView' = [laReadView EXCEPT ![r] = laApplied]
    /\ UNCHANGED <<laNow, laMode, laGeneration, laLive, laSent, laStart, laEnd, laRoundGen, laPending,
        laActive, laPublished, laGranted, laUntil, laDelivered, laHold, laRecovery, laHigherVotes,
        laReplacement, laCommit, laKnown, laApplied, laReadFloor, laReadTarget, laReadRound,
        laReadGen>>

LAReadFinish(r) ==
    /\ laReadPhase[r] = "view" /\ LATicketValid(r)
    /\ laReadPhase' = [laReadPhase EXCEPT ![r] = "success"]
    /\ UNCHANGED <<laNow, laMode, laGeneration, laLive, laSent, laStart, laEnd, laRoundGen, laPending,
        laActive, laPublished, laGranted, laUntil, laDelivered, laHold, laRecovery, laHigherVotes,
        laReplacement, laCommit, laKnown, laApplied, laReadFloor, laReadTarget, laReadView,
        laReadRound, laReadGen>>

LAReadFallback(r) ==
    /\ laReadPhase[r] \in {"idle", "wait", "view"}
    /\ laReadPhase' = [laReadPhase EXCEPT ![r] = "fallback"]
    /\ UNCHANGED <<laNow, laMode, laGeneration, laLive, laSent, laStart, laEnd, laRoundGen, laPending,
        laActive, laPublished, laGranted, laUntil, laDelivered, laHold, laRecovery, laHigherVotes,
        laReplacement, laCommit, laKnown, laApplied, laReadFloor, laReadTarget, laReadView,
        laReadRound, laReadGen>>

LANext ==
    \/ LAAdvance \/ LARevoke \/ LARearm \/ LAElect \/ LAOldCommit \/ LANewCommit \/ LAApply
    \/ \E r \in LARounds : LAStartRound(r) \/ LAActivate(r)
    \/ \E r \in LARounds, q \in LANodes : LADeliver(r, q)
    \/ \E r \in LARounds, q \in LANodes, d \in LAGrantMin..LAGrantMax : LAGrant(r, q, d)
    \/ \E q \in LANodes : LACrash(q) \/ LARestart(q) \/ LAVote(q)
    \/ \E r \in LAReads : LAReadBegin(r) \/ LAReadView(r) \/ LAReadFinish(r) \/ LAReadFallback(r)

LASpec == LAInit /\ [][LANext]_laVars
=============================================================================
