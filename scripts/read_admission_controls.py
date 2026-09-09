"""Single-defect read-admission controls shared by TLC and deductive checking."""
from ready_controls import replace_once


def mutations(model):
    cases = [
        ('ignore-current-term', 'ELSE IF raCommitTerm = raTerm', 'ELSE IF TRUE',
         'RAInvariant', r'PROVE\s+RAInvariant => RAAdmission'),
        ('ignore-leadership', 'IF ~raLeader', 'IF FALSE',
         'RAEffects', r'PROVE\s+RAInvariant => RAAdmission'),
        ('forget-deferred-context', 'ELSE UNCHANGED <<raPhase, RARequest>>',
         "ELSE /\\ raContext' = RAOther /\\ UNCHANGED <<raPhase, raSubmits, raAdmittedTerm, raAdmittedCommitTerm>>",
         'RAInvariant', r'PROVE\s+RAPoll => RAInvariant'),
        ('accept-another-context', 'IF c = RAOwn', 'IF TRUE',
         'RAInvariant', r'PROVE\s+.*RADeliver.*RAInvariant'),
        ('skip-apply-coverage', 'RAComplete == /\\ raPhase = "Confirmed" /\\ raApplied',
         'RAComplete == /\\ raPhase = "Confirmed" /\\ TRUE',
         'RAInvariant', r'PROVE\s+RAComplete => RAInvariant'),
        ('submit-more-than-once', 'RAPoll == /\\ raPhase = "Waiting"',
         'RAPoll == /\\ raPhase \\in {"Waiting", "Submitted"}',
         'RAInvariant', r'PROVE\s+RAPoll => RAInvariant'),
        ('grow-request-budget', "raBudget' = raBudget - 1", "raBudget' = raBudget + 1",
         'RAInvariant', r'PROVE\s+RATick => RAInvariant'),
        ('missing-admission-fairness', '/\\ WF_raVars(RAPoll)', '/\\ TRUE',
         'RASuccess', r'PROVE\s+RAStableSpec => RAStage1 ~> RAStage2'),
    ]
    controls = []
    for name, before, after, prop, pattern in cases:
        config = ('SPECIFICATION RAMCSuccess\n' if prop == 'RASuccess' else 'SPECIFICATION RAMCSpec\n')
        config += 'CONSTANTS RAOwn = 1 RAOther = 2 RABudget = 2 RATermBound = 3\n'
        config += ('INVARIANT ' if prop == 'RAInvariant' else 'PROPERTY ') + prop + '\n'
        controls.append(dict(name=name, model=replace_once(model, before, after), property=prop,
                             proof_pattern=pattern, config=config))
    return controls
