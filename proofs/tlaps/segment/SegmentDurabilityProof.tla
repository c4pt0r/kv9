---------------------- MODULE SegmentDurabilityProof ----------------------
EXTENDS SegmentDurability, TLAPS, NaturalsInduction

THEOREM SGInvariantInit == SGInit => SGInvariant
BY SGLegalInputs, SMT DEF SGInit, SGInvariant, SGType, SGPrefix, SGOrder, SGSummary, SGAuthority, SGStage, SGAdvance, SGCanAppend, SGPinned, SGSlots, SGPhases

THEOREM SGStartStep == ASSUME SGInvariant PROVE \A f \in SGFrames : SGStart(f) => SGInvariant'
BY SGLegalInputs, SMT DEF SGStart, SGInvariant, SGType, SGPrefix, SGOrder, SGSummary, SGAuthority, SGStage, SGAdvance, SGCanAppend, SGPinned, SGSlots, SGPhases

THEOREM SGWriteStep == ASSUME SGInvariant, SGWrite PROVE SGInvariant'
<1>1. SGType' BY SGLegalInputs, SMT DEF SGWrite, SGInvariant, SGType, SGPrefix, SGOrder, SGSummary, SGAuthority, SGStage, SGAdvance, SGCanAppend, SGPinned, SGSlots, SGPhases
<1>2. SGPrefix' BY SGLegalInputs, SMT DEF SGWrite, SGInvariant, SGType, SGPrefix, SGOrder, SGSummary, SGAuthority, SGStage, SGAdvance, SGCanAppend, SGPinned, SGSlots, SGPhases
<1>3. SGOrder'
    <2>1. /\ sgWritten' = sgWritten + 1 /\ sgPosition'[sgWritten + 1] = sgPending
           /\ \A k \in 1..sgWritten : sgPosition'[k] = sgPosition[k]
        BY SGLegalInputs, SMT DEF SGWrite, SGInvariant, SGType, SGStage, SGSlots
    <2>2. /\ SGAdvance(SGPrevious, sgPending)
           /\ \A k \in 1..sgWritten : SGAdvance(sgPosition[k], sgPending)
        BY SMT DEF SGInvariant, SGStage, SGCanAppend, SGWrite
    <2>3. \A k \in 1..sgWritten' : SGAdvance(SGPrevious, sgPosition'[k])
        BY <2>1, <2>2, SMT DEF SGInvariant, SGType, SGOrder
    <2>4. \A i, j \in 1..sgWritten' : i < j => SGAdvance(sgPosition'[i], sgPosition'[j])
        BY <2>1, <2>2, SMT DEF SGInvariant, SGType, SGOrder
    <2> QED BY <2>3, <2>4 DEF SGOrder
<1>4. SGSummary' BY SGLegalInputs, SMT DEF SGWrite, SGInvariant, SGType, SGPrefix, SGOrder, SGSummary, SGAuthority, SGStage, SGAdvance, SGCanAppend, SGPinned, SGSlots, SGPhases
<1>5. SGAuthority' BY SGLegalInputs, SMT DEF SGWrite, SGInvariant, SGType, SGPrefix, SGOrder, SGSummary, SGAuthority, SGStage, SGAdvance, SGCanAppend, SGPinned, SGSlots, SGPhases
<1>6. SGStage' BY SGLegalInputs, SMT DEF SGWrite, SGInvariant, SGType, SGPrefix, SGOrder, SGSummary, SGAuthority, SGStage, SGAdvance, SGCanAppend, SGPinned, SGSlots, SGPhases
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6 DEF SGInvariant

THEOREM SGSyncStep == ASSUME SGInvariant PROVE SGSync => SGInvariant'
BY SGLegalInputs, SMT DEF SGSync, SGInvariant, SGType, SGPrefix, SGOrder, SGSummary, SGAuthority, SGStage, SGAdvance, SGCanAppend, SGPinned, SGSlots, SGPhases

THEOREM SGPublishStep == ASSUME SGInvariant PROVE SGPublish => SGInvariant'
BY SGLegalInputs, SMT DEF SGPublish, SGInvariant, SGType, SGPrefix, SGOrder, SGSummary, SGAuthority, SGStage, SGAdvance, SGCanAppend, SGPinned, SGSlots, SGPhases

