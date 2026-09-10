------------------------- MODULE WalTopology -------------------------
EXTENDS Naturals
CONSTANTS WTMax, WTRecords, WTUnpositioned, WTAnchors, WTCover, WTTrusted, WTReclaimTarget
ASSUME WTLegalInputs ==
    /\ WTMax \in Nat \ {0} /\ WTReclaimTarget \in 1..WTMax
    /\ WTRecords # {} /\ WTRecords \subseteq Nat \ {0}
    /\ WTUnpositioned \subseteq WTRecords
    /\ WTAnchors \subseteq Nat /\ 0 \in WTAnchors
    /\ WTCover \in [WTAnchors -> SUBSET (WTRecords \ WTUnpositioned)]
    /\ WTCover[0] = {} /\ WTTrusted \subseteq WTAnchors /\ 0 \in WTTrusted

\* One exclusive owner and one stream identity. Segment tokens carry the exact
\* selected identity/sequence/predecessor. Topologies are inseparable checked
\* anchor/retained-suffix/active records; byte decoding is a refinement premise.
\* wtData contains the primitive's complete durable frames. Failed append may
\* contribute an unacknowledged frame; incomplete tails remain inside that proof.
\* WTCover is the complete logical state certified by the external committed
\* checkpoint authority, not an upload receipt or a local position comparison.
VARIABLES wtPhase, wtVisible, wtDurable, wtLocal, wtCandidate, wtPlan, wtFiles, wtDiskFiles, wtData, wtAck, wtView, wtWriter, wtRestored
wtVars == <<wtPhase, wtVisible, wtDurable, wtLocal, wtCandidate, wtPlan, wtFiles, wtDiskFiles, wtData, wtAck, wtView, wtWriter, wtRestored>>

WTSegments == 1..WTMax
WTTopologies == [anchor : WTTrusted, first : WTSegments, active : WTSegments]
WTInitialTopology == [anchor |-> 0, first |-> 1, active |-> 1]
WTPhases == {"ready", "create", "write_top", "rename", "sync_top", "install",
             "fenced", "down", "restore", "stabilize", "replay", "rejected"}
WTSelected(t) == t.first..t.active
WTClosed(t) == t.first..(t.active - 1)
WTContents(t) == WTCover[t.anchor] \cup
    {r \in WTRecords : \E s \in WTSelected(t) : r \in wtData[s]}
WTTail(t) == {r \in WTRecords : r \notin WTCover[t.anchor] /\
    \E s \in WTSelected(t) : r \in wtData[s]}
WTValid(t) == /\ t \in WTTopologies /\ t.first <= t.active
              /\ \A s \in WTSegments : s < t.first => wtData[s] \subseteq WTCover[t.anchor]
WTAvailable(t) == WTSelected(t) \subseteq wtFiles /\ WTSelected(t) \subseteq wtDiskFiles
WTCheckpointChoice(c, f) ==
    /\ c \in WTTrusted /\ c # 0 /\ f \in wtLocal.first..wtLocal.active
    /\ WTCover[wtLocal.anchor] \subseteq WTCover[c]
    /\ WTCover[c] \subseteq WTContents(wtLocal)
    /\ WTContents(wtLocal) \cap WTUnpositioned = {}
    /\ \A s \in WTSegments : s < f => wtData[s] \subseteq WTCover[c]
\* f may conservatively retain covered closed files. The implementation chooses
\* the first uncovered summary; preserving additional files cannot grant more
\* reclaim authority. Every retained file is replayed in full before filtering.

WTInit == wtPhase = "ready"
    /\ wtVisible = WTInitialTopology
    /\ wtDurable = WTInitialTopology
    /\ wtLocal = WTInitialTopology
    /\ wtCandidate = WTInitialTopology
    /\ wtPlan = WTInitialTopology
    /\ wtFiles = {1}
    /\ wtDiskFiles = {1}
    /\ wtData = [s \in WTSegments |-> {}]
    /\ wtAck = {}
    /\ wtView = {}
    /\ wtWriter = 1
    /\ wtRestored = TRUE

WTAck(r) == /\ wtPhase = "ready" /\ r \in WTRecords \ WTContents(wtLocal) /\ (wtLocal.anchor = 0 \/ r \notin WTUnpositioned)
    /\ wtData' = [wtData EXCEPT ![wtWriter] = @ \cup {r}]
    /\ wtAck' = wtAck \cup {r}
    /\ wtView' = wtView \cup {r}
    /\ UNCHANGED <<wtPhase, wtVisible, wtDurable, wtLocal, wtCandidate, wtPlan, wtFiles, wtDiskFiles, wtWriter, wtRestored>>

