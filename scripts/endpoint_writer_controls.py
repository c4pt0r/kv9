"""Single faults in endpoint authority and serialized route installation."""
from ready_controls import replace_once


def mutations(model):
    controls = []
    for name, before, after, prop, pattern in [
        ('retain-obsolete-admission', "ewAdmission' = IF registration THEN \"Consumed\" ELSE IF ewAdmission = \"None\" THEN \"None\" ELSE \"Revoked\"",
         "ewAdmission' = IF registration THEN \"Consumed\" ELSE ewAdmission", 'EWEffects', r'PROVE\s+EWRevocation'),
        ('registration-reuses-generation', 'IF registration /\\ a = ewAddress THEN 0 ELSE 1',
         'IF registration THEN 0 ELSE 1', 'EWEffects', r'PROVE\s+EWVersion'),
        ('unlocked-eager-writer', 'EWEager(a) == /\\ ~ewCapturing /\\ a \\in EWAddresses',
         'EWEager(a) == /\\ TRUE /\\ a \\in EWAddresses', 'EWEffects', r'PROVE\s+\\A a \\in EWAddresses : EWEager\(a\) => EWInvariant'),
        ('capture-wrong-source', "ewCaptureGeneration' = ewGeneration /\\ ewCaptureAddress' = ewAddress",
         "ewCaptureGeneration' = ewGeneration /\\ ewCaptureAddress' = EWInitial", 'EWInvariant', r'PROVE\s+EWStart => EWInvariant'),
        ('ignore-generation-limit', 'ewGeneration + EWDelta(a, registration) <= EWLimit',
         'ewGeneration + EWDelta(a, registration) <= EWLimit + 1', 'EWInvariant', r'PROVE\s+\\A a \\in EWAddresses, registration, local \\in BOOLEAN :\s+EWChange'),
        ('missing-start-fairness', 'EWStableFairness == WF_ewVars(EWStart) /\\ WF_ewVars(EWInstall)',
         'EWStableFairness == TRUE /\\ WF_ewVars(EWInstall)', 'EWConverges', r'PROVE\s+EWStableSpec => EWWaiting ~> EWFresh'),
        ('missing-install-fairness', 'EWStableFairness == WF_ewVars(EWStart) /\\ WF_ewVars(EWInstall)',
         'EWStableFairness == WF_ewVars(EWStart) /\\ TRUE', 'EWConverges', r'PROVE\s+EWStableSpec => EWOld ~> EWWaiting'),
    ]:
        spec = 'EWMCFromOld' if prop == 'EWConverges' else 'EWSpec'
        config = f'SPECIFICATION {spec}\nCONSTANTS EWAddresses = {{1, 2}} EWInitial = 1 EWLimit = 2\n'
        config += ('INVARIANT ' if prop == 'EWInvariant' else 'PROPERTY ') + prop + '\n'
        controls.append(dict(name=name, model=replace_once(model, before, after), property=prop,
                             proof_pattern=pattern, config=config))
    return controls