THEOREM SGSealStartStep == ASSUME SGInvariant PROVE SGSealStart => SGInvariant'
BY SGLegalInputs, SMT DEF SGSealStart, SGInvariant, SGType, SGPrefix, SGOrder, SGSummary, SGAuthority, SGStage, SGAdvance, SGCanAppend, SGPinned, SGSlots, SGPhases

THEOREM SGSealSyncStep == ASSUME SGInvariant PROVE SGSealSync => SGInvariant'
BY SGLegalInputs, SMT DEF SGSealSync, SGInvariant, SGType, SGPrefix, SGOrder, SGSummary, SGAuthority, SGStage, SGAdvance, SGCanAppend, SGPinned, SGSlots, SGPhases

THEOREM SGFailStep == ASSUME SGInvariant PROVE SGFail => SGInvariant'
BY SGLegalInputs, SMT DEF SGFail, SGInvariant, SGType, SGPrefix, SGOrder, SGSummary, SGAuthority, SGStage, SGAdvance, SGCanAppend, SGPinned, SGSlots, SGPhases

THEOREM SGCrashStep == ASSUME SGInvariant PROVE \A d \in 0..SGMax, t \in BOOLEAN : SGCrash(d, t) => SGInvariant'
BY SGLegalInputs, SMT DEF SGCrash, SGInvariant, SGType, SGPrefix, SGOrder, SGSummary, SGAuthority, SGStage, SGAdvance, SGCanAppend, SGPinned, SGSlots, SGPhases

THEOREM SGOpenStep == ASSUME SGInvariant PROVE \A i \in SGIdentities, b \in BOOLEAN : SGOpen(i, b) => SGInvariant'
BY SGLegalInputs, SMT DEF SGOpen, SGInvariant, SGType, SGPrefix, SGOrder, SGSummary, SGAuthority, SGStage, SGAdvance, SGCanAppend, SGPinned, SGSlots, SGPhases

THEOREM SGValidateStep == ASSUME SGInvariant PROVE SGValidate => SGInvariant'
BY SGLegalInputs, SMT DEF SGValidate, SGInvariant, SGType, SGPrefix, SGOrder, SGSummary, SGAuthority, SGStage, SGAdvance, SGCanAppend, SGPinned, SGSlots, SGPhases

THEOREM SGRejectStep == ASSUME SGInvariant PROVE SGReject => SGInvariant'
BY SGLegalInputs, SMT DEF SGReject, SGInvariant, SGType, SGPrefix, SGOrder, SGSummary, SGAuthority, SGStage, SGAdvance, SGCanAppend, SGPinned, SGSlots, SGPhases

THEOREM SGRepairStep == ASSUME SGInvariant PROVE SGRepair => SGInvariant'
BY SGLegalInputs, SMT DEF SGRepair, SGInvariant, SGType, SGPrefix, SGOrder, SGSummary, SGAuthority, SGStage, SGAdvance, SGCanAppend, SGPinned, SGSlots, SGPhases

THEOREM SGRecoverySyncStep == ASSUME SGInvariant PROVE SGRecoverySync => SGInvariant'
BY SGLegalInputs, SMT DEF SGRecoverySync, SGInvariant, SGType, SGPrefix, SGOrder, SGSummary, SGAuthority, SGStage, SGAdvance, SGCanAppend, SGPinned, SGSlots, SGPhases

THEOREM SGRecoveryPublishStep == ASSUME SGInvariant PROVE SGRecoveryPublish => SGInvariant'
BY SGLegalInputs, SMT DEF SGRecoveryPublish, SGInvariant, SGType, SGPrefix, SGOrder, SGSummary, SGAuthority, SGStage, SGAdvance, SGCanAppend, SGPinned, SGSlots, SGPhases

THEOREM SGClosedReplayStep == ASSUME SGInvariant PROVE SGClosedReplay => SGInvariant'
BY SGLegalInputs, SMT DEF SGClosedReplay, SGInvariant, SGType, SGPrefix, SGOrder, SGSummary, SGAuthority, SGStage, SGAdvance, SGCanAppend, SGPinned, SGSlots, SGPhases

