--------------------------- MODULE RawGroupProof ---------------------------
EXTENDS RawGroup, RawMutationProof, TLAPS, NaturalsInduction

THEOREM RGInputBounds == RGItems \in Nat \ {0} /\ RGMaxCount \in Nat \ {0} /\ RGMaxBytes \in Nat \ {0}
BY RGLegalInputs

THEOREM RGEntryType == ASSUME NEW i \in RGEntries
    PROVE RGWeight[i] \in Nat /\ RGPosition[i] \in RGPositions /\ RGKind[i] \in RGKinds /\ RGRegion[i] \in Nat
BY RGLegalInputs, SMT

THEOREM RGInitialTypes == 1 \in RGEntries /\ RGWeight[1] \in Nat /\ RGPosition[RGItems] \in RGPositions
BY RGLegalInputs, SMT DEF RGEntries

THEOREM RGIntervalNext == ASSUME NEW n \in 0..RGItems, n < RGItems PROVE n + 1 \in RGEntries
BY RGInputBounds, SMT DEF RGEntries

THEOREM RGEffectiveType == ASSUME NEW i \in RGEntries, NEW state \in RMStates
    PROVE RGEffective(i, state) \in Seq(RMMutations) /\ RGVerdict(i, state) \in {"ok", "stale"}
BY RGLegalInputs, SMT DEF RGEffective, RGVerdict, RGEntries

THEOREM RGEffectiveRaw == ASSUME NEW i \in RGEntries, NEW state \in RMStates, RGEligible(i)
    PROVE RMRawBatch(RGEffective(i, state))
BY RMRawBatchEmpty, SMT DEF RGEffective, RGEligible

THEOREM RGVerdictSameSystem == ASSUME NEW i \in RGEntries, RGEligible(i),
    NEW before \in RMStates, NEW after \in RMStates,
    \A key \in RMSystemKeys : before[key] = after[key]
    PROVE RGVerdict(i, before) = RGVerdict(i, after) /\ RGEffective(i, before) = RGEffective(i, after)
BY RGLegalInputs, RMLegalInputs, SMT DEF RGVerdict, RGEffective, RGEligible, RGEntries, RMStates

THEOREM RGSequenceDefinition == FiniteNatInductiveDefConclusion(RGSequence, RGInitial,
    LAMBDA value, i : RMApply(value, RGEffective(i, value)), 0, RGItems)
<1> DEFINE Step(value, i) == RMApply(value, RGEffective(i, value))
<1> HIDE DEF Step
<1>0. SUFFICES FiniteNatInductiveDefConclusion(RGSequence, RGInitial, Step, 0, RGItems)
    BY DEF Step
<1>1. FiniteNatInductiveDefHypothesis(RGSequence, RGInitial, Step, 0, RGItems)
    BY DEF RGSequence, FiniteNatInductiveDefHypothesis, Step
<1>2. RGItems \in Nat /\ 0 \in Nat BY RGLegalInputs, SMT
<1> QED BY <1>1, <1>2, FiniteNatInductiveDef

THEOREM RGSequenceRecurrence ==
    /\ DOMAIN RGSequence = 0..RGItems
    /\ RGSequence = [i \in 0..RGItems |-> RGSequence[i]]
    /\ RGSequence[0] = RGInitial
    /\ \A i \in RGEntries : RGSequence[i] = RMApply(RGSequence[i - 1], RGEffective(i, RGSequence[i - 1]))
BY RGSequenceDefinition, RGLegalInputs, SMT DEF FiniteNatInductiveDefConclusion, RGEntries

THEOREM RGSequenceType == RGSequence \in [0..RGItems -> RMStates]
<1> DEFINE P(n) == n <= RGItems => RGSequence[n] \in RMStates
<1> HIDE DEF P
<1>1. P(0) BY RGLegalInputs, RGSequenceRecurrence, SMT DEF P
<1>2. ASSUME NEW n \in Nat, P(n) PROVE P(n + 1)
    <2>1. CASE n + 1 <= RGItems
        <3>1. n + 1 \in RGEntries /\ RGSequence[n] \in RMStates
            BY <1>2, <2>1, RGLegalInputs, SMT DEF P, RGEntries
        <3> QED BY <3>1, RGSequenceRecurrence, RGEffectiveType, RMApplyType, SMT DEF P
    <2>2. CASE ~(n + 1 <= RGItems) BY <2>2 DEF P
    <2> QED BY <2>1, <2>2, SMT
<1>3. \A n \in Nat : P(n) BY <1>1, <1>2, NatInduction, Isa
<1> QED BY <1>3, RGSequenceRecurrence, RGLegalInputs, SMT DEF P

THEOREM RGStateNumbers == ASSUME RGType PROVE
    rgCount \in Nat /\ rgPrepared \in Nat /\ rgProcessed \in Nat /\ rgBytes \in Nat /\
    rgCount <= RGItems /\ rgPrepared <= rgCount /\ rgProcessed <= RGItems
BY RGInputBounds, SMT DEF RGType

THEOREM RGIntervalExtend == ASSUME NEW n \in Nat PROVE 1..(n + 1) = (1..n) \cup {n + 1}
BY SMT

THEOREM RGIntervalPredecessor == ASSUME NEW n \in Nat, NEW i \in 1..n
    PROVE i \in Nat /\ i - 1 \in 0..n /\ i <= n /\ i - 1 < n + 1
BY SMT

THEOREM RGPlanVerdictTypes == ASSUME RGInvariant, RGPlan PROVE
    RGVerdict(rgPrepared + 1, RGInitial) \in RGOutcomes /\
    RGVerdict(rgPrepared + 1, rgReference) \in RGOutcomes
<1>1. rgPrepared + 1 \in RGEntries /\ rgReference \in RMStates /\ RGInitial \in RMStates
    BY RGStateNumbers, RGInputBounds, RGLegalInputs, SMT DEF RGPlan, RGCanPlan, RGInvariant, RGType, RGEntries
<1> QED BY <1>1, RGEffectiveType, SMT DEF RGOutcomes

THEOREM RGLedgerExtension ==
    ASSUME NEW bound \in Nat, NEW n \in 1..bound, n < bound,
        NEW ledger \in [0..bound -> Nat], NEW weight \in [1..bound -> Nat],
        ledger[0] = 0, \A i \in 1..n : ledger[i] = ledger[i - 1] + weight[i]
    PROVE LET next == [ledger EXCEPT ![n + 1] = ledger[n] + weight[n + 1]] IN
        /\ next \in [0..bound -> Nat] /\ next[0] = 0
        /\ next[n + 1] = ledger[n] + weight[n + 1]
        /\ \A i \in 1..(n + 1) : next[i] = next[i - 1] + weight[i]
<1> DEFINE next == [ledger EXCEPT ![n + 1] = ledger[n] + weight[n + 1]]
<1>0. n \in Nat /\ n + 1 \in 1..bound /\ n + 1 \in 0..bound /\ ledger[n] \in Nat /\ weight[n + 1] \in Nat BY SMT
<1>1. next \in [0..bound -> Nat] /\ next[0] = 0 /\ next[n + 1] = ledger[n] + weight[n + 1]
    BY <1>0, SMT DEF next
<1>2. \A i \in 1..(n + 1) : next[i] = next[i - 1] + weight[i]
    <2>1. SUFFICES ASSUME NEW i \in 1..(n + 1) PROVE next[i] = next[i - 1] + weight[i] OBVIOUS
    <2>2. CASE i = n + 1
        BY <2>1, <2>2, <1>0, SMT DEF next
    <2>3. CASE i # n + 1
        <3>1. i \in 1..n /\ i - 1 \in 0..bound /\ i - 1 # n + 1
            BY <2>1, <2>3, <1>0, RGIntervalExtend, RGIntervalPredecessor, SMT
        <3> QED BY <3>1, <2>3, SMT DEF next
    <2> QED BY <2>2, <2>3, SMT
