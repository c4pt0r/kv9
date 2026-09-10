"""Single-defect controls for the native batch model and imported retry layer."""

from ready_controls import replace_once


def witness_mutations(models):
    """Remove a permitted ordering; its corresponding witness must disappear."""
    native = models["NativeBatch.tla"]
    return [
        dict(name="late-effect-order", filename="NativeBatch.tla", property="NBNoLateEffect",
             action=True, text=replace_once(native, "/\\ CREffect(i)",
                                           '/\\ CREffect(i) /\\ crPhase # "unknown"')),
        dict(name="collection-overlap-order", filename="NativeBatch.tla",
             property="NBNoCollectionOverlap", action=True,
             text=replace_once(native, "batch \\in NBBackgroundBatches /\\ nbBg < NBBgLimit",
                               'batch \\in NBBackgroundBatches /\\ nbReadPhase # "collecting" /\\ nbBg < NBBgLimit')),
    ]


def mutations(models):
    native, retry = models["NativeBatch.tla"], models["ClientRetry.tla"]
    cases = [
        ("partial-engine-effect", "NBWholeEffectSafety", True,
         replace_once(native, "nbStore' = RMApply(nbStore, nbCommand)",
                      "nbStore' = RMApply(nbStore, <<nbCommand[1]>>)"), "NBWholeEffectStepProof"),
        ("changed-command-order", "NBOrderedImage", False,
         replace_once(native, "nbCommand' = NBWriteBatch",
                      "nbCommand' = [i \\in 1..Len(NBWriteBatch) |-> NBWriteBatch[Len(NBWriteBatch) - i + 1]]"),
         "NBDispatchStep"),
        ("live-view-per-item", "NBReadAtomic", False,
         replace_once(native, "Append(nbResult, nbView[NBReadKeys[Len(nbResult) + 1]])",
                      "Append(nbResult, nbStore[NBReadKeys[Len(nbResult) + 1]])"), "NBReadItemStep"),
        ("reordered-read-positions", "NBReadAtomic", False,
         replace_once(native, "Append(nbResult, nbView[NBReadKeys[Len(nbResult) + 1]])",
                      "Append(nbResult, nbView[NBReadKeys[Len(NBReadKeys) - Len(nbResult)]])"),
         "NBReadItemStep"),
        ("partial-read-published", "NBReadAtomic", False,
         replace_once(native, 'nbReadPhase = "collecting" /\\ Len(nbResult) = Len(NBReadKeys)',
                      'nbReadPhase = "collecting" /\\ Len(nbResult) = 1'), "NBReadPublishStep"),
        ("foreign-applied-receipt", "NBAckBinding", False,
         replace_once(native, "CRSuccess /\\ nbReceipt' = NBPosition",
                      "CRSuccess /\\ nbReceipt' = <<NBPosition[1], NBPosition[2] + 1>>"), "NBSuccessStep"),
    ]
    result = [dict(name=n, filename="NativeBatch.tla", property=p, action=a, text=t,
                   proof_module="NativeBatchProof", proof_theorem=th) for n, p, a, t, th in cases]
    for name, old, new in [
        ("retry-unknown-batch", '/\\ crPhase = "ready" /\\ crSent = crClosed',
         '/\\ ((crPhase = "ready" /\\ crSent = crClosed) \\/ crPhase = "unknown")'),
        ("refuse-already-applied-batch", '/\\ crPhase = "pending" /\\ crApplied = {}',
         '/\\ crPhase = "pending" /\\ TRUE'),
    ]:
        result.append(dict(name=name, filename="ClientRetry.tla", property="CRAtMostOneEffect",
                           action=False, text=replace_once(retry, old, new),
                           proof_module="ClientRetryProof", proof_theorem="CRInvariantStep"))
    return result