THEOREM SGInvariantStep == ASSUME SGInvariant, [SGNext]_sgVars PROVE SGInvariant'
BY SGStartStep, SGWriteStep, SGSyncStep, SGPublishStep, SGSealStartStep, SGSealSyncStep, SGFailStep, SGCrashStep, SGOpenStep, SGValidateStep, SGRejectStep, SGRepairStep, SGRecoverySyncStep, SGRecoveryPublishStep, SGClosedReplayStep, SGLegalInputs, SMT
   DEF SGNext, SGQuiesce, sgVars, SGInvariant, SGType, SGPrefix, SGOrder, SGSummary, SGAuthority, SGStage, SGAdvance, SGCanAppend, SGPinned, SGSlots, SGPhases

THEOREM SGInvariantAlways == SGSpec => []SGInvariant
BY SGInvariantInit, SGInvariantStep, PTL DEF SGSpec

THEOREM SGAcknowledgedAlways == SGSpec => []SGAcknowledged
<1>1. SGInvariant => SGAcknowledged BY SMT DEF SGInvariant, SGPrefix, SGAcknowledged
<1> QED BY SGInvariantAlways, <1>1, PTL

THEOREM SGBindingAlways == SGSpec => []SGBinding
<1>1. SGInvariant => SGBinding BY SMT DEF SGInvariant, SGPrefix, SGBinding
<1> QED BY SGInvariantAlways, <1>1, PTL

THEOREM SGUnpositionedPreservedAlways == SGSpec => []SGSummary
<1>1. SGInvariant => SGSummary BY SMT DEF SGInvariant, SGPrefix, SGSummary
<1> QED BY SGInvariantAlways, <1>1, PTL

THEOREM SGAcknowledgedStep == ASSUME SGInvariant, SGNext PROVE SGAckStep
BY SGLegalInputs, SMT DEF SGAckStep, SGNext, SGQuiesce, sgVars, SGStart, SGWrite, SGSync, SGPublish, SGSealStart, SGSealSync, SGFail, SGCrash, SGOpen, SGValidate, SGReject, SGRepair, SGRecoverySync, SGRecoveryPublish, SGClosedReplay, SGInvariant, SGType, SGPrefix, SGOrder, SGSummary, SGAuthority, SGStage, SGAdvance, SGCanAppend, SGPinned, SGSlots, SGPhases

THEOREM SGClosedStepProof == ASSUME SGInvariant, SGNext PROVE SGClosedStep
BY SGLegalInputs, SMT DEF SGClosedStep, SGNext, SGQuiesce, sgVars, SGStart, SGWrite, SGSync, SGPublish, SGSealStart, SGSealSync, SGFail, SGCrash, SGOpen, SGValidate, SGReject, SGRepair, SGRecoverySync, SGRecoveryPublish, SGClosedReplay, SGInvariant, SGType, SGPrefix, SGOrder, SGSummary, SGAuthority, SGStage, SGAdvance, SGCanAppend, SGPinned, SGSlots, SGPhases

THEOREM SGFencedStepProof == ASSUME SGInvariant, SGNext PROVE SGFencedStep
BY SGLegalInputs, SMT DEF SGFencedStep, SGNext, SGQuiesce, sgVars, SGStart, SGWrite, SGSync, SGPublish, SGSealStart, SGSealSync, SGFail, SGCrash, SGOpen, SGValidate, SGReject, SGRepair, SGRecoverySync, SGRecoveryPublish, SGClosedReplay, SGInvariant, SGType, SGPrefix, SGOrder, SGSummary, SGAuthority, SGStage, SGAdvance, SGCanAppend, SGPinned, SGSlots, SGPhases

THEOREM SGAcknowledgedStable == SGSpec => SGAckStableAlways
<1>1. SGInvariant /\ [SGNext]_sgVars => [SGAckStep]_sgVars
    BY SGAcknowledgedStep, SMT DEF SGAckStep, sgVars
<1> QED BY SGInvariantAlways, <1>1, PTL DEF SGSpec, SGAckStableAlways

THEOREM SGClosedPreserved == SGSpec => SGClosedAlways
<1>1. SGInvariant /\ [SGNext]_sgVars => [SGClosedStep]_sgVars
    BY SGClosedStepProof, SMT DEF SGClosedStep, sgVars
<1> QED BY SGInvariantAlways, <1>1, PTL DEF SGSpec, SGClosedAlways

THEOREM SGFencedPreserved == SGSpec => SGFencedAlways
<1>1. SGInvariant /\ [SGNext]_sgVars => [SGFencedStep]_sgVars
    BY SGFencedStepProof, SMT DEF SGFencedStep, sgVars