<1> QED BY <1>1, <1>2 DEF next

THEOREM RGGrowLedger == ASSUME RGInvariant, RGGrow PROVE
    /\ rgByteLedger' \in [0..RGItems -> Nat] /\ rgByteLedger'[0] = 0
    /\ rgByteLedger'[rgCount + 1] = rgBytes + RGWeight[rgCount + 1]
    /\ \A i \in 1..(rgCount + 1) : rgByteLedger'[i] = rgByteLedger'[i - 1] + RGWeight[i]
<1>1. RGItems \in Nat /\ rgCount \in 1..RGItems /\ rgCount < RGItems /\
    rgByteLedger \in [0..RGItems -> Nat] /\ RGWeight \in [1..RGItems -> Nat] /\
    rgByteLedger[0] = 0 /\ rgBytes = rgByteLedger[rgCount] /\
    (\A i \in 1..rgCount : rgByteLedger[i] = rgByteLedger[i - 1] + RGWeight[i])
    BY RGLegalInputs, RGStateNumbers, SMT DEF RGGrow, RGCanGrow, RGInvariant, RGType, RGSelection, RGEntries
<1>2. rgByteLedger' = [rgByteLedger EXCEPT ![rgCount + 1] = rgByteLedger[rgCount] + RGWeight[rgCount + 1]]
    BY <1>1, SMT DEF RGGrow
<1> QED BY ONLY <1>1, <1>2, RGLedgerExtension, SMT

THEOREM RGInvariantInit == RGInit => RGInvariant
<1>1. RGInit => RGType
    <2>1. RGInit => (rgPhase \in RGPhases)
        BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGInit, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
    <2>2. RGInit => (rgCount \in 1..RGItems)
        BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGInit, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
    <2>3. RGInit => (rgBytes \in Nat)
        BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGInit, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
    <2>4. RGInit => (rgByteLedger \in [0..RGItems -> Nat])
        BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGInit, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
    <2>5. RGInit => (rgPrepared \in 0..rgCount)
        BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGInit, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
    <2>6. RGInit => (rgPrevious \in RGPositions)
        BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGInit, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
    <2>7. RGInit => (rgBatch \in Seq(RMMutations))
        BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGInit, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
    <2>8. RGInit => (rgReference \in RMStates)
        BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGInit, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
    <2>9. RGInit => (rgStaged \in [RGEntries -> RGOutcomes])
        BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGInit, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
    <2>10. RGInit => (rgSequential \in [RGEntries -> RGOutcomes])
        BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGInit, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
    <2>11. RGInit => (rgHasEffect \in BOOLEAN)
        BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGInit, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
    <2>12. RGInit => (rgImage \in RMStates)
        BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGInit, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
    <2>13. RGInit => (rgImagePosition \in RGPositions)
        BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGInit, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
    <2>14. RGInit => (rgEngineOk \in BOOLEAN)
        BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGInit, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
    <2>15. RGInit => (rgSmPublication \in RGPositions)
        BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGInit, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
    <2>16. RGInit => (rgReceipts \in [RGEntries -> [term : Nat, index : Nat, outcome : RGOutcomes, region : Nat]])
        BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGInit, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
    <2>17. RGInit => (rgProcessed \in 0..RGItems)
        BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGInit, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
    <2>18. RGInit => (rgDriver \in RGPositions)
        BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGInit, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
    <2>19. RGInit => (rgFailure \in RGGroupFailures \cup {"none", "later"})
        BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGInit, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
    <2> QED BY <2>1, <2>2, <2>3, <2>4, <2>5, <2>6, <2>7, <2>8, <2>9, <2>10, <2>11, <2>12, <2>13, <2>14, <2>15, <2>16, <2>17, <2>18, <2>19 DEF RGType
<1>2. RGInit => RGSelection
    <2>1. RGInit => rgPhase = "choose" /\ rgCount = 1 /\ rgBytes = RGWeight[1] /\
        rgByteLedger[0] = 0 /\ rgByteLedger[1] = RGWeight[1]
        BY RGInitialTypes, RGInputBounds, SMT DEF RGInit
    <2> QED BY ONLY <2>1, RGInitialTypes, RGInputBounds, SMT DEF RGSelection
<1>3. RGInit => RGPlanIdentity
    BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, RGSequenceRecurrence, SMT DEF RGInit, RGPlanIdentity, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
<1>4. RGInit => RGComposition
    BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, RMApplyEmpty, RMRawBatchEmpty, RGSequenceRecurrence, SMT DEF RGInit, RGComposition, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
<1>5. RGInit => RGSystemUnchanged
    BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGInit, RGSystemUnchanged, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
<1>6. RGInit => RGPhaseOrder
    BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGInit, RGPhaseOrder, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
<1>7. RGInit => RGEngineBinding
    BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGInit, RGEngineBinding, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
<1>8. RGInit => RGReceiptsExact
    BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGInit, RGReceiptsExact, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
<1>9. RGInit => RGPublication
    BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGInit, RGPublication, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
<1>10. RGInit => RGContiguity
    BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGInit, RGContiguity, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
<1>11. RGInit => RGDriverAuthority
    BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGInit, RGDriverAuthority, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
<1>12. RGInit => RGFenced
    BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGInit, RGFenced, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12 DEF RGInvariant

THEOREM RGGrowStep == ASSUME RGInvariant, RGGrow PROVE RGInvariant'
<1>1. RGType'
    <2>1. (rgPhase \in RGPhases)'
        BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGGrow, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases, RGInvariant, RGCanGrow, RGCanPlan, RGPhaseOrder, RGPublication
    <2>2. (rgCount \in 1..RGItems)'
        BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGGrow, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases, RGInvariant, RGCanGrow, RGCanPlan, RGPhaseOrder, RGPublication
    <2>3. (rgBytes \in Nat)'
        BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGGrow, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases, RGInvariant, RGCanGrow, RGCanPlan, RGPhaseOrder, RGPublication
    <2>4. (rgByteLedger \in [0..RGItems -> Nat])'
        BY RGGrowLedger
    <2>5. (rgPrepared \in 0..rgCount)'
        BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGGrow, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases, RGInvariant, RGCanGrow, RGCanPlan, RGPhaseOrder, RGPublication
    <2>6. (rgPrevious \in RGPositions)'
        BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGGrow, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases, RGInvariant, RGCanGrow, RGCanPlan, RGPhaseOrder, RGPublication
    <2>7. (rgBatch \in Seq(RMMutations))'
        BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGGrow, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases, RGInvariant, RGCanGrow, RGCanPlan, RGPhaseOrder, RGPublication
    <2>8. (rgReference \in RMStates)'
        BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGGrow, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases, RGInvariant, RGCanGrow, RGCanPlan, RGPhaseOrder, RGPublication
    <2>9. (rgStaged \in [RGEntries -> RGOutcomes])'
        BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGGrow, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases, RGInvariant, RGCanGrow, RGCanPlan, RGPhaseOrder, RGPublication
    <2>10. (rgSequential \in [RGEntries -> RGOutcomes])'
        BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGGrow, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases, RGInvariant, RGCanGrow, RGCanPlan, RGPhaseOrder, RGPublication
    <2>11. (rgHasEffect \in BOOLEAN)'
        BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGGrow, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases, RGInvariant, RGCanGrow, RGCanPlan, RGPhaseOrder, RGPublication
    <2>12. (rgImage \in RMStates)'
        BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGGrow, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases, RGInvariant, RGCanGrow, RGCanPlan, RGPhaseOrder, RGPublication
    <2>13. (rgImagePosition \in RGPositions)'
        BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGGrow, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases, RGInvariant, RGCanGrow, RGCanPlan, RGPhaseOrder, RGPublication
    <2>14. (rgEngineOk \in BOOLEAN)'
        BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGGrow, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases, RGInvariant, RGCanGrow, RGCanPlan, RGPhaseOrder, RGPublication
    <2>15. (rgSmPublication \in RGPositions)'
        BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGGrow, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases, RGInvariant, RGCanGrow, RGCanPlan, RGPhaseOrder, RGPublication
    <2>16. (rgReceipts \in [RGEntries -> [term : Nat, index : Nat, outcome : RGOutcomes, region : Nat]])'
        BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGGrow, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases, RGInvariant, RGCanGrow, RGCanPlan, RGPhaseOrder, RGPublication
    <2>17. (rgProcessed \in 0..RGItems)'
        BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGGrow, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases, RGInvariant, RGCanGrow, RGCanPlan, RGPhaseOrder, RGPublication
    <2>18. (rgDriver \in RGPositions)'
        BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGGrow, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases, RGInvariant, RGCanGrow, RGCanPlan, RGPhaseOrder, RGPublication
    <2>19. (rgFailure \in RGGroupFailures \cup {"none", "later"})'
        BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGGrow, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases, RGInvariant, RGCanGrow, RGCanPlan, RGPhaseOrder, RGPublication
    <2> QED BY <2>1, <2>2, <2>3, <2>4, <2>5, <2>6, <2>7, <2>8, <2>9, <2>10, <2>11, <2>12, <2>13, <2>14, <2>15, <2>16, <2>17, <2>18, <2>19 DEF RGType
