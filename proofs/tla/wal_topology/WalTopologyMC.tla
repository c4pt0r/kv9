------------------------- MODULE WalTopologyMC -------------------------
EXTENDS WalTopology
WTMCCover == [c \in WTAnchors |-> CASE c = 0 -> {} [] c = 1 -> {1} [] OTHER -> {1, 2}]
WTMCEmpty == [phase |-> "ready", visible |-> WTInitialTopology, durable |-> WTInitialTopology,
    local |-> WTInitialTopology, candidate |-> WTInitialTopology, plan |-> WTInitialTopology,
    files |-> {1}, diskFiles |-> {1}, data |-> [s \in WTSegments |-> {}],
    ack |-> {}, view |-> {}, writer |-> 1, restored |-> TRUE]
WTMCNew == [WTInitialTopology EXCEPT !.active = 2]
WTMCCuts == {
    [WTMCEmpty EXCEPT !.phase = "create", !.candidate = WTMCNew, !.writer = 0],
    [WTMCEmpty EXCEPT !.phase = "write_top", !.candidate = WTMCNew, !.writer = 0,
        !.files = {1, 2}, !.diskFiles = {1, 2}],
    [WTMCEmpty EXCEPT !.phase = "rename", !.candidate = WTMCNew, !.writer = 0,
        !.files = {1, 2}, !.diskFiles = {1, 2}],
    [WTMCEmpty EXCEPT !.phase = "sync_top", !.candidate = WTMCNew, !.visible = WTMCNew,
        !.writer = 0, !.files = {1, 2}, !.diskFiles = {1, 2}],
    [WTMCEmpty EXCEPT !.phase = "install", !.candidate = WTMCNew, !.visible = WTMCNew,
        !.durable = WTMCNew, !.writer = 0, !.files = {1, 2}, !.diskFiles = {1, 2}],
    [WTMCEmpty EXCEPT !.phase = "restore", !.data[1] = {1}, !.ack = {1},
        !.view = {}, !.writer = 0, !.restored = FALSE]}
WTMCFairInit == \E cut \in WTMCCuts :
    /\ wtPhase = cut.phase /\ wtVisible = cut.visible /\ wtDurable = cut.durable
    /\ wtLocal = cut.local /\ wtCandidate = cut.candidate /\ wtPlan = cut.plan
    /\ wtFiles = cut.files /\ wtDiskFiles = cut.diskFiles /\ wtData = cut.data
    /\ wtAck = cut.ack /\ wtView = cut.view /\ wtWriter = cut.writer /\ wtRestored = cut.restored
WTMCLiveSpec == WTMCFairInit /\ WTFairSpec
WTMCReclaimTopology == [anchor |-> 1, first |-> 2, active |-> 2]
WTMCReclaimInit ==
    /\ wtPhase = "install" /\ wtVisible = WTMCReclaimTopology /\ wtDurable = WTMCReclaimTopology
    /\ wtLocal = [anchor |-> 0, first |-> 1, active |-> 2] /\ wtCandidate = WTMCReclaimTopology /\ wtPlan = WTMCReclaimTopology
    /\ wtFiles = {1, 2} /\ wtDiskFiles = {1, 2} /\ wtData = [s \in WTSegments |-> {s}]
    /\ wtAck = {1, 2} /\ wtView = {1, 2} /\ wtWriter = 2 /\ wtRestored = TRUE
\* Begin at the real post-parent-sync/pre-install checkpoint cut. The mandatory
\* installation gives the negative fairness trace a nonempty protocol prefix.
WTCleanupLiveSpec == WTMCReclaimInit /\ WTInvariant
    /\ [][WTInstall \/ WTCleanupNext]_wtVars
    /\ WF_wtVars(WTInstall) /\ WTCleanupFairness
=============================================================================
