"""One-mechanism faults for the WAL topology proof and finite protocol model."""
from ready_controls import replace_once


def mutations(model):
    controls = []
    for name, before, after, prop, pattern in [
        ('early-successor-ack', 'WTInstall == wtPhase = "install"',
         'WTInstall == wtPhase \\in {"sync_top", "install"}',
         'WTAckRecoverable', r"PROVE\s+WTInstall => WTInvariant'"),
        ('unlink-before-durable-topology',
         r'WTUnlink(s) == /\ wtPhase = "ready" /\ wtLocal.anchor # 0 /\ s \in wtFiles /\ s < wtLocal.first',
         r'WTUnlink(s) == /\ wtPhase \in {"ready", "sync_top"} /\ wtCandidate.anchor # 0 /\ s \in wtFiles /\ s < wtCandidate.first',
         'WTLayouts', r"WTUnlink\(s\) => WTInvariant'"),
        ('mismatched-anchor-and-retained-set',
         "wtCandidate' = [anchor |-> c, first |-> f, active |-> wtLocal.active]",
         "wtCandidate' = [anchor |-> wtLocal.anchor, first |-> f, active |-> wtLocal.active]",
         'WTLayouts', r"PROVE\s+WTLayouts'"),
        ('drop-uncovered-closed-segment',
         r'/\ \A s \in WTSegments : s < f => wtData[s] \subseteq WTCover[c]',
         r'/\ TRUE', 'WTAckRecoverable', r"PROVE\s+WTContents\(wtLocal\)\s+\\subseteq\s+WTContents"),
        ('restore-after-namespace-stabilization',
         r'WTRecoverySync == /\ wtPhase = "stabilize" /\ wtPlan = wtVisible',
         r'WTRecoverySync == /\ wtPhase \in {"restore", "stabilize"} /\ wtPlan = wtVisible',
         'WTStage', r"PROVE\s+WTStage'"),
        ('lose-unpositioned-pin',
         r'/\ WTContents(wtLocal) \cap WTUnpositioned = {}',
         r'/\ TRUE', 'WTLayouts', r"PROVE\s+WTLayouts'"),
    ]:
        controls.append(dict(name=name, model=replace_once(model, before, after),
                             property=prop, proof_pattern=pattern))
    for name, start, end, before, after, prop, pattern in [
        ('lose-failure-fence', 'WTFail ==', 'WTCrash(t, files) ==',
         "wtPhase' = \"fenced\"", "wtPhase' = \"ready\"", 'WTAuthority', r"PROVE\s+WTFail => WTInvariant'"),
        ('elect-orphan-successor', 'WTRead ==', 'WTRestore ==',
         "wtPlan' = wtVisible",
         "wtPlan' = (IF wtVisible.active < WTMax /\\ wtVisible.active + 1 \\in wtFiles THEN [wtVisible EXCEPT !.active = @ + 1] ELSE wtVisible)",
         'WTPlanAuthority', r"PROVE\s+WTRead => WTInvariant'"),
        ('discard-retained-tail-on-replay', 'WTReplay ==', 'WTUnlink(s) ==',
         "wtView' = WTCover[wtPlan.anchor] \\cup WTTail(wtPlan)",
         "wtView' = WTCover[wtPlan.anchor]", 'WTAuthority', r"PROVE\s+WTReplay => WTInvariant'"),
    ]:
        first, last = model.index(start), model.index(end)
        original = model[first:last]
        controls.append(dict(name=name, model=replace_once(model, original, replace_once(original, before, after)),
                             property=prop, proof_pattern=pattern))
    controls.append(dict(name='unfair-parent-sync',
        model=replace_once(model, r'/\ WF_wtVars(WTParentSync)', r'/\ TRUE'),
        property='WTProgress', live=True, config='TopologyLive.cfg',
        proof_pattern=r'PROVE\s+WTFairSpec\s+=>\s+wtPhase\s*=\s*"sync_top"\s*~>\s*wtPhase\s*=\s*"install"'))
    controls.append(dict(name='unfair-reclamation',
        model=replace_once(model, r'WTCleanupFairness == WF_wtVars(WTUnlink(WTReclaimTarget))', r'WTCleanupFairness == TRUE'),
        property='WTReclaimAll', live=True, config='TopologyCleanup.cfg',
        proof_pattern=r'PROVE\s+WTCleanupSpec\s+=>\s+\(?WF_'))
    return controls