<1>2. RGSelection'
    <2>1. rgCount' = rgCount + 1 /\ rgCount + 1 \in RGEntries /\
        rgBytes' = rgBytes + RGWeight[rgCount + 1] /\ rgPhase' = "choose"
        BY RGInputBounds, RGStateNumbers, RGIntervalNext, SMT DEF RGGrow, RGCanGrow, RGInvariant, RGType, RGEntries
    <2>2. rgCount' <= RGMaxCount /\ rgBytes' <= RGMaxBytes
        BY <2>1, RGEntryType, RGStateNumbers, RGInputBounds, SMT DEF RGGrow, RGCanGrow, RGInvariant, RGType, RGEntries
    <2>3. \A i \in 1..rgCount' : RGKind[i] \in {"put", "write", "fenced"} /\ RMRawBatch(RGOps[i]) /\ (RGKind[i] = "fenced" => RGOptIn)
        <3>1. SUFFICES ASSUME NEW i \in 1..rgCount' PROVE RGKind[i] \in {"put", "write", "fenced"} /\ RMRawBatch(RGOps[i]) /\ (RGKind[i] = "fenced" => RGOptIn) OBVIOUS
        <3>2. CASE i = rgCount + 1 \/ rgCount = 1
            BY <3>1, <3>2, <2>1, RGStateNumbers, RGIntervalExtend, SMT DEF RGGrow, RGCanGrow, RGEligible, RGInvariant, RGType
        <3>3. CASE i # rgCount + 1 /\ rgCount # 1
            <4>1. i \in 1..rgCount /\ rgCount > 1
                BY <3>1, <3>3, <2>1, RGStateNumbers, RGIntervalExtend, SMT DEF RGInvariant
            <4> QED BY <4>1, SMT DEF RGInvariant, RGSelection
        <3> QED BY <3>2, <3>3, SMT
    <2> QED BY ONLY <2>1, <2>2, <2>3, RGGrowLedger, RGInvariant, RGGrow, SMT DEF RGSelection
<1>3. RGPlanIdentity'
    BY SMT DEF RGGrow, RGInvariant, RGPlanIdentity
<1>4. RGComposition'
    BY SMT DEF RGGrow, RGInvariant, RGComposition
<1>5. RGSystemUnchanged'
    BY SMT DEF RGGrow, RGInvariant, RGSystemUnchanged
<1>6. RGPhaseOrder'
    BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGGrow, RGCanGrow, RGCanPlan, RGFailureAt, RGEligible, RGAdvances, RGInvariant, RGPhaseOrder, RGType, RGPublication, RGFenced, RGSelection, RGComposition, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
<1>7. RGEngineBinding'
    BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGGrow, RGCanGrow, RGCanPlan, RGFailureAt, RGEligible, RGAdvances, RGInvariant, RGEngineBinding, RGType, RGPhaseOrder, RGPublication, RGFenced, RGSelection, RGComposition, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
<1>8. RGReceiptsExact'
    BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGGrow, RGCanGrow, RGCanPlan, RGFailureAt, RGEligible, RGAdvances, RGInvariant, RGReceiptsExact, RGType, RGPhaseOrder, RGPublication, RGFenced, RGSelection, RGComposition, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
<1>9. RGPublication'
    BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGGrow, RGCanGrow, RGCanPlan, RGFailureAt, RGEligible, RGAdvances, RGInvariant, RGPublication, RGType, RGPhaseOrder, RGFenced, RGSelection, RGComposition, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
<1>10. RGContiguity'
    BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGGrow, RGCanGrow, RGCanPlan, RGFailureAt, RGEligible, RGAdvances, RGInvariant, RGContiguity, RGType, RGPhaseOrder, RGPublication, RGFenced, RGSelection, RGComposition, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
<1>11. RGDriverAuthority'
    BY SMT DEF RGGrow, RGInvariant, RGDriverAuthority
<1>12. RGFenced'
    BY SMT DEF RGGrow, RGInvariant, RGFenced
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12 DEF RGInvariant

THEOREM RGChooseDoneStep == ASSUME RGInvariant, RGChooseDone PROVE RGInvariant'
<1>1. RGType'
    BY SMT DEF RGChooseDone, RGInvariant, RGType, RGPhases
<1>2. RGSelection'
    BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGChooseDone, RGCanGrow, RGCanPlan, RGFailureAt, RGEligible, RGAdvances, RGInvariant, RGSelection, RGType, RGPhaseOrder, RGPublication, RGFenced, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
<1>3. RGPlanIdentity'
    BY SMT DEF RGChooseDone, RGInvariant, RGPlanIdentity
<1>4. RGComposition'
    BY SMT DEF RGChooseDone, RGInvariant, RGComposition
<1>5. RGSystemUnchanged'
    BY SMT DEF RGChooseDone, RGInvariant, RGSystemUnchanged
<1>6. RGPhaseOrder'
    BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGChooseDone, RGCanGrow, RGCanPlan, RGFailureAt, RGEligible, RGAdvances, RGInvariant, RGPhaseOrder, RGType, RGPublication, RGFenced, RGSelection, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
<1>7. RGEngineBinding'
    BY SMT DEF RGChooseDone, RGInvariant, RGEngineBinding
<1>8. RGReceiptsExact'
    BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGChooseDone, RGCanGrow, RGCanPlan, RGFailureAt, RGEligible, RGAdvances, RGInvariant, RGReceiptsExact, RGType, RGPhaseOrder, RGPublication, RGFenced, RGSelection, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
<1>9. RGPublication'
    BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGChooseDone, RGCanGrow, RGCanPlan, RGFailureAt, RGEligible, RGAdvances, RGInvariant, RGPublication, RGType, RGPhaseOrder, RGFenced, RGSelection, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
<1>10. RGContiguity'
    BY SMT DEF RGChooseDone, RGInvariant, RGContiguity
<1>11. RGDriverAuthority'
    BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGChooseDone, RGCanGrow, RGCanPlan, RGFailureAt, RGEligible, RGAdvances, RGInvariant, RGDriverAuthority, RGType, RGSelection, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