<1> QED BY SGInvariantAlways, <1>1, PTL DEF SGSpec, SGFencedAlways

THEOREM SGAdvanceTransitive ==
    \A a, c \in SGFrames \cup {0}, b \in SGPositioned :
        SGAdvance(a, b) /\ SGAdvance(b, c) => SGAdvance(a, c)
BY SGLegalInputs, SMT DEF SGAdvance

THEOREM SGLastGuardEquivalence == ASSUME SGInvariant
    PROVE \A l \in 0..sgWritten, f \in SGFrames : SGLatest(l) => (SGLastGuard(l, f) <=> SGCanAppend(f))
<1>1. \A l \in 0..sgWritten, f \in SGFrames :
          SGLatest(l) /\ SGCanAppend(f) => SGLastGuard(l, f)
    BY SMT DEF SGLatest, SGLastOf, SGCanAppend, SGLastGuard
<1>2. \A l \in 0..sgWritten, f \in SGFrames :
          SGLatest(l) /\ SGLastGuard(l, f) => SGAdvance(SGPrevious, f)
    BY SGAdvanceTransitive, SGLegalInputs, SMT DEF SGLatest, SGLastOf, SGLastGuard, SGInvariant, SGOrder, SGPrefix
<1>3. \A l \in 0..sgWritten, f \in SGFrames :
          SGLatest(l) /\ SGLastGuard(l, f) => \A k \in 1..sgWritten : SGAdvance(sgPosition[k], f)
    <2>1. SUFFICES ASSUME NEW l \in 0..sgWritten, NEW f \in SGFrames,
                         SGLatest(l), SGLastGuard(l, f), NEW k \in 1..sgWritten
                  PROVE SGAdvance(sgPosition[k], f)
        OBVIOUS
    <2>2. CASE l = 0
        BY <2>1, <2>2, SMT DEF SGLatest, SGLastOf, SGAdvance
    <2>3. CASE l > 0 /\ k > l
        BY <2>1, <2>3, SMT DEF SGLatest, SGLastOf, SGAdvance
    <2>4. CASE l > 0 /\ k = l
        BY <2>1, <2>4, SMT DEF SGLastGuard
    <2>5. CASE l > 0 /\ k < l
        <3>1. /\ sgPosition[k] \in SGFrames /\ sgPosition[l] \in SGPositioned
               /\ SGAdvance(sgPosition[k], sgPosition[l]) /\ SGAdvance(sgPosition[l], f)
            BY <2>1, <2>5, SMT DEF SGLatest, SGLastOf, SGLastGuard, SGInvariant, SGPrefix, SGOrder
        <3> QED BY <3>1, SGAdvanceTransitive, SMT
    <2> QED BY <2>2, <2>3, <2>4, <2>5, SMT
<1> QED BY <1>1, <1>2, <1>3, SMT DEF SGCanAppend

THEOREM SGSummaryFoldInitial == \A data \in [SGSlots -> SGFrames \cup {0}] :
    SGSummaryGood(0, 0, 0, FALSE, data)
BY SMT DEF SGSummaryGood, SGFirstOf, SGLastOf, SGPinned

THEOREM SGSummaryFoldStep ==
    ASSUME NEW n \in 0..SGMax, n < SGMax,
           NEW data \in [SGSlots -> SGFrames \cup {0}],
           NEW first \in 0..n, NEW last \in 0..n, NEW pin \in BOOLEAN,
           SGSummaryGood(n, first, last, pin, data)
    PROVE SGSummaryGood(n + 1, SGFoldFirst(first, n, data[n + 1]),
             SGFoldLast(last, n, data[n + 1]), SGFoldPin(pin, data[n + 1]), data)
<1>1. SGFirstOf(SGFoldFirst(first, n, data[n + 1]), n + 1, data)
    BY SGLegalInputs, SMT DEF SGSummaryGood, SGFirstOf, SGFoldFirst, SGSlots
<1>2. SGLastOf(SGFoldLast(last, n, data[n + 1]), n + 1, data)
    BY SGLegalInputs, SMT DEF SGSummaryGood, SGLastOf, SGFoldLast, SGSlots
<1>3. SGFoldPin(pin, data[n + 1]) = SGPinned(n + 1, data)
    BY SMT DEF SGSummaryGood, SGFoldPin, SGPinned
<1> QED BY <1>1, <1>2, <1>3 DEF SGSummaryGood

