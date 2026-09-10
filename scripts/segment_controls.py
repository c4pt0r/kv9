"""Isolated segment durability faults shared by finite checks and TLAPS proofs."""
from ready_controls import replace_once


def mutations(model):
    controls = []
    for name, before, after, prop, pattern in [
        ('split-data-position', "sgPosition' = [sgPosition EXCEPT ![sgWritten + 1] = sgPending]",
         "sgPosition' = [sgPosition EXCEPT ![sgWritten + 1] = 0]", 'SGBinding', r"PROVE\s+SGPrefix'"),
        ('overwrite-first-position',
         'SGFoldFirst(first, n, r) == IF first = 0 /\\ r \\in SGPositioned THEN n + 1 ELSE first',
         'SGFoldFirst(first, n, r) == IF r \\in SGPositioned THEN n + 1 ELSE first',
         'SGFoldLaw', r'PROVE\s+SGFirstOf\('),
        ('forget-last-at-unpositioned',
         'SGFoldLast(last, n, r) == IF r \\in SGPositioned THEN n + 1 ELSE last',
         'SGFoldLast(last, n, r) == IF r \\in SGPositioned THEN n + 1 ELSE 0',
         'SGFoldLaw', r'PROVE\s+SGLastOf\('),
        ('ack-before-fsync', 'SGPublish == /\\ sgPhase = "publish"',
         'SGPublish == /\\ sgPhase \\in {"sync", "publish"}', 'SGAcknowledged', r"PROVE\s+SGPublish => SGInvariant'"),
        ('reuse-failed-writer', "sgPhase' = \"fenced\"", "sgPhase' = \"ready\"",
         'SGAuthority', r"PROVE\s+SGFail => SGInvariant'"),
        ('discard-acknowledged-prefix', r'd \in sgDurable..sgWritten', r'd \in 0..sgWritten',
         'SGAcknowledged', r"SGCrash\(d, t\) => SGInvariant'"),
        ('skip-position-validation', r'SGStart(f) == /\ sgPhase = "ready" /\ sgWritten < SGMax /\ SGCanAppend(f)',
         r'SGStart(f) == /\ sgPhase = "ready" /\ sgWritten < SGMax /\ f \in SGFrames',
         'SGOrder', r"SGStart\(f\) => SGInvariant'"),
        ('accept-wrong-file', r'SGValidate == /\ sgPhase = "scan" /\ sgIdentity = SGExpected /\ ~sgBad',
         r'SGValidate == /\ sgPhase = "scan" /\ TRUE /\ ~sgBad',
         'SGAuthority', r"PROVE\s+SGValidate => SGInvariant'"),
        ('accept-complete-corruption', r'SGValidate == /\ sgPhase = "scan" /\ sgIdentity = SGExpected /\ ~sgBad',
         r'SGValidate == /\ sgPhase = "scan" /\ sgIdentity = SGExpected /\ TRUE',
         'SGAuthority', r"PROVE\s+SGValidate => SGInvariant'"),
        ('repair-before-validation', r'SGRepair == /\ sgPhase = "validated" /\ ~sgClosed',
         r'SGRepair == /\ sgPhase \in {"scan", "validated"} /\ ~sgClosed',
         'SGAuthority', r"PROVE\s+SGRepair => SGInvariant'"),
        ('publish-unsynchronized-repair', "sgPhase' = \"recover_sync\"", "sgPhase' = \"recover_publish\"",
         'SGAuthority', r"PROVE\s+SGRepair => SGInvariant'"),
    ]:
        controls.append(dict(name=name, model=replace_once(model, before, after), property=prop, proof_pattern=pattern))
    start = model.index('SGRecoveryPublish ==')
    end = model.index('SGClosedReplay ==', start)
    before = model[start:end]
    after = replace_once(before, "sgPin' = SGPinned(sgWritten, sgData)", "sgPin' = FALSE")
    controls.append(dict(name='discard-unpositioned-pin', model=replace_once(model, before, after),
                         property='SGSummary', proof_pattern=r"PROVE\s+SGRecoveryPublish => SGInvariant'"))
    start = model.index('SGClosedReplay ==')
    end = model.index('SGQuiesce ==', start)
    before = model[start:end]
    after = replace_once(before, "sgPhase' = \"closed\"", "sgPhase' = \"ready\"")
    controls.append(dict(name='reopen-closed-writer', model=replace_once(model, before, after),
                         property='SGInvariant', proof_pattern=r"PROVE\s+SGClosedReplay => SGInvariant'"))
    controls.append(dict(name='unfair-recovery-sync',
        model=replace_once(model, r'/\ WF_sgVars(SGRecoverySync)', r'/\ TRUE'),
        property='SGProgress', live=True, proof_pattern=r'PROVE\s+SGFairSpec\s+=>\s+sgPhase\s*=\s*"recover_sync"\s*~>\s*sgPhase\s*=\s*"recover_publish"'))
    return controls
