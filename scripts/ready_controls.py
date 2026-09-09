"""Isolated protocol mutations shared by the Ready TLC and TLAPS gates."""


def replace_once(source, before, after):
    if before == after or source.count(before) != 1:
        raise ValueError(f"non-unique Ready mutation target: {before!r}")
    return source.replace(before, after)


def mutations(model):
    failed = model[model.index("RPFailBefore =="):model.index("RPFailAfter ==")]
    emitted = replace_once(failed, "rpLight, rpPublished>>", "rpLight>>")
    emitted = replace_once(emitted, "/\\ UNCHANGED", "/\\ rpPublished' = TRUE\n    /\\ UNCHANGED")
    return [
        {"name": "ready-skip-light-sync", "property": "RPCoverage",
         "model": replace_once(model,
             'rpPhase\' = IF c = rpDiskCommit THEN "publish" ELSE "light"',
             'rpPhase\' = "publish"'),
         "proof_pattern": r"PROVE\s+\\A c \\in 0\.\.RPMaxIndex : RPAdvance\(c\) => RPInvariant'"},
        {"name": "ready-publish-after-error", "property": "RPNoFailedPublication",
         "model": replace_once(model, failed, emitted),
         "proof_pattern": r"PROVE\s+RPFailBefore => RPInvariant'"},
        {"name": "ready-apply-after-fatal", "property": "RPFatalFreeze",
         "model": replace_once(model,
             'RPApply(c) ==\n    /\\ rpPhase # "failed" /\\ c \\in (rpApplied + 1)..rpDelivered',
             'RPApply(c) ==\n    /\\ TRUE /\\ c \\in (rpApplied + 1)..rpDelivered'),
         "proof_pattern": r"PROVE\s+RPFailedStop"},
    ]