THEOREM SGSummaryFoldCorrect ==
    ASSUME NEW data, NEW sums, SGFoldTranscript(data, sums)
    PROVE \A n \in 0..SGMax :
        SGSummaryGood(n, sums[n].first, sums[n].last, sums[n].pin, data)
<1>1. DEFINE P(n) == n \in 0..SGMax =>
          SGSummaryGood(n, sums[n].first, sums[n].last, sums[n].pin, data)
<1>2. P(0)
    BY SGSummaryFoldInitial, SGLegalInputs, SMT DEF P, SGFoldTranscript
<1>3. ASSUME NEW n \in Nat, P(n) PROVE P(n + 1)
    <2>1. CASE n < SGMax
        <3>1. /\ n \in 0..SGMax /\ data \in [SGSlots -> SGFrames \cup {0}]
               /\ sums[n].first \in 0..n /\ sums[n].last \in 0..n /\ sums[n].pin \in BOOLEAN
               /\ SGSummaryGood(n, sums[n].first, sums[n].last, sums[n].pin, data)
            <4>1. n \in 0..SGMax
                BY <1>3, <2>1, SGLegalInputs, SMT
            <4>2. data \in [SGSlots -> SGFrames \cup {0}]
                BY SMT DEF SGFoldTranscript
            <4>3. SGSummaryGood(n, sums[n].first, sums[n].last, sums[n].pin, data)
                BY <1>3, <4>1 DEF P
            <4>4. sums[n].first \in 0..n /\ sums[n].last \in 0..n
                BY <4>3, SMT DEF SGSummaryGood, SGFirstOf, SGLastOf
            <4>5. sums[n].pin \in BOOLEAN
                BY <4>1, SMT DEF SGFoldTranscript
            <4> QED BY <4>1, <4>2, <4>3, <4>4, <4>5
        <3>2. SGSummaryGood(n + 1, SGFoldFirst(sums[n].first, n, data[n + 1]),
                   SGFoldLast(sums[n].last, n, data[n + 1]), SGFoldPin(sums[n].pin, data[n + 1]), data)
            BY <3>1, <2>1, SGSummaryFoldStep, SMT
        <3>3. sums[n + 1] = [first |-> SGFoldFirst(sums[n].first, n, data[n + 1]),
                  last |-> SGFoldLast(sums[n].last, n, data[n + 1]), pin |-> SGFoldPin(sums[n].pin, data[n + 1])]
            BY <3>1, <2>1, SMT DEF SGFoldTranscript
        <3> QED BY <3>2, <3>3, SMT DEF P
    <2>2. CASE n >= SGMax
        BY <1>3, <2>2, SGLegalInputs, SMT DEF P
    <2> QED BY <1>3, <2>1, <2>2, SGLegalInputs, SMT
<1>4. \A n \in Nat : P(n)
    BY <1>2, <1>3, NatInduction, Isa
<1> QED BY <1>4, SGLegalInputs, SMT DEF P

THEOREM SGFoldLawAlways == SGSpec => []SGFoldLaw
<1>1. SGInvariant => SGFoldLaw
    BY SGSummaryFoldStep, SGLegalInputs, SMT DEF SGInvariant, SGType, SGFoldLaw
<1> QED BY SGInvariantAlways, <1>1, PTL

THEOREM SGStableInvariant == SGFairSpec => []SGInvariant
<1>1. SGStableNext => SGNext BY SMT DEF SGStableNext, SGNext
<1>2. SGInvariant /\ [SGStableNext]_sgVars => SGInvariant'
    BY <1>1, SGInvariantStep, SMT
<1> QED BY <1>2, PTL DEF SGFairSpec

THEOREM SGWriteProgress == SGFairSpec => ((sgPhase = "write") ~> (sgPhase = "sync"))
<1>1. SGInvariant /\ (sgPhase = "write") /\ [SGStableNext]_sgVars =>
          SGInvariant' /\ ((sgPhase = "write")' \/ (sgPhase = "sync")')
    BY SGInvariantStep, SGLegalInputs, SMT DEF SGNext, SGStableNext, SGQuiesce, sgVars, SGWrite, SGSync, SGPublish, SGSealSync, SGValidate, SGReject, SGRepair, SGRecoverySync, SGRecoveryPublish, SGClosedReplay
<1>2. SGInvariant /\ (sgPhase = "write") /\ SGWrite => (sgPhase = "sync")'
    BY SMT DEF SGWrite
