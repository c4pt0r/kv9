"""One-mechanism protocol faults for the frozen first scheduling boundary."""
from ready_controls import replace_once


def mutations(models):
    result = []

    def add(module, name, changes, prop, pattern, *, live=False, action=False):
        text = models[module]
        for before, after in changes:
            text = replace_once(text, before, after)
        result.append(dict(module=module, name=name, model=text, property=prop,
                           proof_pattern=pattern, live=live, action=action))

    m = "RaftSchedule"
    inv = r"PROVE\s+RSInvariant'"
    add(m, "notify-before-publication", [
        ('rsProducer[p] = "publishing" /\\ rsQueue < RSMaxQueue', 'rsProducer[p] = "notified" /\\ rsQueue < RSMaxQueue'),
        ('rsProducer\' = [rsProducer EXCEPT ![p] = "published"]', 'rsProducer\' = [rsProducer EXCEPT ![p] = "idle"]'),
        ('rsProducer[p] = "published"\n    /\\ rsProducer\' = [rsProducer EXCEPT ![p] = "idle"]',
         'rsProducer[p] = "publishing"\n    /\\ rsProducer\' = [rsProducer EXCEPT ![p] = "notified"]')], "RSNoLostWake", inv)
    add(m, "clear-after-drain", [('(IF rsRedo /\\ ~rsStopped THEN TRUE ELSE rsPending)',
                                  '(IF rsRedo /\\ ~rsStopped THEN TRUE ELSE FALSE)')], "RSNoLostWake", inv)
    add(m, "lose-retained-notification", [('(IF rsRedo /\\ ~rsStopped THEN TRUE ELSE rsPending)', 'rsPending')], "RSNoLostWake", inv)
    add(m, "non-atomic-predicate-park", [('rsPhase = "wait" /\\ ~rsPending /\\ ~rsStopped',
                                         'rsPhase = "wait" /\\ ~rsStopped')], "RSParkedSignal", inv)
    add(m, "lose-stop-wake", [("rsWake' = (rsWake \\/ (rsPhase = \"park\"))", "rsWake' = rsWake")], "RSParkedSignal", inv)
    add(m, "begin-after-stop", [('rsPhase = "awake" /\\ ~rsStopped', 'rsPhase = "awake"')],
        "RSStopAlways", r"PROVE\s+RSStopMonotonic /\\ RSHaltTerminal /\\ RSNoBeginAfterStop", action=True)
    add(m, "duplicate-background-owner", [('o \\in RSOwners /\\ rsClaimed = {}', 'o \\in RSOwners')], "RSUniqueOwner", inv)
    add(m, "unfair-publication-callback", [('RSFairness == WF_rsVars(RSNotifyAny)', 'RSFairness == TRUE')],
        "RSServiceProgress", r"PROVE\s+RSServiceSpec\s*=>", live=True)
    add(m, "unfair-owner-turn", [(r'/\ WF_rsVars(RSBegin)', r'/\ TRUE')],
        "RSServiceProgress", r"PROVE\s+RSServiceSpec\s*=>", live=True)
    m = "RaftTick"
    add(m, "early-tick", [(r'/\ rtNow >= rtNext', r'/\ TRUE')], "RTSafety", r"PROVE\s+RTSpacing", action=True)
    add(m, "catch-up-tick-deadline", [("/\\ rtNext' = rtNow + RTPeriod", "/\\ rtNext' = rtNext + RTPeriod")],
        "RTInvariant", r"PROVE\s+RTInvariant'")
    add(m, "traffic-postpones-deadline", [('RTTraffic == UNCHANGED rtVars',
        "RTTraffic == rtNext' = rtNow + RTPeriod /\\ UNCHANGED <<rtNow, rtLast, rtHas>>")],
        "RTInvariant", r"PROVE\s+RTInvariant'")
    # The MC wrapper names the same fairness operator, so changing this single
    # operator affects the finite control and the parameterized theorem.
    add(m, "unfair-due-service", [('RTFairness == WF_rtVars(RTTick)', 'RTFairness == TRUE')],
        "RTProgress", r"PROVE\s+RTFairSpec => RTProgress", live=True)
    m = "RaftInboxBudget"
    add(m, "lose-inbox-count-cap", [(r'/\ rbCount < RBMaxMessages /\ w <= RBMaxBytes - rbBytes',
                                     r'/\ w <= RBMaxBytes - rbBytes')], "RBInvariant", r"PROVE\s+RBInvariant'")
    add(m, "lose-inbox-byte-cap", [(r'/\ rbCount < RBMaxMessages /\ w <= RBMaxBytes - rbBytes',
                                    r'/\ rbCount < RBMaxMessages')], "RBInvariant", r"PROVE\s+RBInvariant'")
    add(m, "lose-turn-count-cap", [(r'rbCount > 0 /\ rbRemovedCount < RBDrainMessages /\ rbRemovedBytes < RBDrainBytes',
                                    r'rbCount > 0 /\ rbRemovedBytes < RBDrainBytes')], "RBInvariant", r"PROVE\s+RBInvariant'")
    add(m, "lose-turn-byte-target", [(r'rbCount > 0 /\ rbRemovedCount < RBDrainMessages /\ rbRemovedBytes < RBDrainBytes',
                                      r'rbCount > 0 /\ rbRemovedCount < RBDrainMessages')], "RBSafety", r"PROVE\s+RBBoundedPop", action=True)
    return result
