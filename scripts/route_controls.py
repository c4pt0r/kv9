"""Isolated route-ownership faults shared by TLC and TLAPS checks."""
from ready_controls import replace_once


def mutations(model):
    controls = []
    for name, before, after, prop, pattern in [
        ('reuse-route-generation', "rtGeneration' = rtGeneration + 1", "rtGeneration' = rtGeneration",
         'RTInvariant', r'RTUpdate\(a\) => RTInvariant'),
        ('spawn-with-live-owner', r'RTSpawn == /\ ~rtClosed /\ rtWorkers = 0',
         r'RTSpawn == /\ ~rtClosed /\ rtWorkers >= 0', 'RTInvariant', r'PROVE\s+RTSpawn'),
        ('missing-route-observation-fairness', r'/\ WF_rtVars(RTObserve)', r'/\ TRUE',
         'RTProgress', r'PROVE\s+RTFairSpec => RTOwned ~> RTAligned'),
        ('accept-foreign-queue-generation', r"/\ rtQueue = rtSession /\ rtBatch' = rtQueue",
         r"/\ TRUE /\ rtBatch' = rtQueue",
         'RTInvariant', r'PROVE\s+RTSpawn'),
        ('enqueue-under-old-session', r"rtQueue = 0 /\ rtQueue' = rtGeneration", r"rtQueue = 0 /\ rtQueue' = rtSession",
         'RTEffects', r'PROVE\s+RTQueueCapture'),
        ('carry-batch-across-route-change', r'''rtPhase' = "Connect" /\ rtBatch' = 0''',
         r'''rtPhase' = "Connect" /\ rtBatch' = rtBatch''', 'RTInvariant', r'PROVE\s+RTSpawn'),
        ('missing-retransmission-fairness', r'/\ WF_rtVars(RTEnqueue)', r'/\ TRUE',
         'RTProgress', r'PROVE\s+RTFairSpec => RTClean ~> RTOffered'),
        ('retain-worker-after-close', "rtWorkers' = 0", "rtWorkers' = 1",
         'RTStopProgress', r'PROVE\s+RTStopping /\\ RTTerminate => RTStopped'),
    ]:
        spec = 'RTMCLiveSpec' if prop == 'RTProgress' else 'RTMCStopSpec' if prop == 'RTStopProgress' else 'RTMCSpec'
        config = f'SPECIFICATION {spec}\nCONSTANTS RTAddresses = {{1, 2}} RTInitial = 1 RTLimit = 3\n'
        config += ('PROPERTY ' if prop in ('RTProgress', 'RTStopProgress', 'RTEffects') else 'INVARIANT ') + prop + '\n'
        controls.append(dict(name=name, model=replace_once(model, before, after), property=prop,
                             proof_pattern=pattern, config=config))
    return controls
