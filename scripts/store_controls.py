"""Isolated store-lifecycle faults shared by TLC and TLAPS checks."""
from ready_controls import replace_once


def mutations(model):
    controls = []
    for name, before, after, prop, pattern in [
        ('reuse-prepared-identity', r'i \in SLIncarnations \ slUsed',
         r'i \in SLIncarnations', 'SLInvariant', r"SLPrepare\(i\) => SLInvariant'"),
        ('recreate-active-log', r'SLCreateLog == /\ SLHealthy /\ slPhase = 2',
         r'SLCreateLog == /\ SLHealthy /\ slPhase \in {2, 3}',
         'SLNoRecreatedLog', r'PROVE\s+SLRootBind'),
        ('start-before-publication', r'SLStart == /\ SLHealthy /\ slPhase = 3',
         r'SLStart == /\ SLHealthy /\ slWritten = 3', 'SLPermission', r'PROVE\s+SLRootBind'),
        ('activate-before-log-sync', r'SLWriteActivation == /\ SLHealthy /\ slPhase = 2 /\ slWritten = 2 /\ slLog = slInc',
         r'SLWriteActivation == /\ SLHealthy /\ slPhase = 2 /\ slWritten = 2 /\ TRUE',
         'SLActivationAlways', r'PROVE\s+SLActivationStep'),
        ('legacy-without-certificate', r'SLLegacyWrite == /\ SLHealthy /\ slPhase = 0 /\ slCert',
         r'SLLegacyWrite == /\ SLHealthy /\ slPhase = 0 /\ TRUE',
         'SLActivationAlways', r'PROVE\s+SLActivationStep'),
        ('legacy-without-log', r"/\ slLog = slInc /\ slRoot = slInc /\ slWritten' = 3",
         r"/\ TRUE /\ slRoot = slInc /\ slWritten' = 3",
         'SLActivationAlways', r'PROVE\s+SLActivationStep'),
        ('start-after-publication-error', r'SLStart == /\ SLHealthy',
         r'SLStart == /\ slLive', 'SLPermission', r'PROVE\s+SLRootBind'),
    ]:
        control = dict(name=name, model=replace_once(model, before, after), property=prop, proof_pattern=pattern)
        if prop == 'SLActivationAlways':
            control['config'] = 'SPECIFICATION SLSpec\nCONSTANT SLIncarnations = {1, 2, 3}\nPROPERTY SLActivationAlways\n'
        controls.append(control)
    controls.append(dict(name='unfair-owner',
        model=replace_once(model, '/\\ WF_slVars(SLStart)', '/\\ TRUE'),
        property='SLProgress', config='SPECIFICATION SLFairSpec\nCONSTANT SLIncarnations = {1, 2}\nPROPERTY SLProgress\n',
        proof_pattern=r'PROVE\s+SLFairSpec => SLActiveReady ~> SLStarted'))
    return controls