<1>3. SGInvariant /\ (sgPhase = "write") => ENABLED <<SGWrite>>_sgVars
    BY ExpandENABLED, SGLegalInputs, SMT DEF SGWrite, sgVars
<1> QED BY SGStableInvariant, <1>1, <1>2, <1>3, PTL DEF SGFairSpec

THEOREM SGSyncProgress == SGFairSpec => ((sgPhase = "sync") ~> (sgPhase = "publish"))
<1>1. SGInvariant /\ (sgPhase = "sync") /\ [SGStableNext]_sgVars =>
          SGInvariant' /\ ((sgPhase = "sync")' \/ (sgPhase = "publish")')
    BY SGInvariantStep, SGLegalInputs, SMT DEF SGNext, SGStableNext, SGQuiesce, sgVars, SGWrite, SGSync, SGPublish, SGSealSync, SGValidate, SGReject, SGRepair, SGRecoverySync, SGRecoveryPublish, SGClosedReplay
<1>2. SGInvariant /\ (sgPhase = "sync") /\ SGSync => (sgPhase = "publish")'
    BY SMT DEF SGSync
<1>3. SGInvariant /\ (sgPhase = "sync") => ENABLED <<SGSync>>_sgVars
    BY ExpandENABLED, SGLegalInputs, SMT DEF SGSync, sgVars
<1> QED BY SGStableInvariant, <1>1, <1>2, <1>3, PTL DEF SGFairSpec

THEOREM SGPublishProgress == SGFairSpec => ((sgPhase = "publish") ~> (sgPhase = "ready"))
<1>1. SGInvariant /\ (sgPhase = "publish") /\ [SGStableNext]_sgVars =>
          SGInvariant' /\ ((sgPhase = "publish")' \/ (sgPhase = "ready")')
    BY SGInvariantStep, SGLegalInputs, SMT DEF SGNext, SGStableNext, SGQuiesce, sgVars, SGWrite, SGSync, SGPublish, SGSealSync, SGValidate, SGReject, SGRepair, SGRecoverySync, SGRecoveryPublish, SGClosedReplay
<1>2. SGInvariant /\ (sgPhase = "publish") /\ SGPublish => (sgPhase = "ready")'
    BY SMT DEF SGPublish
<1>3. SGInvariant /\ (sgPhase = "publish") => ENABLED <<SGPublish>>_sgVars
    BY ExpandENABLED, SGLegalInputs, SMT DEF SGPublish, sgVars
<1> QED BY SGStableInvariant, <1>1, <1>2, <1>3, PTL DEF SGFairSpec

THEOREM SGSealProgress == SGFairSpec => ((sgPhase = "seal") ~> (sgPhase = "closed"))
<1>1. SGInvariant /\ (sgPhase = "seal") /\ [SGStableNext]_sgVars =>
          SGInvariant' /\ ((sgPhase = "seal")' \/ (sgPhase = "closed")')
    BY SGInvariantStep, SGLegalInputs, SMT DEF SGNext, SGStableNext, SGQuiesce, sgVars, SGWrite, SGSync, SGPublish, SGSealSync, SGValidate, SGReject, SGRepair, SGRecoverySync, SGRecoveryPublish, SGClosedReplay
<1>2. SGInvariant /\ (sgPhase = "seal") /\ SGSealSync => (sgPhase = "closed")'
    BY SMT DEF SGSealSync
<1>3. SGInvariant /\ (sgPhase = "seal") => ENABLED <<SGSealSync>>_sgVars
    BY ExpandENABLED, SGLegalInputs, SMT DEF SGSealSync, sgVars
<1> QED BY SGStableInvariant, <1>1, <1>2, <1>3, PTL DEF SGFairSpec

THEOREM SGValidateProgress == SGFairSpec => ((sgPhase = "scan" /\ sgIdentity = SGExpected /\ ~sgBad) ~> (sgPhase = "validated"))
<1>1. SGInvariant /\ (sgPhase = "scan" /\ sgIdentity = SGExpected /\ ~sgBad) /\ [SGStableNext]_sgVars =>
          SGInvariant' /\ ((sgPhase = "scan" /\ sgIdentity = SGExpected /\ ~sgBad)' \/ (sgPhase = "validated")')
    BY SGInvariantStep, SGLegalInputs, SMT DEF SGNext, SGStableNext, SGQuiesce, sgVars, SGWrite, SGSync, SGPublish, SGSealSync, SGValidate, SGReject, SGRepair, SGRecoverySync, SGRecoveryPublish, SGClosedReplay