WTUnknown(r) == /\ wtPhase = "ready" /\ r \in WTRecords \ WTContents(wtLocal) /\ (wtLocal.anchor = 0 \/ r \notin WTUnpositioned)
    /\ wtData' = [wtData EXCEPT ![wtWriter] = @ \cup {r}]
    /\ wtPhase' = "fenced"
    /\ UNCHANGED <<wtVisible, wtDurable, wtLocal, wtCandidate, wtPlan, wtFiles, wtDiskFiles, wtAck, wtView, wtWriter, wtRestored>>

WTRotate == /\ wtPhase = "ready" /\ wtLocal.active < WTMax
    /\ wtCandidate' = [wtLocal EXCEPT !.active = @ + 1]
    /\ wtWriter' = 0
    /\ wtPhase' = "create"
    /\ UNCHANGED <<wtVisible, wtDurable, wtLocal, wtPlan, wtFiles, wtDiskFiles, wtData, wtAck, wtView, wtRestored>>

WTCreate == wtPhase = "create"
    /\ wtFiles' = wtFiles \cup {wtCandidate.active}
    /\ wtDiskFiles' = wtFiles \cup {wtCandidate.active}
    /\ wtPhase' = "write_top"
    /\ UNCHANGED <<wtVisible, wtDurable, wtLocal, wtCandidate, wtPlan, wtData, wtAck, wtView, wtWriter, wtRestored>>

WTCheckpoint(c, f) == /\ wtPhase = "ready" /\ WTCheckpointChoice(c, f)
    /\ wtCandidate' = [anchor |-> c, first |-> f, active |-> wtLocal.active]
    /\ wtPhase' = "write_top"
    /\ UNCHANGED <<wtVisible, wtDurable, wtLocal, wtPlan, wtFiles, wtDiskFiles, wtData, wtAck, wtView, wtWriter, wtRestored>>

WTTemporarySync == wtPhase = "write_top"
    /\ wtPhase' = "rename"
    /\ UNCHANGED <<wtVisible, wtDurable, wtLocal, wtCandidate, wtPlan, wtFiles, wtDiskFiles, wtData, wtAck, wtView, wtWriter, wtRestored>>

WTRename == wtPhase = "rename"
    /\ wtVisible' = wtCandidate
    /\ wtPhase' = "sync_top"
    /\ UNCHANGED <<wtDurable, wtLocal, wtCandidate, wtPlan, wtFiles, wtDiskFiles, wtData, wtAck, wtView, wtWriter, wtRestored>>

WTParentSync == wtPhase = "sync_top"
    /\ wtDurable' = wtVisible
    /\ wtPhase' = "install"
    /\ UNCHANGED <<wtVisible, wtLocal, wtCandidate, wtPlan, wtFiles, wtDiskFiles, wtData, wtAck, wtView, wtWriter, wtRestored>>

WTInstall == wtPhase = "install"
    /\ wtLocal' = wtCandidate
    /\ wtPlan' = wtCandidate
    /\ wtWriter' = wtCandidate.active
    /\ wtPhase' = "ready"
    /\ UNCHANGED <<wtVisible, wtDurable, wtCandidate, wtFiles, wtDiskFiles, wtData, wtAck, wtView, wtRestored>>

WTFail == wtPhase \in {"create", "write_top", "rename", "sync_top", "install", "restore", "stabilize", "replay"}
    /\ wtPhase' = "fenced"
    /\ UNCHANGED <<wtVisible, wtDurable, wtLocal, wtCandidate, wtPlan, wtFiles, wtDiskFiles, wtData, wtAck, wtView, wtWriter, wtRestored>>

WTCrash(t, files) == /\ wtPhase # "down" /\ t \in {wtVisible, wtDurable}
    /\ files \subseteq wtDiskFiles /\ wtFiles \subseteq files
    /\ wtVisible' = t
    /\ wtDurable' = t
    /\ wtLocal' = t
    /\ wtCandidate' = t
    /\ wtPlan' = t
    /\ wtFiles' = files
    /\ wtDiskFiles' = files
    /\ wtView' = {}
    /\ wtWriter' = 0
    /\ wtRestored' = FALSE
    /\ wtPhase' = "down"
    /\ UNCHANGED <<wtData, wtAck>>

WTRead == wtPhase = "down"
    /\ wtPlan' = wtVisible
    /\ wtRestored' = FALSE
    /\ wtPhase' = "restore"
    /\ UNCHANGED <<wtVisible, wtDurable, wtLocal, wtCandidate, wtFiles, wtDiskFiles, wtData, wtAck, wtView, wtWriter>>

