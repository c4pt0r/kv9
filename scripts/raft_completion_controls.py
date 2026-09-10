"""Isolated protocol faults for completion wakeups and queued outbound work."""
from ready_controls import replace_once


def mutations(models):
    controls = []

    def add(module, name, changes, prop, pattern, *, config=None, live=False, action=False):
        text = models[module]
        for before, after in changes:
            text = replace_once(text, before, after)
        controls.append(dict(module=module, name=name, model=text, property=prop,
            proof_pattern=pattern, config=config, live=live, action=action))

    m = "RaftCompletion"
    inv = r"PROVE\s+RCInvariant'"
    original = models[m]
    observe = original[original.index('RCObserve(w) =='):original.index('RCCheck(w) ==')]
    check = original[original.index('RCCheck(w) =='):original.index('RCRegister(w) ==')]
    later_observe = replace_once(observe, 'rcPhase[w] = "observe"', 'rcPhase[w] = "check"')
    later_observe = replace_once(later_observe, 'THEN "error" ELSE "check"', 'THEN "error" ELSE "register"')
    earlier_check = replace_once(check, 'rcPhase[w] = "check"', 'rcPhase[w] = "observe"')
    earlier_check = replace_once(earlier_check, 'THEN "success" ELSE "register"', 'THEN "success" ELSE "check"')
    add(m, "observe-after-condition-check", [(observe, later_observe), (check, earlier_check)], "RCNoLostCompletion", inv)
    add(m, "non-atomic-generation-registration", [(r'/\ rcGeneration # RCDead /\ rcGeneration = rcSeen[w]', r'/\ rcGeneration # RCDead')], "RCRegisteredSignal", inv)
    add(m, "omit-post-state-notification", [('rcPublisher\' = [rcPublisher EXCEPT ![t] = "written"]', 'rcPublisher\' = [rcPublisher EXCEPT ![t] = "done"]')], "RCNoLostCompletion", inv)
    add(m, "wake-only-one-waiter", [('rcWake\' = {w \\in RCWaiters : rcPhase[w] = "park"}', 'rcWake\' = {w \\in RCWaiters : rcPhase[w] = "park" /\\ w = RCChosen}')], "RCRegisteredSignal", inv)
    add(m, "wrapping-generation", [('IF g = RCDead \\/ g = RCMaxGeneration THEN RCDead ELSE g + 1', 'IF g = RCDead \\/ g = RCMaxGeneration THEN 0 ELSE g + 1')], "RCSafety", r"PROVE\s+RCGenerationOrder", action=True)
    add(m, "successful-exhausted-publication", [('rcNotifyOk\' = (RCAdvance(rcGeneration) # RCDead)', 'rcNotifyOk\' = TRUE')], "RCPublication", inv)
    add(m, "hint-as-exact-authority", [(check, replace_once(check, 'RCTarget[w] \\in rcEvidence', 'rcGeneration # rcSeen[w]'))], "RCSafety", r"PROVE\s+RCGenerationOrder", action=True)
    add(m, "evicted-receipt-as-retained-authority", [(check, replace_once(check, 'RCTarget[w] \\in rcEvidence', 'RCTarget[w] \\in rcPublished'))], "RCSafety", r"PROVE\s+RCGenerationOrder", action=True)
    add(m, "silent-stop", [('RCStop == rcStopped\' = TRUE /\\ rcHint\' = TRUE', 'RCStop == rcStopped\' = TRUE /\\ rcHint\' = rcHint')], "RCSafety", r"PROVE\s+RCGenerationOrder", action=True)
    add(m, "silent-fatal", [('RCFatal == rcFatal\' = TRUE /\\ rcHint\' = TRUE', 'RCFatal == rcFatal\' = TRUE /\\ rcHint\' = rcHint')], "RCNoLostFatal", inv)
    add(m, "unfair-notification-publication", [('RCFairness == WF_rcVars(RCNotify)', 'RCFairness == TRUE')], "RCRecheckProgress", r"PROVE\s+RCFairSpec\s*=>", live=True)
    add(m, "unfair-chosen-waiter", [(r'/\ WF_rcVars(RCReturn(RCChosen))', r'/\ TRUE')], "RCRecheckProgress", r"PROVE\s+RCFairSpec\s*=>", live=True)
    m = "RaftOutbound"
    add(m, "receive-unadmitted-envelope", [('roBytes < ROByteTarget /\\ roHead <= roArrived', 'roBytes < ROByteTarget /\\ roHead <= ROMaxItems')], "ROPrefix", r"PROVE\s+ROPrefix'")
    add(m, "address-equality-as-session-identity", [('ROMatches(i) == RORoute[i] = ROSession', 'ROMatches(i) == ROAddress[RORoute[i]] = ROAddress[ROSession]')], "ROExactDestination", r"PROVE\s+ROExactDestination'", config="OutboundStale.cfg")
    add(m, "stale-envelope-not-counted", [('roInspected\' = roInspected + 1', 'roInspected\' = (IF ROMatches(roHead) THEN roInspected + 1 ELSE roInspected)')], "ROPrefix", r"PROVE\s+ROPrefix'", config="OutboundStale.cfg")
    add(m, "unbounded-inspection-count", [('ROCanPop == roInspected < ROMaxBatch /\\ roBytes < ROByteTarget', 'ROCanPop == roBytes < ROByteTarget')], "ROPrefix", r"PROVE\s+ROPrefix'")
    add(m, "ignore-byte-target", [('ROCanPop == roInspected < ROMaxBatch /\\ roBytes < ROByteTarget', 'ROCanPop == roInspected < ROMaxBatch')], "ROByteLedger", r"PROVE\s+roBytes' < ROByteTarget", config="OutboundBytes.cfg")
    add(m, "overwrite-fifo-output-slot", [('[roOrder EXCEPT ![roHead] = roCount + 1]', '[roOrder EXCEPT ![roHead] = 1]')], "ROFIFO", r"PROVE\s+roAccepted' = roAccepted \\cup \{roHead\}[\s\S]*?roOrder' = \[roOrder EXCEPT !\s*\[roHead\] = roCount \+ 1\]")
    add(m, "rebind-batch-to-current-client", [('roClient\' = ROSession', 'roClient\' = roCurrent')], "ROCapturedClient", r"PROVE\s+ROInvariant'")
    add(m, "wait-for-future-arrival", [('ROFinish == roPhase = "coalesce" /\\ ~ROCanPop', 'ROFinish == roPhase = "coalesce" /\\ (roInspected >= ROMaxBatch \\/ roBytes >= ROByteTarget)')], "ROProgress", r"PROVE\s+ENABLED ROService", live=True)
    add(m, "unfair-coalescing-service", [('ROFairness == WF_roVars(ROService)', 'ROFairness == TRUE')], "ROProgress", r"PROVE\s+ROBudgetDrop\(k\)", live=True)
    return controls