<1>2. SGInvariant /\ (sgPhase = "scan" /\ sgIdentity = SGExpected /\ ~sgBad) /\ SGValidate => (sgPhase = "validated")'
    BY SMT DEF SGValidate
<1>3. SGInvariant /\ (sgPhase = "scan" /\ sgIdentity = SGExpected /\ ~sgBad) => ENABLED <<SGValidate>>_sgVars
    BY ExpandENABLED, SGLegalInputs, SMT DEF SGValidate, sgVars
<1> QED BY SGStableInvariant, <1>1, <1>2, <1>3, PTL DEF SGFairSpec

THEOREM SGRejectProgress == SGFairSpec => ((sgPhase = "scan" /\ (sgIdentity # SGExpected \/ sgBad)) ~> (sgPhase = "rejected"))
<1>1. SGInvariant /\ (sgPhase = "scan" /\ (sgIdentity # SGExpected \/ sgBad)) /\ [SGStableNext]_sgVars =>
          SGInvariant' /\ ((sgPhase = "scan" /\ (sgIdentity # SGExpected \/ sgBad))' \/ (sgPhase = "rejected")')
    BY SGInvariantStep, SGLegalInputs, SMT DEF SGNext, SGStableNext, SGQuiesce, sgVars, SGWrite, SGSync, SGPublish, SGSealSync, SGValidate, SGReject, SGRepair, SGRecoverySync, SGRecoveryPublish, SGClosedReplay
<1>2. SGInvariant /\ (sgPhase = "scan" /\ (sgIdentity # SGExpected \/ sgBad)) /\ SGReject => (sgPhase = "rejected")'
    BY SMT DEF SGReject
<1>3. SGInvariant /\ (sgPhase = "scan" /\ (sgIdentity # SGExpected \/ sgBad)) => ENABLED <<SGReject>>_sgVars
    BY ExpandENABLED, SGLegalInputs, SMT DEF SGReject, sgVars
<1> QED BY SGStableInvariant, <1>1, <1>2, <1>3, PTL DEF SGFairSpec

THEOREM SGRepairProgress == SGFairSpec => ((sgPhase = "validated" /\ ~sgClosed) ~> (sgPhase = "recover_sync"))
<1>1. SGInvariant /\ (sgPhase = "validated" /\ ~sgClosed) /\ [SGStableNext]_sgVars =>
          SGInvariant' /\ ((sgPhase = "validated" /\ ~sgClosed)' \/ (sgPhase = "recover_sync")')
    BY SGInvariantStep, SGLegalInputs, SMT DEF SGNext, SGStableNext, SGQuiesce, sgVars, SGWrite, SGSync, SGPublish, SGSealSync, SGValidate, SGReject, SGRepair, SGRecoverySync, SGRecoveryPublish, SGClosedReplay
<1>2. SGInvariant /\ (sgPhase = "validated" /\ ~sgClosed) /\ SGRepair => (sgPhase = "recover_sync")'
    BY SMT DEF SGRepair
<1>3. SGInvariant /\ (sgPhase = "validated" /\ ~sgClosed) => ENABLED <<SGRepair>>_sgVars
    BY ExpandENABLED, SGLegalInputs, SMT DEF SGRepair, sgVars
<1> QED BY SGStableInvariant, <1>1, <1>2, <1>3, PTL DEF SGFairSpec

THEOREM SGClosedReplayProgress == SGFairSpec => ((sgPhase = "validated" /\ sgClosed) ~> (sgPhase = "closed"))
<1>1. SGInvariant /\ (sgPhase = "validated" /\ sgClosed) /\ [SGStableNext]_sgVars =>
          SGInvariant' /\ ((sgPhase = "validated" /\ sgClosed)' \/ (sgPhase = "closed")')
    BY SGInvariantStep, SGLegalInputs, SMT DEF SGNext, SGStableNext, SGQuiesce, sgVars, SGWrite, SGSync, SGPublish, SGSealSync, SGValidate, SGReject, SGRepair, SGRecoverySync, SGRecoveryPublish, SGClosedReplay
<1>2. SGInvariant /\ (sgPhase = "validated" /\ sgClosed) /\ SGClosedReplay => (sgPhase = "closed")'
    BY SMT DEF SGClosedReplay
<1>3. SGInvariant /\ (sgPhase = "validated" /\ sgClosed) => ENABLED <<SGClosedReplay>>_sgVars
    BY ExpandENABLED, SGLegalInputs, SMT DEF SGClosedReplay, sgVars
<1> QED BY SGStableInvariant, <1>1, <1>2, <1>3, PTL DEF SGFairSpec

THEOREM SGRecoverySyncProgress == SGFairSpec => ((sgPhase = "recover_sync") ~> (sgPhase = "recover_publish"))
<1>1. SGInvariant /\ (sgPhase = "recover_sync") /\ [SGStableNext]_sgVars =>
          SGInvariant' /\ ((sgPhase = "recover_sync")' \/ (sgPhase = "recover_publish")')
    BY SGInvariantStep, SGLegalInputs, SMT DEF SGNext, SGStableNext, SGQuiesce, sgVars, SGWrite, SGSync, SGPublish, SGSealSync, SGValidate, SGReject, SGRepair, SGRecoverySync, SGRecoveryPublish, SGClosedReplay
<1>2. SGInvariant /\ (sgPhase = "recover_sync") /\ SGRecoverySync => (sgPhase = "recover_publish")'
    BY SMT DEF SGRecoverySync
<1>3. SGInvariant /\ (sgPhase = "recover_sync") => ENABLED <<SGRecoverySync>>_sgVars
    BY ExpandENABLED, SGLegalInputs, SMT DEF SGRecoverySync, sgVars
<1> QED BY SGStableInvariant, <1>1, <1>2, <1>3, PTL DEF SGFairSpec

THEOREM SGRecoveryPublishProgress == SGFairSpec => ((sgPhase = "recover_publish") ~> (sgPhase = "ready"))
<1>1. SGInvariant /\ (sgPhase = "recover_publish") /\ [SGStableNext]_sgVars =>
          SGInvariant' /\ ((sgPhase = "recover_publish")' \/ (sgPhase = "ready")')
    BY SGInvariantStep, SGLegalInputs, SMT DEF SGNext, SGStableNext, SGQuiesce, sgVars, SGWrite, SGSync, SGPublish, SGSealSync, SGValidate, SGReject, SGRepair, SGRecoverySync, SGRecoveryPublish, SGClosedReplay
<1>2. SGInvariant /\ (sgPhase = "recover_publish") /\ SGRecoveryPublish => (sgPhase = "ready")'
    BY SMT DEF SGRecoveryPublish
<1>3. SGInvariant /\ (sgPhase = "recover_publish") => ENABLED <<SGRecoveryPublish>>_sgVars
    BY ExpandENABLED, SGLegalInputs, SMT DEF SGRecoveryPublish, sgVars
<1> QED BY SGStableInvariant, <1>1, <1>2, <1>3, PTL DEF SGFairSpec

THEOREM SGProgressProof == SGFairSpec => SGProgress
<1>1. SGInFlight <=> (sgPhase = "write" \/ sgPhase = "sync" \/ sgPhase = "publish"
           \/ sgPhase = "seal" \/ sgPhase = "scan" \/ sgPhase = "validated"
           \/ sgPhase = "recover_sync" \/ sgPhase = "recover_publish")
    BY SMT DEF SGInFlight
<1>2. (sgPhase = "ready" \/ sgPhase = "closed" \/ sgPhase = "rejected") => SGSettled
    BY SMT DEF SGSettled
<1>3. (sgPhase = "scan") <=> ((sgPhase = "scan" /\ sgIdentity = SGExpected /\ ~sgBad)
            \/ (sgPhase = "scan" /\ (sgIdentity # SGExpected \/ sgBad)))
    BY SMT
<1>4. (sgPhase = "validated") <=> ((sgPhase = "validated" /\ sgClosed)
            \/ (sgPhase = "validated" /\ ~sgClosed))
    BY SMT
<1> QED BY <1>1, <1>2, <1>3, <1>4, SGWriteProgress, SGSyncProgress,
    SGPublishProgress, SGSealProgress, SGValidateProgress, SGRejectProgress,
    SGRepairProgress, SGClosedReplayProgress, SGRecoverySyncProgress,
    SGRecoveryPublishProgress, SGStableInvariant, PTL DEF SGProgress

=============================================================================