<1>12. RGFenced'
    BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGChooseDone, RGCanGrow, RGCanPlan, RGFailureAt, RGEligible, RGAdvances, RGInvariant, RGFenced, RGType, RGSelection, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12 DEF RGInvariant

THEOREM RGDelegateStep == ASSUME RGInvariant, RGDelegate PROVE RGInvariant'
<1>1. RGType'
    BY SMT DEF RGDelegate, RGInvariant, RGType, RGPhases
<1>2. RGSelection'
    BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGDelegate, RGCanGrow, RGCanPlan, RGFailureAt, RGEligible, RGAdvances, RGInvariant, RGSelection, RGType, RGPhaseOrder, RGPublication, RGFenced, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
<1>3. RGPlanIdentity'
    BY SMT DEF RGDelegate, RGInvariant, RGPlanIdentity
<1>4. RGComposition'
    BY SMT DEF RGDelegate, RGInvariant, RGComposition
<1>5. RGSystemUnchanged'
    BY SMT DEF RGDelegate, RGInvariant, RGSystemUnchanged
<1>6. RGPhaseOrder'
    BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGDelegate, RGCanGrow, RGCanPlan, RGFailureAt, RGEligible, RGAdvances, RGInvariant, RGPhaseOrder, RGType, RGPublication, RGFenced, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
<1>7. RGEngineBinding'
    BY SMT DEF RGDelegate, RGInvariant, RGEngineBinding
<1>8. RGReceiptsExact'
    BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGDelegate, RGCanGrow, RGCanPlan, RGFailureAt, RGEligible, RGAdvances, RGInvariant, RGReceiptsExact, RGType, RGPhaseOrder, RGPublication, RGFenced, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
<1>9. RGPublication'
    BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGDelegate, RGCanGrow, RGCanPlan, RGFailureAt, RGEligible, RGAdvances, RGInvariant, RGPublication, RGType, RGPhaseOrder, RGFenced, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
<1>10. RGContiguity'
    BY SMT DEF RGDelegate, RGInvariant, RGContiguity
<1>11. RGDriverAuthority'
    BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGDelegate, RGCanGrow, RGCanPlan, RGFailureAt, RGEligible, RGAdvances, RGInvariant, RGDriverAuthority, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
<1>12. RGFenced'
    BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGDelegate, RGCanGrow, RGCanPlan, RGFailureAt, RGEligible, RGAdvances, RGInvariant, RGFenced, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12 DEF RGInvariant

THEOREM RGLoadStep == ASSUME RGInvariant, RGLoad PROVE RGInvariant'
<1>1. RGType'
    BY SMT DEF RGLoad, RGInvariant, RGType, RGPhases
<1>2. RGSelection'
    BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGLoad, RGCanGrow, RGCanPlan, RGFailureAt, RGEligible, RGAdvances, RGInvariant, RGSelection, RGType, RGPhaseOrder, RGPublication, RGFenced, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
<1>3. RGPlanIdentity'
    BY SMT DEF RGLoad, RGInvariant, RGPlanIdentity
<1>4. RGComposition'
    BY SMT DEF RGLoad, RGInvariant, RGComposition
<1>5. RGSystemUnchanged'
    BY SMT DEF RGLoad, RGInvariant, RGSystemUnchanged
<1>6. RGPhaseOrder'
    BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGLoad, RGCanGrow, RGCanPlan, RGFailureAt, RGEligible, RGAdvances, RGInvariant, RGPhaseOrder, RGType, RGPublication, RGFenced, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
<1>7. RGEngineBinding'
    BY SMT DEF RGLoad, RGInvariant, RGEngineBinding
<1>8. RGReceiptsExact'
    BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGLoad, RGCanGrow, RGCanPlan, RGFailureAt, RGEligible, RGAdvances, RGInvariant, RGReceiptsExact, RGType, RGPhaseOrder, RGPublication, RGFenced, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
<1>9. RGPublication'
    BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGLoad, RGCanGrow, RGCanPlan, RGFailureAt, RGEligible, RGAdvances, RGInvariant, RGPublication, RGType, RGPhaseOrder, RGFenced, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
<1>10. RGContiguity'
    BY SMT DEF RGLoad, RGInvariant, RGContiguity
<1>11. RGDriverAuthority'
    BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGLoad, RGCanGrow, RGCanPlan, RGFailureAt, RGEligible, RGAdvances, RGInvariant, RGDriverAuthority, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
<1>12. RGFenced'
    BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGLoad, RGCanGrow, RGCanPlan, RGFailureAt, RGEligible, RGAdvances, RGInvariant, RGFenced, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12 DEF RGInvariant

THEOREM RGPlanDoneStep == ASSUME RGInvariant, RGPlanDone PROVE RGInvariant'
<1>1. RGType'
    BY SMT DEF RGPlanDone, RGInvariant, RGType, RGPhases
<1>2. RGSelection'
    BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGPlanDone, RGCanGrow, RGCanPlan, RGFailureAt, RGEligible, RGAdvances, RGInvariant, RGSelection, RGType, RGPhaseOrder, RGPublication, RGFenced, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
<1>3. RGPlanIdentity'
    BY SMT DEF RGPlanDone, RGInvariant, RGPlanIdentity
<1>4. RGComposition'
    BY SMT DEF RGPlanDone, RGInvariant, RGComposition
<1>5. RGSystemUnchanged'
    BY SMT DEF RGPlanDone, RGInvariant, RGSystemUnchanged
<1>6. RGPhaseOrder'
    BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGPlanDone, RGCanGrow, RGCanPlan, RGFailureAt, RGEligible, RGAdvances, RGInvariant, RGPhaseOrder, RGType, RGPublication, RGFenced, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
<1>7. RGEngineBinding'
    BY SMT DEF RGPlanDone, RGInvariant, RGEngineBinding
<1>8. RGReceiptsExact'
    BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGPlanDone, RGCanGrow, RGCanPlan, RGFailureAt, RGEligible, RGAdvances, RGInvariant, RGReceiptsExact, RGType, RGPhaseOrder, RGPublication, RGFenced, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
<1>9. RGPublication'
    BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGPlanDone, RGCanGrow, RGCanPlan, RGFailureAt, RGEligible, RGAdvances, RGInvariant, RGPublication, RGType, RGPhaseOrder, RGFenced, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
<1>10. RGContiguity'
    BY SMT DEF RGPlanDone, RGInvariant, RGContiguity
<1>11. RGDriverAuthority'
    BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGPlanDone, RGCanGrow, RGCanPlan, RGFailureAt, RGEligible, RGAdvances, RGInvariant, RGDriverAuthority, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
<1>12. RGFenced'
    BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGPlanDone, RGCanGrow, RGCanPlan, RGFailureAt, RGEligible, RGAdvances, RGInvariant, RGFenced, RGType, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12 DEF RGInvariant

THEOREM RGEffectStep == ASSUME RGInvariant, RGEffect PROVE RGInvariant'
<1>1. RGType'
    BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, RMApplyType, SMT DEF RGEffect, RGCanGrow, RGCanPlan, RGFailureAt, RGEligible, RGAdvances, RGInvariant, RGType, RGComposition, RGEngineBinding, RGPlanIdentity, RGReceiptsExact, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
<1>2. RGSelection'
    BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, RMApplyType, SMT DEF RGEffect, RGCanGrow, RGCanPlan, RGFailureAt, RGEligible, RGAdvances, RGInvariant, RGSelection, RGType, RGPhaseOrder, RGPublication, RGFenced, RGComposition, RGEngineBinding, RGPlanIdentity, RGReceiptsExact, RGEntries, RGKinds, RGPositions, RGOutcomes, RGNoReceipt, RGGroupFailures, RGPhases