WTRestore == /\ wtPhase = "restore" /\ wtPlan.anchor \in WTTrusted
    /\ wtRestored' = TRUE
    /\ wtPhase' = "stabilize"
    /\ UNCHANGED <<wtVisible, wtDurable, wtLocal, wtCandidate, wtPlan, wtFiles, wtDiskFiles, wtData, wtAck, wtView, wtWriter>>

WTRefuse == wtPhase \in {"restore", "stabilize", "replay"}
    /\ wtPhase' = "rejected"
    /\ UNCHANGED <<wtVisible, wtDurable, wtLocal, wtCandidate, wtPlan, wtFiles, wtDiskFiles, wtData, wtAck, wtView, wtWriter, wtRestored>>

WTRecoverySync == /\ wtPhase = "stabilize" /\ wtPlan = wtVisible
    /\ wtDurable' = wtPlan
    /\ wtPhase' = "replay"
    /\ UNCHANGED <<wtVisible, wtLocal, wtCandidate, wtPlan, wtFiles, wtDiskFiles, wtData, wtAck, wtView, wtWriter, wtRestored>>

WTReplay == /\ wtPhase = "replay" /\ wtRestored /\ WTAvailable(wtPlan)
    /\ wtLocal' = wtPlan
    /\ wtCandidate' = wtPlan
    /\ wtWriter' = wtPlan.active
    /\ wtView' = WTCover[wtPlan.anchor] \cup WTTail(wtPlan)
    /\ wtPhase' = "ready"
    /\ UNCHANGED <<wtVisible, wtDurable, wtPlan, wtFiles, wtDiskFiles, wtData, wtAck, wtRestored>>

WTUnlink(s) == /\ wtPhase = "ready" /\ wtLocal.anchor # 0 /\ s \in wtFiles /\ s < wtLocal.first
    /\ wtFiles' = wtFiles \ {s}
    /\ UNCHANGED <<wtPhase, wtVisible, wtDurable, wtLocal, wtCandidate, wtPlan, wtDiskFiles, wtData, wtAck, wtView, wtWriter, wtRestored>>

WTCleanupSync == wtPhase = "ready"
    /\ wtDiskFiles' = wtFiles
    /\ UNCHANGED <<wtPhase, wtVisible, wtDurable, wtLocal, wtCandidate, wtPlan, wtFiles, wtData, wtAck, wtView, wtWriter, wtRestored>>

WTQuiesce == UNCHANGED wtVars
WTNext == (\E r \in WTRecords : WTAck(r) \/ WTUnknown(r)) \/ WTRotate \/ WTCreate
    \/ (\E c \in WTAnchors, f \in WTSegments : WTCheckpoint(c, f))
    \/ WTTemporarySync \/ WTRename \/ WTParentSync \/ WTInstall \/ WTFail
    \/ (\E t \in WTTopologies, files \in SUBSET WTSegments : WTCrash(t, files))
    \/ WTRead \/ WTRestore \/ WTRefuse \/ WTRecoverySync \/ WTReplay
    \/ (\E s \in WTSegments : WTUnlink(s)) \/ WTCleanupSync \/ WTQuiesce
WTSpec == WTInit /\ [][WTNext]_wtVars

WTType == /\ wtPhase \in WTPhases
    /\ \A t \in {wtVisible, wtDurable, wtLocal, wtCandidate, wtPlan} : t \in WTTopologies
    /\ wtFiles \subseteq wtDiskFiles /\ wtDiskFiles \subseteq WTSegments
    /\ wtData \in [WTSegments -> SUBSET WTRecords]
    /\ wtAck \subseteq WTRecords /\ wtView \subseteq WTRecords
    /\ wtWriter \in WTSegments \cup {0} /\ wtRestored \in BOOLEAN
