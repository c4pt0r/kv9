"""Isolated faults in endpoint confirmation, durable authority and fair recovery."""
from ready_controls import replace_once


def mutations(model):
    config = ('CONSTANTS ERAddresses = {1, 2} ERLimit = 5\n'
              'CONSTANT ERVersion <- ERVersions\nCONSTANT ERAddress <- ERAddressesAt\n')
    cases = [
        ('serve-before-publication',
         'ERServe == /\\ ~erServing /\\ ERSavedUsable', 'ERServe == /\\ ~erServing /\\ TRUE',
         'ERInvariant', r'PROVE\s+ERServe => ERInvariant'),
        ('publish-before-apply',
         'ERPublish == /\\ erPending > 0 /\\ erApplied >= erPending',
         'ERPublish == /\\ erPending > 0 /\\ TRUE',
         'ERInvariant', r'PROVE\s+ERPublish => ERInvariant'),
        ('publish-superseded-confirmation',
         '/\\ ERVersion[erApplied] = ERVersion[erPending]\n             /\\ ERAddress',
         '/\\ TRUE\n             /\\ ERAddress',
         'EREffects', r'PROVE\s+ERPublication'),
        ('initial-authority-after-migration',
         'ERInitial == /\\ ~erSaved /\\ ERVersion[erApplied] = 0',
         'ERInitial == /\\ ~erSaved /\\ TRUE',
         'ERInvariant', r'PROVE\s+ERInitial => ERInvariant'),
        ('confirmation-reuses-prior-position',
         "erCommit' = erCommit + 1 /\\ erPending' = erCommit + 1",
         "erCommit' = erCommit + 1 /\\ erPending' = erCommit",
         'EREffects', r'PROVE\s+ERFreshReceipt'),
        ('confirm-unregistered-address',
         '/\\ erCommit < ERLimit /\\ ERAddress[erCommit] = erConfigured',
         '/\\ erCommit < ERLimit /\\ TRUE',
         'ERInvariant', r'PROVE\s+ERAddress\[erCommit \+ 1\] = erConfigured'),
    ]
    for action, predicate in [('ERApply', 'ERCatching'), ('ERConfirm', 'ERNeed'),
                              ('ERPublish', 'ERObserved'), ('ERServe', 'ERReady')]:
        cases.append((f'missing-{action}-fairness', f'WF_erVars({action})', 'TRUE', 'ERConverges',
                      rf'PROVE\s+ERStableSpec\s+=>\s+{predicate}\s+~>'))
    result = []
    for name, before, after, prop, pattern in cases:
        live = prop == 'ERConverges'
        run = config.replace('ERLimit = 5', 'ERLimit = 3') if live else config
        run = f'SPECIFICATION {"ERMCStableSpec" if live else "ERSpec"}\n' + run
        run += ('INVARIANT ' if prop == 'ERInvariant' else 'PROPERTY ') + prop + '\n'
        result.append(dict(name=name, model=replace_once(model, before, after), property=prop,
                           proof_pattern=pattern, config=run))
    return result