<1>3. RGPlanIdentity'
    BY SMT DEF RGEffect, RGInvariant, RGPlanIdentity
<1>4. RGComposition'
    BY SMT DEF RGEffect, RGInvariant, RGComposition
<1>5. RGSystemUnchanged'
    BY SMT DEF RGEffect, RGInvariant, RGSystemUnchanged
<1>6. RGPhaseOrder'
    BY SMT DEF RGEffect, RGInvariant, RGPhaseOrder, RGFenced
<1>7. RGEngineBinding'
    BY SMT DEF RGEffect, RGInvariant, RGEngineBinding, RGComposition, RGPhaseOrder
<1>8. RGReceiptsExact'
    BY SMT DEF RGEffect, RGInvariant, RGReceiptsExact
<1>9. RGPublication'
    BY SMT DEF RGEffect, RGInvariant, RGPublication
<1>10. RGContiguity'
    BY SMT DEF RGEffect, RGInvariant, RGContiguity
<1>11. RGDriverAuthority'
    BY SMT DEF RGEffect, RGInvariant, RGDriverAuthority
<1>12. RGFenced'
    BY SMT DEF RGEffect, RGInvariant, RGFenced
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12 DEF RGInvariant

THEOREM RGEngineSuccessStep == ASSUME RGInvariant, RGEngineSuccess PROVE RGInvariant'
<1>1. RGType'
    BY SMT DEF RGEngineSuccess, RGInvariant, RGType, RGPhases
<1>2. RGSelection'
    BY SMT DEF RGEngineSuccess, RGInvariant, RGSelection, RGCanGrow
<1>3. RGPlanIdentity'
    BY SMT DEF RGEngineSuccess, RGInvariant, RGPlanIdentity
<1>4. RGComposition'
    BY SMT DEF RGEngineSuccess, RGInvariant, RGComposition
<1>5. RGSystemUnchanged'
    BY SMT DEF RGEngineSuccess, RGInvariant, RGSystemUnchanged
<1>6. RGPhaseOrder'
    BY SMT DEF RGEngineSuccess, RGInvariant, RGPhaseOrder, RGFenced
<1>7. RGEngineBinding'
    BY SMT DEF RGEngineSuccess, RGInvariant, RGEngineBinding
<1>8. RGReceiptsExact'
    BY SMT DEF RGEngineSuccess, RGInvariant, RGReceiptsExact
<1>9. RGPublication'
    BY SMT DEF RGEngineSuccess, RGInvariant, RGPublication, RGFenced, RGGroupFailures
<1>10. RGContiguity'
    BY SMT DEF RGEngineSuccess, RGInvariant, RGContiguity
<1>11. RGDriverAuthority'
    BY SMT DEF RGEngineSuccess, RGInvariant, RGDriverAuthority
<1>12. RGFenced'
    BY SMT DEF RGEngineSuccess, RGInvariant, RGFenced
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12 DEF RGInvariant

THEOREM RGPublishStep == ASSUME RGInvariant, RGPublish PROVE RGInvariant'
<1>1. RGType'
    <2>1. rgCount \in RGEntries /\ rgCount \in 0..RGItems /\ RGPosition[rgCount] \in RGPositions
        BY RGEntryType, RGStateNumbers, RGInputBounds, SMT DEF RGInvariant, RGType, RGEntries
    <2>2. rgReceipts' \in [RGEntries -> [term : Nat, index : Nat, outcome : RGOutcomes, region : Nat]]
        <3>1. RGPosition \in [RGEntries -> RGPositions] /\ RGRegion \in [RGEntries -> Nat] /\
            rgStaged \in [RGEntries -> RGOutcomes]
            BY RGLegalInputs, SMT DEF RGInvariant, RGType
        <3>2. rgReceipts' = [i \in RGEntries |-> IF i <= rgCount
            THEN [term |-> RGPosition[i].term, index |-> RGPosition[i].index, outcome |-> rgStaged[i],
                region |-> IF rgStaged[i] = "stale" THEN RGRegion[i] ELSE 0] ELSE RGNoReceipt]
            BY DEF RGPublish
        <3> QED BY ONLY <3>1, <3>2, SMT DEF RGPositions, RGNoReceipt, RGOutcomes
    <2> QED BY <2>1, <2>2, SMT DEF RGPublish, RGInvariant, RGType, RGPhases
<1>2. RGSelection'
    BY SMT DEF RGPublish, RGInvariant, RGSelection, RGCanGrow
<1>3. RGPlanIdentity'
    BY SMT DEF RGPublish, RGInvariant, RGPlanIdentity
<1>4. RGComposition'
    BY SMT DEF RGPublish, RGInvariant, RGComposition
<1>5. RGSystemUnchanged'
    BY SMT DEF RGPublish, RGInvariant, RGSystemUnchanged
<1>6. RGPhaseOrder'
    BY SMT DEF RGPublish, RGInvariant, RGPhaseOrder, RGFenced
<1>7. RGEngineBinding'
    BY SMT DEF RGPublish, RGInvariant, RGEngineBinding
<1>8. RGReceiptsExact'
    <2>1. rgPrepared = rgCount /\ rgHasEffect /\ rgEngineOk
        BY SMT DEF RGPublish, RGInvariant, RGPhaseOrder
    <2>2. \A i \in 1..rgCount : rgStaged[i] = rgSequential[i] /\ rgStaged[i] # "none"
        BY <2>1, RGEffectiveType, RGLegalInputs, RGStateNumbers, SMT DEF RGInvariant, RGType, RGPlanIdentity, RGEntries
    <2>3. rgHasEffect' /\ rgEngineOk' /\ rgCount' = rgCount /\ rgSequential' = rgSequential /\
        rgPhase' = "tail" /\ rgReceipts' = [i \in RGEntries |-> IF i <= rgCount
            THEN [term |-> RGPosition[i].term, index |-> RGPosition[i].index, outcome |-> rgStaged[i],
                region |-> IF rgStaged[i] = "stale" THEN RGRegion[i] ELSE 0] ELSE RGNoReceipt]
        BY <2>1, SMT DEF RGPublish
    <2>4. rgCount \in Nat /\ RGItems \in Nat /\ rgCount <= RGItems
        BY RGStateNumbers, RGInputBounds, SMT DEF RGInvariant
    <2> QED BY ONLY <2>2, <2>3, <2>4, SMT DEF RGReceiptsExact, RGNoReceipt, RGEntries
<1>9. RGPublication'
    BY RGStateNumbers, SMT DEF RGPublish, RGInvariant, RGPublication, RGFenced, RGGroupFailures
<1>10. RGContiguity'
    BY RGStateNumbers, RGInputBounds, SMT DEF RGPublish, RGInvariant, RGContiguity
<1>11. RGDriverAuthority'
    BY SMT DEF RGPublish, RGInvariant, RGDriverAuthority
<1>12. RGFenced'
    BY SMT DEF RGPublish, RGInvariant, RGFenced
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12 DEF RGInvariant

THEOREM RGTailPassStep == ASSUME RGInvariant, RGTailPass PROVE RGInvariant'
<1>1. RGType'
    BY RGStateNumbers, RGInputBounds, SMT DEF RGTailPass, RGInvariant, RGType
<1>2. RGSelection'
    BY SMT DEF RGTailPass, RGInvariant, RGSelection, RGCanGrow
<1>3. RGPlanIdentity'
    BY SMT DEF RGTailPass, RGInvariant, RGPlanIdentity
<1>4. RGComposition'
    BY SMT DEF RGTailPass, RGInvariant, RGComposition
<1>5. RGSystemUnchanged'
    BY SMT DEF RGTailPass, RGInvariant, RGSystemUnchanged
<1>6. RGPhaseOrder'
    BY SMT DEF RGTailPass, RGInvariant, RGPhaseOrder
