"""Isolated first-formation faults shared by TLC and TLAPS checks."""
from ready_controls import replace_once


def mutations(model):
    controls = []
    for name, before, after, prop, pattern in [
        ('nonpristine-formation-fence', r"rfCanForm' = (local = RFRoot /\ owned /\ ~rfMarker)",
         r"rfCanForm' = (local = RFRoot /\ owned /\ ~rfMarker /\ ~RFRaftSeen)",
         'RFProgress', r'PROVE\s+RFCrashed /\\ RFRestore => RFReady'),
        ('plan-before-drain', r"/\ rfBarrier /\ rfApplied = 0 /\ rfPlan' = rfTerm",
         r"/\ TRUE /\ rfApplied = 0 /\ rfPlan' = rfTerm",
         'RFSafety', r'PROVE\s+RFDrain'),
        ('append-in-another-term', r'RFAppend == /\ rfLive /\ rfPlan # 0 /\ rfPlan = rfTerm',
         r'RFAppend == /\ rfLive /\ rfPlan # 0 /\ TRUE', 'RFSafety', r'PROVE\s+RFStepSafety'),
        ('forget-pending-initialization', r"/\ rfLog' = rfLocal /\ rfPending' = TRUE /\ rfUser' = FALSE",
         r"/\ rfLog' = rfLocal /\ rfPending' = FALSE /\ rfUser' = FALSE", 'RFCuts', r'PROVE\s+RFDrain'),
        ('ignore-applied-catalog', r"/\ rfBarrier /\ rfApplied = 0 /\ rfPlan' = rfTerm",
         r"/\ rfBarrier /\ TRUE /\ rfPlan' = rfTerm",
         'RFSafety', r'PROVE\s+RFDrain'),
        ('recover-foreign-root', r"rfLive' = (local = RFRoot /\ owned)",
         r"rfLive' = (TRUE /\ owned)", 'RFAuthority', r'RFRecover\(local, owned\) => RFInvariant'),
        ('recover-lost-store', r"rfLive' = (local = RFRoot /\ owned)",
         r"rfLive' = (local = RFRoot /\ TRUE)", 'RFAuthority', r'RFRecover\(local, owned\) => RFInvariant'),
        ('unfair-apply', r'/\ WF_rfVars(RFDrain)', r'/\ TRUE',
         'RFProgress', r'PROVE\s+RFFairSpec => RFReady ~> RFBarrierReady'),
    ]:
        control = dict(name=name, model=replace_once(model, before, after), property=prop, proof_pattern=pattern)
        control['config'] = ('SPECIFICATION RFFairSpec\n' if prop == 'RFProgress' else 'SPECIFICATION RFSpec\n')
        control['config'] += 'CONSTANTS RFRoots = {1, 2} RFRoot = 1 RFMaxTerm = 2\n'
        control['config'] += ('PROPERTY ' if prop in ('RFProgress', 'RFSafety') else 'INVARIANT ') + prop + '\n'
        controls.append(control)
    return controls
