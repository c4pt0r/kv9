"""Isolated catalog endpoint CAS faults shared by TLC and TLAPS checks."""
from ready_controls import replace_once


def mutations(model):
    controls = []
    for name, before, after, prop, pattern in [
        ('ignore-cas-generation', r'/\ epGeneration = epExpected /\ epAddress = epOld',
         r'/\ TRUE /\ epAddress = epOld', 'EPEffects', r'PROVE\s+EPInvariant => EPChangePrecondition'),
        ('ignore-store-binding', r'EPMatch == epClaimedStore = EPStore /\ epExpected < EPLimit',
         r'EPMatch == TRUE /\ epExpected < EPLimit', 'EPEffects', r'PROVE\s+EPInvariant => EPChangePrecondition'),
        ('ignore-confirmed-generation', r'/\ epGeneration = epExpected + 1 /\ epAddress = epNew',
         r'/\ TRUE /\ epAddress = epNew', 'EPEffects', r'PROVE\s+EPInvariant => EPConfirmation'),
        ('ignore-confirmed-old-address', r'/\ epAddress = epNew /\ epPrevious = epOld',
         r'/\ epAddress = epNew /\ TRUE', 'EPEffects', r'PROVE\s+EPInvariant => EPConfirmation'),
        ('reuse-endpoint-generation', r"THEN /\ epGeneration' = epGeneration + 1",
         r"THEN /\ epGeneration' = epGeneration", 'EPEffects', r'PROVE\s+EPAtomicVersion'),
        ('ignore-generation-exhaustion', r'EPMatch == epClaimedStore = EPStore /\ epExpected < EPLimit',
         r'EPMatch == epClaimedStore = EPStore /\ epExpected <= EPLimit', 'EPInvariant', r'PROVE\s+EPEvaluate => EPInvariant'),
        ('write-on-refusal', r'''ELSE /\ UNCHANGED EPDirectory /\ epState' = "Refused"''',
         r'''ELSE /\ epGeneration' = epGeneration /\ epPrevious' = epPrevious /\ epAddress' = epNew /\ epState' = "Refused"''',
         'EPEffects', r'PROVE\s+EPInvariant => EPNoRefusalWrite'),
        ('missing-completion-fairness', r'EPFairSpec == EPInvariant /\ [][EPNext]_epVars /\ WF_epVars(EPEvaluate)',
         r'EPFairSpec == EPInvariant /\ [][EPNext]_epVars /\ TRUE', 'EPProgress', r'PROVE\s+EPFairSpec => EPPending ~> EPDone'),
    ]:
        spec = 'EPMCFairSpec' if prop == 'EPProgress' else 'EPSpec'
        config = f'SPECIFICATION {spec}\nCONSTANTS EPAddresses = {{1, 2}} EPStores = {{1, 2}} EPStore = 1 EPInitial = 1 EPLimit = 3\n'
        config += ('INVARIANT ' if prop == 'EPInvariant' else 'PROPERTY ') + prop + '\n'
        controls.append(dict(name=name, model=replace_once(model, before, after), property=prop,
                             proof_pattern=pattern, config=config))
    return controls