<1>7. RGEngineBinding'
    BY SMT DEF RGTailPass, RGInvariant, RGEngineBinding
<1>8. RGReceiptsExact'
    BY SMT DEF RGTailPass, RGInvariant, RGReceiptsExact
<1>9. RGPublication'
    BY RGStateNumbers, SMT DEF RGTailPass, RGInvariant, RGPublication
<1>10. RGContiguity'
    <2>1. SUFFICES ASSUME NEW i \in (rgCount' + 1)..rgProcessed'
        PROVE RGKind[i] # "malformed" /\ i \in RGTailSuccess BY DEF RGContiguity
    <2>2. CASE i = rgProcessed + 1
        BY <2>1, <2>2, SMT DEF RGTailPass
    <2>3. CASE i # rgProcessed + 1
        <3>1. i \in (rgCount + 1)..rgProcessed
            BY <2>1, <2>3, RGStateNumbers, SMT DEF RGTailPass, RGInvariant
        <3> QED BY ONLY <3>1, RGInvariant, SMT DEF RGInvariant, RGContiguity
    <2> QED BY <2>2, <2>3, SMT
<1>11. RGDriverAuthority'
    BY SMT DEF RGTailPass, RGInvariant, RGDriverAuthority
<1>12. RGFenced'
    BY SMT DEF RGTailPass, RGInvariant, RGFenced
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12 DEF RGInvariant

THEOREM RGReportStep == ASSUME RGInvariant, RGReport PROVE RGInvariant'
<1>1. RGType'
    <2>1. RGItems \in RGEntries /\ RGPosition[RGItems] \in RGPositions
        BY RGEntryType, RGInputBounds, SMT DEF RGEntries
    <2>2. rgDriver' \in RGPositions
        BY <2>1, SMT DEF RGReport, RGInvariant, RGType
    <2> QED BY <2>2, SMT DEF RGReport, RGInvariant, RGType, RGPhases
<1>2. RGSelection'
    BY SMT DEF RGReport, RGInvariant, RGSelection, RGCanGrow
<1>3. RGPlanIdentity'
    BY SMT DEF RGReport, RGInvariant, RGPlanIdentity
<1>4. RGComposition'
    BY SMT DEF RGReport, RGInvariant, RGComposition
<1>5. RGSystemUnchanged'
    BY SMT DEF RGReport, RGInvariant, RGSystemUnchanged
<1>6. RGPhaseOrder'
    BY SMT DEF RGReport, RGInvariant, RGPhaseOrder
<1>7. RGEngineBinding'
    BY SMT DEF RGReport, RGInvariant, RGEngineBinding
<1>8. RGReceiptsExact'
    BY SMT DEF RGReport, RGInvariant, RGReceiptsExact
<1>9. RGPublication'
    BY SMT DEF RGReport, RGInvariant, RGPublication, RGFenced, RGGroupFailures
<1>10. RGContiguity'
    BY SMT DEF RGReport, RGInvariant, RGContiguity
<1>11. RGDriverAuthority'
    BY RGEntryType, RGInputBounds, SMT DEF RGReport, RGInvariant, RGType, RGDriverAuthority, RGEntries, RGPositions
<1>12. RGFenced'
    BY SMT DEF RGReport, RGInvariant, RGFenced
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12 DEF RGInvariant

THEOREM RGFailStep == ASSUME RGInvariant, RGFail PROVE RGInvariant'
<1>1. RGType'
    BY SMT DEF RGFail, RGInvariant, RGType, RGFailureAt, RGGroupFailures, RGPhases
<1>2. RGSelection'
    BY SMT DEF RGFail, RGInvariant, RGSelection, RGFailureAt, RGCanGrow
<1>3. RGPlanIdentity'
    BY SMT DEF RGFail, RGInvariant, RGPlanIdentity
<1>4. RGComposition'
    BY SMT DEF RGFail, RGInvariant, RGComposition
<1>5. RGSystemUnchanged'
    BY SMT DEF RGFail, RGInvariant, RGSystemUnchanged
<1>6. RGPhaseOrder'
    BY SMT DEF RGFail, RGInvariant, RGPhaseOrder, RGFailureAt, RGFenced
<1>7. RGEngineBinding'
    BY SMT DEF RGFail, RGInvariant, RGEngineBinding
<1>8. RGReceiptsExact'
    BY SMT DEF RGFail, RGInvariant, RGReceiptsExact, RGFailureAt, RGFenced
<1>9. RGPublication'
    BY SMT DEF RGFail, RGInvariant, RGPublication, RGFailureAt, RGPhaseOrder, RGDriverAuthority, RGFenced, RGGroupFailures
<1>10. RGContiguity'
    BY SMT DEF RGFail, RGInvariant, RGContiguity
<1>11. RGDriverAuthority'
    BY SMT DEF RGFail, RGInvariant, RGDriverAuthority, RGFailureAt
<1>12. RGFenced'
    BY SMT DEF RGFail, RGInvariant, RGFenced
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12 DEF RGInvariant

THEOREM RGPlanStep == ASSUME RGInvariant, RGPlan PROVE RGInvariant'
<1>0. /\ rgPrepared + 1 \in RGEntries
    /\ RGEffective(rgPrepared + 1, RGInitial) = RGEffective(rgPrepared + 1, rgReference)
    /\ RGVerdict(rgPrepared + 1, RGInitial) = RGVerdict(rgPrepared + 1, rgReference)
    <2>1. rgPrepared + 1 \in RGEntries /\ RGEligible(rgPrepared + 1) /\
        RGInitial \in RMStates /\ rgReference \in RMStates
        BY RGStateNumbers, RGInputBounds, RGLegalInputs, SMT
            DEF RGPlan, RGCanPlan, RGInvariant, RGType, RGEntries
    <2>2. \A key \in RMSystemKeys : RGInitial[key] = rgReference[key]
        BY SMT DEF RGInvariant, RGSystemUnchanged
    <2> QED BY ONLY <2>1, <2>2, RGVerdictSameSystem, SMT
<1>1. RGType'
    <2>1. rgBatch' \in Seq(RMMutations) /\ rgReference' \in RMStates
        BY <1>0, RGEffectiveType, RMApplyType, RGLegalInputs, RGStateNumbers, RGInputBounds, RGInitialTypes, RGEntryType, RGIntervalExtend, RGIntervalPredecessor, SMT DEF RGPlan, RGInvariant, RGType
    <2>2. rgStaged' \in [RGEntries -> RGOutcomes] /\ rgSequential' \in [RGEntries -> RGOutcomes]
        BY <1>0, RGPlanVerdictTypes, SMT DEF RGPlan, RGInvariant, RGType
    <2>3. rgPrepared' \in 0..rgCount' /\ rgPrevious' \in RGPositions
        BY <1>0, RGEntryType, RGInputBounds, SMT DEF RGPlan, RGCanPlan, RGInvariant, RGType, RGEntries
    <2> QED BY <2>1, <2>2, <2>3, SMT DEF RGPlan, RGInvariant, RGType
<1>2. RGSelection'
    BY SMT DEF RGPlan, RGInvariant, RGSelection, RGCanGrow
<1>3. RGPlanIdentity'
    <2>1. rgPrevious' = RGPosition[rgPrepared'] /\ rgPrepared' = rgPrepared + 1 /\
        RGAdvances(rgPrevious, RGPosition[rgPrepared + 1])
        BY SMT DEF RGPlan, RGCanPlan
    <2>2. \A i \in 1..rgPrepared' : RGAdvances(IF i = 1 THEN RGBasePosition ELSE RGPosition[i - 1], RGPosition[i])
        <3>1. SUFFICES ASSUME NEW i \in 1..rgPrepared'
            PROVE RGAdvances(IF i = 1 THEN RGBasePosition ELSE RGPosition[i - 1], RGPosition[i]) OBVIOUS
        <3>2. CASE i = rgPrepared + 1
            BY <3>1, <3>2, <2>1, RGInputBounds, SMT DEF RGInvariant, RGType, RGPlanIdentity
        <3>3. CASE i # rgPrepared + 1
            <4>1. i \in 1..rgPrepared
                BY <3>1, <3>3, <2>1, RGStateNumbers, RGIntervalExtend, SMT DEF RGInvariant
            <4> QED BY <4>1, SMT DEF RGInvariant, RGPlanIdentity
        <3> QED BY <3>2, <3>3, SMT
    <2>3. \A i \in 1..rgPrepared' : ~(RGKind[i] = "fenced" /\ i \in RGReadFails)
        BY <2>1, RGInputBounds, SMT DEF RGPlan, RGCanPlan, RGInvariant, RGType, RGPlanIdentity
    <2>4. \A i \in RGEntries :
        /\ (i <= rgPrepared' => rgStaged'[i] = rgSequential'[i] /\ rgStaged'[i] = RGVerdict(i, RGInitial) /\ rgSequential'[i] = RGVerdict(i, RGSequence[i - 1]))
        /\ (i > rgPrepared' => rgStaged'[i] = "none" /\ rgSequential'[i] = "none")
        <3>1. SUFFICES ASSUME NEW i \in RGEntries PROVE
            /\ (i <= rgPrepared' => rgStaged'[i] = rgSequential'[i] /\ rgStaged'[i] = RGVerdict(i, RGInitial) /\ rgSequential'[i] = RGVerdict(i, RGSequence[i - 1]))
            /\ (i > rgPrepared' => rgStaged'[i] = "none" /\ rgSequential'[i] = "none") OBVIOUS
        <3>2. CASE i = rgPrepared + 1
            BY <3>1, <3>2, <1>0, <2>1, RGInputBounds, SMT DEF RGPlan, RGInvariant, RGType, RGPlanIdentity, RGComposition, RGEntries
        <3>3. CASE i # rgPrepared + 1
            BY <3>1, <3>3, <2>1, RGInputBounds, SMT DEF RGPlan, RGInvariant, RGType, RGPlanIdentity, RGEntries
        <3> QED BY <3>2, <3>3, SMT
    <2> QED BY <2>1, <2>2, <2>3, <2>4, RGInputBounds, SMT DEF RGPlanIdentity, RGInvariant, RGType
<1>4. RGComposition'
    BY <1>0, RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, RGEffectiveType, RGEffectiveRaw, RMApplyConcatenation, RMRawBatchConcatenation, RGSequenceRecurrence, SMT DEF RGPlan, RGCanPlan, RGCanGrow, RGAdvances, RGInvariant, RGType, RGComposition, RGPhaseOrder, RGPublication, RGFenced
<1>5. RGSystemUnchanged'
    BY <1>0, RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, RGEffectiveType, RGEffectiveRaw, RMApplySystem, SMT DEF RGPlan, RGCanPlan, RGCanGrow, RGAdvances, RGInvariant, RGType, RGSystemUnchanged, RGPhaseOrder, RGPublication, RGFenced
<1>6. RGPhaseOrder'
    BY SMT DEF RGPlan, RGInvariant, RGPhaseOrder
<1>7. RGEngineBinding'
    BY SMT DEF RGPlan, RGInvariant, RGEngineBinding, RGPhaseOrder, RGFenced
<1>8. RGReceiptsExact'
    BY SMT DEF RGPlan, RGInvariant, RGReceiptsExact, RGPublication, RGFenced, RGNoReceipt
<1>9. RGPublication'
    BY SMT DEF RGPlan, RGInvariant, RGPublication
<1>10. RGContiguity'
    BY SMT DEF RGPlan, RGInvariant, RGContiguity
<1>11. RGDriverAuthority'
    BY SMT DEF RGPlan, RGInvariant, RGDriverAuthority
<1>12. RGFenced'
    BY SMT DEF RGPlan, RGInvariant, RGFenced
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, <1>9, <1>10, <1>11, <1>12 DEF RGInvariant

THEOREM RGStutterStep == ASSUME RGInvariant, UNCHANGED rgVars PROVE RGInvariant'
BY SMT DEF rgVars, RGCanGrow, RGInvariant, RGType, RGSelection, RGPlanIdentity, RGComposition, RGSystemUnchanged, RGPhaseOrder, RGEngineBinding, RGReceiptsExact, RGPublication, RGContiguity, RGDriverAuthority, RGFenced

THEOREM RGInvariantStep == ASSUME RGInvariant, [RGNext]_rgVars PROVE RGInvariant'
<1>1. RGService => RGInvariant'
    BY RGGrowStep, RGChooseDoneStep, RGDelegateStep, RGLoadStep, RGPlanDoneStep, RGEffectStep, RGEngineSuccessStep, RGPublishStep, RGTailPassStep, RGReportStep, RGFailStep, RGPlanStep, SMT DEF RGService
<1> QED BY <1>1, RGStutterStep, SMT DEF RGNext, RGQuiesce

THEOREM RGInvariantAlways == RGSpec => []RGInvariant
BY RGInvariantInit, RGInvariantStep, PTL DEF RGSpec

THEOREM RGTerminalStep == ASSUME RGInvariant, RGNext PROVE RGTerminalStable
<1>1. RGTerminal => ~RGService
    BY SMT DEF RGTerminal, RGService, RGGrow, RGChooseDone, RGDelegate,
        RGLoad, RGPlan, RGPlanDone, RGEffect, RGEngineSuccess, RGPublish,
        RGTailPass, RGReport, RGFail, RGFailureAt
<1> QED BY <1>1, SMT DEF RGNext, RGQuiesce, RGTerminalStable

THEOREM RGTerminalAlways == RGSpec => RGSafety
<1>1. RGInvariant /\ [RGNext]_rgVars => [RGTerminalStable]_rgVars
    BY RGTerminalStep, SMT DEF rgVars
<1> QED BY <1>1, RGInvariantAlways, PTL DEF RGSpec, RGSafety

THEOREM RGSingletonFallback == ASSUME RGInvariant, RGChooseDone, rgCount = 1 PROVE rgPhase' = "singleton"
BY DEF RGChooseDone

THEOREM RGFallbackAlways == RGSpec => RGFallbackSafety
<1>1. RGInvariant /\ [RGNext]_rgVars => [RGFallbackStep]_rgVars
    BY SMT DEF RGFallbackStep, RGNext, RGService, RGQuiesce, rgVars,
        RGGrow, RGChooseDone, RGDelegate, RGLoad, RGPlan, RGPlanDone, RGEffect,
        RGEngineSuccess, RGPublish, RGTailPass, RGReport, RGFail, RGFailureAt
<1> QED BY <1>1, RGInvariantAlways, PTL DEF RGSpec, RGFallbackSafety

THEOREM RGRankType == ASSUME RGInvariant PROVE RGRank \in Nat /\ RGRank <= 4 * RGItems + 7 /\ (RGRank = 0 <=> RGTerminal)
BY RGLegalInputs, RGInputBounds, RGEntryType, RGInitialTypes, RGIntervalNext, SMT DEF RGRank, RGTerminal, RGInvariant, RGType, RGPhaseOrder, RGSelection, RGPublication, RGFenced, RGEntries, RGPhases

THEOREM RGRankDecreases == ASSUME RGInvariant, RGService PROVE RGRank' < RGRank
<1>0. RGItems \in Nat \ {0} /\ rgCount \in 1..RGItems /\
    rgPrepared \in 0..rgCount /\ rgProcessed \in 0..RGItems /\
    (rgPhase = "choose" => rgPrepared = 0) /\
    (rgPhase \in {"write", "effect", "publish", "tail", "done"} => rgPrepared = rgCount)
    BY RGInputBounds, SMT DEF RGInvariant, RGType, RGPhaseOrder
<1>1. RGService => RGRank' < RGRank
    BY ONLY <1>0, SMT DEF RGRank, RGService, RGGrow, RGChooseDone, RGDelegate,
        RGLoad, RGPlan, RGPlanDone, RGEffect, RGEngineSuccess, RGPublish,
        RGTailPass, RGReport, RGFail, RGCanGrow, RGCanPlan, RGFailureAt
<1> QED BY <1>1

THEOREM RGFairInvariant == RGFairSpec => []RGInvariant
BY RGInvariantStep, PTL DEF RGFairSpec

THEOREM RGGrowEnabled == (rgPhase = "choose" /\ RGCanGrow) => ENABLED RGGrow
BY ExpandENABLED, Isa DEF RGGrow

THEOREM RGChooseDoneEnabled == (rgPhase = "choose" /\ ~RGCanGrow) => ENABLED RGChooseDone
BY ExpandENABLED, Isa DEF RGChooseDone

THEOREM RGDelegateEnabled == (rgPhase = "singleton") => ENABLED RGDelegate
BY ExpandENABLED, Isa DEF RGDelegate

THEOREM RGLoadEnabled == (rgPhase = "load") => ENABLED RGLoad
BY ExpandENABLED, Isa DEF RGLoad

THEOREM RGPlanEnabled == (rgPhase = "prepare" /\ RGCanPlan) => ENABLED RGPlan
BY ExpandENABLED, Isa DEF RGPlan

THEOREM RGPlanDoneEnabled == (rgPhase = "prepare" /\ rgPrepared = rgCount) => ENABLED RGPlanDone
BY ExpandENABLED, Isa DEF RGPlanDone

THEOREM RGEffectEnabled == (rgPhase = "write") => ENABLED RGEffect
BY ExpandENABLED, Isa DEF RGEffect

THEOREM RGEngineSuccessEnabled == (rgPhase = "effect") => ENABLED RGEngineSuccess
BY ExpandENABLED, Isa DEF RGEngineSuccess

THEOREM RGPublishEnabled == (rgPhase = "publish" /\ rgEngineOk) => ENABLED RGPublish
BY ExpandENABLED, Isa DEF RGPublish

THEOREM RGTailPassEnabled == (rgPhase = "tail" /\ rgProcessed < RGItems /\ RGKind[rgProcessed + 1] # "malformed" /\ rgProcessed + 1 \in RGTailSuccess) => ENABLED RGTailPass
BY ExpandENABLED, Isa DEF RGTailPass

THEOREM RGReportEnabled == (rgPhase = "tail" /\ rgProcessed = RGItems) => ENABLED RGReport
BY ExpandENABLED, Isa DEF RGReport

THEOREM RGFailEnabled == (RGFailureAt # "none") => ENABLED RGFail
BY ExpandENABLED, Isa DEF RGFail

THEOREM RGEnabledDisjunction == (ENABLED RGService) <=> ((ENABLED RGGrow) \/ (ENABLED RGChooseDone) \/ (ENABLED RGDelegate) \/ (ENABLED RGLoad) \/ (ENABLED RGPlan) \/ (ENABLED RGPlanDone) \/ (ENABLED RGEffect) \/ (ENABLED RGEngineSuccess) \/ (ENABLED RGPublish) \/ (ENABLED RGTailPass) \/ (ENABLED RGReport) \/ (ENABLED RGFail))
BY ExpandENABLED, Isa DEF RGService, RGGrow, RGChooseDone, RGDelegate, RGLoad, RGPlan, RGPlanDone, RGEffect, RGEngineSuccess, RGPublish, RGTailPass, RGReport, RGFail

THEOREM RGServiceEnabled == ASSUME RGInvariant, ~RGTerminal PROVE ENABLED <<RGService>>_rgVars
<1>1. ENABLED RGService
    BY RGGrowEnabled, RGChooseDoneEnabled, RGDelegateEnabled, RGLoadEnabled, RGPlanEnabled, RGPlanDoneEnabled, RGEffectEnabled, RGEngineSuccessEnabled, RGPublishEnabled, RGTailPassEnabled, RGReportEnabled, RGFailEnabled, RGEnabledDisjunction, RGInputBounds, SMT DEF RGInvariant, RGType, RGPhaseOrder, RGTerminal, RGPhases, RGFailureAt, RGCanPlan
<1>2. RGService <=> <<RGService>>_rgVars
    <2>1. RGService => RGRank' < RGRank BY RGRankDecreases
    <2>2. UNCHANGED rgVars => RGRank' = RGRank
        BY SMT DEF rgVars, RGRank
    <2> QED BY <2>1, <2>2, SMT
<1>3. (ENABLED RGService) <=> (ENABLED <<RGService>>_rgVars)
    BY <1>2, ENABLEDaxioms
<1> QED BY <1>1, <1>3, SMT

RGBudgetDrop(k) == RGFairSpec => ((~RGTerminal /\ RGRank = k) ~> (RGTerminal \/ RGRank < k))
THEOREM RGBudgetProgress == ASSUME NEW k \in Nat PROVE RGBudgetDrop(k)
<1>1. DEFINE P == ~RGTerminal /\ RGRank = k
            Q == RGTerminal \/ RGRank < k
<1>2. RGInvariant /\ P /\ [RGNext]_rgVars => RGInvariant' /\ (P' \/ Q')
    BY RGInvariantStep, RGRankDecreases, SMT DEF P, Q, RGNext, RGQuiesce, rgVars, RGRank, RGTerminal
<1>3. RGInvariant /\ P /\ RGService => Q'
    BY RGRankDecreases, SMT DEF P, Q
<1>4. RGInvariant /\ P => ENABLED <<RGService>>_rgVars
    BY RGServiceEnabled, SMT DEF P
<1> QED BY RGFairInvariant, <1>2, <1>3, <1>4, PTL DEF RGFairSpec, RGFairness, P, Q, RGBudgetDrop

THEOREM RGBoundedProgress == RGFairSpec => RGProgress
<1>1. DEFINE P(n) == RGFairSpec => ((RGRank <= n) ~> RGTerminal)
<1> HIDE DEF P
<1>2. P(0)
    <2>1. RGInvariant /\ RGRank <= 0 => RGTerminal BY RGRankType, SMT
    <2> QED BY <2>1, RGFairInvariant, PTL DEF P
<1>3. ASSUME NEW n \in Nat PROVE P(n) => P(n + 1)
    <2>1. RGInvariant /\ RGRank <= n + 1 =>
        (RGRank <= n) \/ RGTerminal \/ (~RGTerminal /\ RGRank = n + 1)
        BY <1>3, RGRankType, SMT
    <2>2. RGInvariant /\ RGRank < n + 1 => RGRank <= n BY <1>3, RGRankType, SMT
    <2>3. RGBudgetDrop(n + 1) BY <1>3, RGBudgetProgress, SMT
    <2> QED BY <2>1, <2>2, <2>3, RGFairInvariant, PTL DEF P, RGBudgetDrop
<1>4. \A n \in Nat : P(n) BY <1>2, <1>3, NatInduction, Isa
<1>5. P(4 * RGItems + 7) BY <1>4, RGLegalInputs, RGStateNumbers, RGInputBounds, RGInitialTypes, RGEntryType, RGIntervalExtend, RGIntervalPredecessor, SMT
<1>6. RGInvariant => RGRank <= 4 * RGItems + 7 BY RGRankType
<1> QED BY <1>5, <1>6, RGFairInvariant, PTL DEF P, RGProgress
=============================================================================
