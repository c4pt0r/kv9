"""Isolated receive-authority faults shared by TLC and TLAPS checks."""
from ready_controls import replace_once


def mutations(model):
    controls = []
    for name, before, after, prop, pattern in [
        ('receive-before-grant', r'GRReceive == /\ grLive /\ grGate',
         r'GRReceive == /\ grLive /\ TRUE', 'GRSafety', r'PROVE\s+GRReply'),
        ('start-before-grant', r'GRStart == /\ grLive /\ grGate',
         r'GRStart == /\ grLive /\ TRUE', 'GRPermission', r'PROVE\s+GRReply'),
        ('grant-without-receipt', r'GRGrant == /\ grLive /\ grReceipt = grLocal',
         r'GRGrant == /\ grLive /\ TRUE', 'GRPermission', r'PROVE\s+GRReply'),
        ('recover-another-store', r"/\ grGate' = (grCertificate = i)",
         r"/\ grGate' = (grCertificate # 0)", 'GRPermission', r"GRRestart\(i\) => GRInvariant'"),
        ('route-before-validation', r'GRRoute(i) == /\ i \in GRIncarnations /\ grBound = i',
         r'GRRoute(i) == /\ i \in GRIncarnations /\ TRUE', 'GRRouting', r"GRRoute\(i\) => GRInvariant'"),
        ('rebind-existing-replica', r'GRBind(i) == /\ i \in GRIncarnations /\ grBound = 0',
         r'GRBind(i) == /\ i \in GRIncarnations /\ TRUE', 'GRInvariant', r"GRBind\(i\) => GRInvariant'"),
    ]:
        controls.append(dict(name=name, model=replace_once(model, before, after), property=prop, proof_pattern=pattern))
    controls.append(dict(name='unfair-owner',
        model=replace_once(model, '/\\ WF_grVars(GRStart)', '/\\ TRUE'),
        property='GRProgress', config='SPECIFICATION GRFairSpec\nCONSTANT GRIncarnations = {1, 2}\nPROPERTY GRProgress\n',
        proof_pattern=r'PROVE\s+GRFairSpec => GRGated ~> GRRunning'))
    return controls
