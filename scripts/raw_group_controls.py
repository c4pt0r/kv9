"""Single protocol faults for the exact-fe Raw group boundary."""
from ready_controls import replace_once


def mutations(models):
    original = models["RawGroup"]
    controls = []

    def add(name, changes, prop, *, config="Ordered.cfg", live=False, action=False):
        text = original
        for before, after in changes:
            text = replace_once(text, before, after)
        controls.append(dict(module="RawGroup", name=name, model=text, property=prop,
            proof_pattern=r"PROVE\s+", config=config, live=live, action=action))

    eligible = original[original.index("RGEligible(i) =="):original.index("RGVerdict(i, state) ==")]
    plan = original[original.index("RGPlan =="):original.index("RGPlanDone ==")]
    grow = original[original.index("RGCanGrow =="):original.index("RGGrow ==")]
    publish = original[original.index("RGPublish =="):original.index("\\* A later entry")]
    fail = original[original.index("RGFail =="):original.index("RGService ==")]
    effect = original[original.index("RGEffect =="):original.index("RGEngineSuccess ==")]
    add("reverse-mutation-composition", [(r'rgBatch \o RGEffective(rgPrepared + 1, RGInitial)', r'RGEffective(rgPrepared + 1, RGInitial) \o rgBatch')], "RGComposition")
    add("accept-stale-member", [(plan, replace_once(plan, '[rgStaged EXCEPT ![rgPrepared + 1] = RGVerdict(rgPrepared + 1, RGInitial)]', '[rgStaged EXCEPT ![rgPrepared + 1] = "ok"]'))], "RGPlanIdentity", config="Stale.cfg")
    add("ignore-epoch-read-error", [(r'/\ ~(RGKind[rgPrepared + 1] = "fenced" /\ rgPrepared + 1 \in RGReadFails)', r'/\ TRUE')], "RGPlanIdentity", config="ReadError.cfg")
    add("allow-hidden-system-mutation", [(eligible, replace_once(eligible, r'/\ RMRawBatch(RGOps[i])', r'/\ TRUE'))], "RGSelection", config="HiddenSystem.cfg")
    add("default-adjudicator-opt-in", [(eligible, replace_once(eligible, r'/\ (RGKind[i] = "fenced" => RGOptIn)', r'/\ TRUE'))], "RGSelection", config="OptOut.cfg")
    for kind in ("catalog", "manifest", "conf", "noop", "malformed"):
        add("cross-" + kind + "-barrier", [(eligible, replace_once(eligible, '{"put", "write", "fenced"}', '{"put", "write", "fenced", "' + kind + '"}'))], "RGSelection", config=kind.title() + "Barrier.cfg")
    add("unbounded-member-count", [(grow, replace_once(grow, r'/\ rgCount < RGMaxCount', r'/\ TRUE'))], "RGSelection", config="CountBound.cfg")
    add("ignore-encoded-byte-bound", [(grow, replace_once(grow, r'/\ RGWeight[rgCount + 1] <= RGMaxBytes - rgBytes', r'/\ TRUE'))], "RGSelection", config="ByteBound.cfg")
    add("reject-legal-singleton-fallback", [('THEN "singleton" ELSE "load"', 'THEN "fenced" ELSE "load"')], "RGFallbackSafety", config="Oversized.cfg", action=True)
    add("skip-intermediate-position-validation", [(r'/\ RGAdvances(rgPrevious, RGPosition[rgPrepared + 1])', r'/\ TRUE')], "RGPlanIdentity", config="BadPosition.cfg")
    add("synthesize-tail-position-pair", [(effect, replace_once(effect, "rgImagePosition' = RGPosition[rgCount]", "rgImagePosition' = [term |-> RGPosition[1].term, index |-> RGPosition[rgCount].index]"))], "RGEngineBinding")
    early = original[original.index("RGPlanDone =="):original.index("\\* This is the positioned")]
    early_mutant = replace_once(early, '/\\ rgPhase\' = "write"', '/\\ rgPhase\' = "write" /\\ rgSmPublication\' = RGPosition[rgCount]')
    early_mutant = replace_once(early_mutant, 'rgSmPublication, rgReceipts', 'rgReceipts')
    add("publish-sm-before-engine-success", [(early, early_mutant)], "RGPublication")
    add("tail-identity-for-every-receipt", [(publish, replace_once(publish, 'term |-> RGPosition[i].term, index |-> RGPosition[i].index', 'term |-> RGPosition[rgCount].term, index |-> RGPosition[rgCount].index'))], "RGReceiptsExact")
    add("wrong-rejected-region-identity", [(publish, replace_once(publish, 'THEN RGRegion[i] ELSE 0', 'THEN RGRegion[rgCount] ELSE 0'))], "RGReceiptsExact", config="Stale.cfg")
    unknown = replace_once(fail, '/\\ rgPhase\' = "fenced"', '/\\ rgPhase\' = "fenced" /\\ rgEngineOk\' = TRUE')
    unknown = replace_once(unknown, 'rgImagePosition, rgEngineOk,', 'rgImagePosition,')
    add("ignore-failed-engine-result", [(fail, unknown)], "RGPublication")
    add("lose-failure-fence", [(fail, replace_once(fail, 'rgPhase\' = "fenced"', "rgPhase' = rgPhase"))], "RGFenced")
    report = original[original.index("RGReport =="):original.index("RGFailureAt ==")]
    add("report-driver-before-ready-tail", [(report, replace_once(report, 'rgProcessed = RGItems', 'rgProcessed >= rgCount'))], "RGDriverAuthority", config="MalformedBarrier.cfg")
    tail = original[original.index("RGTailPass =="):original.index("RGReport ==")]
    add("skip-malformed-later-item", [(tail, replace_once(tail, 'RGKind[rgProcessed + 1] # "malformed"', 'TRUE'))], "RGContiguity", config="MalformedBarrier.cfg")
    add("omit-empty-batch-position", [(effect, replace_once(effect, "rgImagePosition' = RGPosition[rgCount]", "rgImagePosition' = IF rgBatch = <<>> THEN RGBasePosition ELSE RGPosition[rgCount]"))], "RGEngineBinding", config="Empty.cfg")
    add("wait-for-unavailable-group-member", [('rgPhase = "choose" /\\ ~RGCanGrow\n', 'rgPhase = "choose" /\\ ~RGCanGrow /\\ rgCount = RGMaxCount\n')], "RGProgress", config="MalformedBarrierLive.cfg", live=True)
    add("unfair-group-service", [('RGFairness == WF_rgVars(RGService)', 'RGFairness == TRUE')], "RGProgress", live=True)
    targets = {
        "reverse-mutation-composition": "RGPlanStep", "accept-stale-member": "RGPlanStep",
        "ignore-epoch-read-error": "RGPlanStep", "allow-hidden-system-mutation": "RGGrowStep",
        "default-adjudicator-opt-in": "RGGrowStep", "unbounded-member-count": "RGGrowStep",
        "ignore-encoded-byte-bound": "RGGrowStep", "reject-legal-singleton-fallback": "RGSingletonFallback",
        "skip-intermediate-position-validation": "RGPlanStep", "synthesize-tail-position-pair": "RGEffectStep",
        "publish-sm-before-engine-success": "RGPlanDoneStep", "tail-identity-for-every-receipt": "RGPublishStep",
        "wrong-rejected-region-identity": "RGPublishStep", "ignore-failed-engine-result": "RGFailStep",
        "lose-failure-fence": "RGFailStep", "report-driver-before-ready-tail": "RGReportStep",
        "skip-malformed-later-item": "RGTailPassStep", "omit-empty-batch-position": "RGEffectStep",
        "wait-for-unavailable-group-member": "RGChooseDoneEnabled", "unfair-group-service": "RGBudgetProgress",
    }
    for c in controls:
        c["proof_theorem"] = "RGGrowStep" if c["name"].startswith("cross-") else targets[c["name"]]
        # A structured proof can fail at an inner formula rather than its
        # named invariant. The gate requires the actual error source location
        # to fall inside this exact theorem, plus a nontrivial failed count.
    return controls