WTLayouts == /\ (\A i, j \in WTSegments : i # j => wtData[i] \cap wtData[j] = {})
    /\ (\A t \in {wtVisible, wtDurable, wtCandidate} : t.anchor # 0 => WTContents(t) \cap WTUnpositioned = {})
    /\ WTValid(wtVisible) /\ WTValid(wtDurable) /\ WTValid(wtCandidate)
    /\ WTAvailable(wtVisible) /\ WTAvailable(wtDurable)
    /\ wtVisible.active >= wtDurable.active /\ wtCandidate.active >= wtDurable.active
    /\ \A s \in WTSegments : s > wtDurable.active => wtData[s] = {}
WTAckRecoverable == wtAck \subseteq WTContents(wtVisible) /\ wtAck \subseteq WTContents(wtDurable)
WTCandidateSafe == wtAck \subseteq WTContents(wtCandidate)
WTAuthority == wtPhase = "ready" =>
    /\ wtLocal = wtVisible /\ wtVisible = wtDurable /\ wtCandidate = wtLocal
    /\ wtWriter = wtLocal.active /\ wtView = WTContents(wtLocal)
WTPlanAuthority == wtPhase \in {"restore", "stabilize", "replay"} => wtPlan = wtVisible
WTStage ==
    /\ (wtPhase \in {"down", "restore", "stabilize", "replay"} => wtCandidate = wtVisible)
    /\ (wtPhase = "create" => wtCandidate = [wtDurable EXCEPT !.active = @ + 1]
          /\ wtCandidate.active <= WTMax /\ wtView = WTContents(wtCandidate) /\ wtWriter = 0)
    /\ (wtPhase \in {"write_top", "rename", "sync_top", "install"} =>
          WTAvailable(wtCandidate) /\ wtView = WTContents(wtCandidate))
    /\ (wtPhase \in {"sync_top", "install"} => wtVisible = wtCandidate)
    /\ (wtPhase = "install" => wtDurable = wtCandidate)
    /\ (wtPhase \in {"stabilize", "replay"} => wtRestored)
    /\ (wtPhase = "replay" => wtDurable = wtPlan)
WTInvariant == WTType /\ WTLayouts /\ WTAckRecoverable /\ WTCandidateSafe
               /\ WTAuthority /\ WTPlanAuthority /\ WTStage
WTClosedContentsSafe == \A s \in WTClosed(wtDurable) : wtData'[s] = wtData[s]
WTClosedContentsAlways == [][WTClosedContentsSafe]_wtVars
WTAckMonotonic == wtAck \subseteq wtAck'
WTAckMonotonicAlways == [][WTAckMonotonic]_wtVars
WTDeleteSafe == \A s \in wtFiles \ wtFiles' :
    s \notin WTSelected(wtDurable) /\ wtData[s] \subseteq WTCover[wtDurable.anchor]
WTDeleteAlways == [][WTDeleteSafe]_wtVars
WTNoEarlyAck == wtAck' \ wtAck # {} =>
    wtWriter = wtDurable.active /\ wtLocal = wtDurable /\ wtVisible = wtDurable
WTNoEarlyAckAlways == [][WTNoEarlyAck]_wtVars
WTFenceStep == wtPhase = "fenced" => wtPhase' \in {"fenced", "down"}
WTFenceAlways == [][WTFenceStep]_wtVars
WTExactReplay == WTReplay => wtView' = WTCover[wtPlan.anchor] \cup WTTail(wtPlan)
WTNoAckWitness == wtAck = {}
WTNoAmbiguityWitness == wtVisible = wtDurable
WTNoReclaimWitness == wtFiles = wtDiskFiles
WTNoUnpositionedWitness == WTContents(wtVisible) \cap WTUnpositioned = {}
WTNoOrphanWitness == wtFiles \subseteq WTSelected(wtDurable) \cup 1..(wtDurable.first - 1)

\* Progress starts after a capacity/coverage check has admitted an operation.
\* Stable local execution has no new operations, crashes or I/O errors. Restore
\* may complete with a valid full state or reject; indefinite storage failures
\* and unavailable capacity or committed checkpoints are not progress premises.
WTStableNext == WTCreate \/ WTTemporarySync \/ WTRename \/ WTParentSync \/ WTInstall
    \/ WTRestore \/ WTRefuse \/ WTRecoverySync \/ WTReplay \/ WTQuiesce
WTFairSpec == WTInvariant /\ [][WTStableNext]_wtVars
    /\ WF_wtVars(WTCreate) /\ WF_wtVars(WTTemporarySync) /\ WF_wtVars(WTRename)
    /\ WF_wtVars(WTParentSync) /\ WF_wtVars(WTInstall)
    /\ WF_wtVars(WTRestore) /\ WF_wtVars(WTRefuse)
    /\ WF_wtVars(WTRecoverySync) /\ WF_wtVars(WTReplay)
WTInFlight == wtPhase \in {"create", "write_top", "rename", "sync_top", "install", "restore", "stabilize", "replay"}
WTSettled == wtPhase \in {"ready", "rejected"}
WTProgress == WTInFlight ~> WTSettled

WTCleanupNext == (\E s \in WTSegments : WTUnlink(s)) \/ WTCleanupSync \/ WTQuiesce
WTCleanupFairness == WF_wtVars(WTUnlink(WTReclaimTarget)) /\ WF_wtVars(WTCleanupSync)
WTCleanupSpec == WTInvariant /\ wtPhase = "ready" /\ [][WTCleanupNext]_wtVars
    /\ WTCleanupFairness
WTGarbage(s) == wtPhase = "ready" /\ wtLocal.anchor # 0 /\ s \in wtFiles /\ s < wtLocal.first
WTReclaimAll == \A s \in WTSegments : (WTGarbage(s) ~> (s \notin wtDiskFiles))
=============================================================================
