------------------------- MODULE RawMutationProof -------------------------
EXTENDS RawMutation, TLAPS, NaturalsInduction

THEOREM RMSequencesInductionAppend ==
    ASSUME NEW S, NEW P(_), P(<<>>),
        \A batch \in Seq(S), mutation \in S : P(batch) => P(Append(batch, mutation))
    PROVE \A batch \in Seq(S) : P(batch)
<1> DEFINE Q(n) == \A batch \in Seq(S) : Len(batch) = n => P(batch)
<1> HIDE DEF Q
<1>1. Q(0) BY SMT DEF Q
<1>2. ASSUME NEW n \in Nat, Q(n) PROVE Q(n + 1)
    <2>1. SUFFICES ASSUME NEW batch \in Seq(S), Len(batch) = n + 1 PROVE P(batch)
        BY DEF Q
    <2>2. [i \in 1..n |-> batch[i]] \in Seq(S) /\ Len([i \in 1..n |-> batch[i]]) = n /\
        batch[n + 1] \in S /\ batch = Append([i \in 1..n |-> batch[i]], batch[n + 1])
        BY <1>2, <2>1, SMT
    <2>3. P([i \in 1..n |-> batch[i]]) BY <1>2, <2>2, SMT DEF Q
    <2> QED BY <2>2, <2>3, SMT
<1>3. \A n \in Nat : Q(n) BY <1>1, <1>2, NatInduction, Isa
<1> QED BY <1>3, SMT DEF Q

THEOREM RMMutateType == ASSUME NEW state \in RMStates, NEW mutation \in RMMutations
    PROVE RMMutate(state, mutation) \in RMStates
BY SMT DEF RMMutate, RMStates, RMMutations

THEOREM RMTraceDefinition == ASSUME NEW state \in RMStates, NEW batch \in Seq(RMMutations)
    PROVE FiniteNatInductiveDefConclusion(RMTrace(state, batch), state,
        LAMBDA value, n : RMMutate(value, batch[n]), 0, Len(batch))
<1> DEFINE Step(value, n) == RMMutate(value, batch[n])
<1> HIDE DEF Step
<1>0. SUFFICES FiniteNatInductiveDefConclusion(RMTrace(state, batch), state, Step, 0, Len(batch))
    BY DEF Step
<1>1. FiniteNatInductiveDefHypothesis(RMTrace(state, batch), state, Step, 0, Len(batch))
    BY DEF RMTrace, FiniteNatInductiveDefHypothesis, Step
<1>2. Len(batch) \in Nat /\ 0 \in Nat BY SMT
<1> QED BY <1>1, <1>2, FiniteNatInductiveDef

THEOREM RMTraceType == ASSUME NEW state \in RMStates, NEW batch \in Seq(RMMutations)
    PROVE RMTrace(state, batch) \in [0..Len(batch) -> RMStates]
<1> DEFINE Step(value, n) == RMMutate(value, batch[n])
<1> HIDE DEF Step
<1>1. \A value \in RMStates, n \in 1..Len(batch) : Step(value, n) \in RMStates
    BY RMMutateType, SMT DEF Step
<1>3. FiniteNatInductiveDefConclusion(RMTrace(state, batch), state, Step, 0, Len(batch))
    BY RMTraceDefinition DEF Step
<1>2. Len(batch) \in Nat /\ 0 \in Nat BY SMT
<1> QED BY <1>1, <1>2, <1>3, FiniteNatInductiveDefType, Isa

THEOREM RMTraceRecurrence == ASSUME NEW state \in RMStates, NEW batch \in Seq(RMMutations)
    PROVE /\ RMTrace(state, batch)[0] = state
          /\ \A n \in 1..Len(batch) :
                RMTrace(state, batch)[n] = RMMutate(RMTrace(state, batch)[n - 1], batch[n])
BY RMTraceDefinition, SMT DEF FiniteNatInductiveDefConclusion

THEOREM RMApplyType == ASSUME NEW state \in RMStates, NEW batch \in Seq(RMMutations)
    PROVE RMApply(state, batch) \in RMStates
BY RMTraceType, SMT DEF RMApply

THEOREM RMApplyEmpty == ASSUME NEW state \in RMStates PROVE RMApply(state, <<>>) = state
BY RMTraceRecurrence, SMT DEF RMApply


THEOREM RMTracePrefix ==
    ASSUME NEW state \in RMStates, NEW a \in Seq(RMMutations), NEW b \in Seq(RMMutations),
           NEW bound \in 0..Len(a), bound <= Len(b),
           \A i \in 1..bound : a[i] = b[i]
    PROVE \A n \in 0..bound : RMTrace(state, a)[n] = RMTrace(state, b)[n]
<1>1. DEFINE P(n) == n <= bound => RMTrace(state, a)[n] = RMTrace(state, b)[n]
<1> HIDE DEF P
<1>2. P(0) BY RMTraceRecurrence, SMT DEF P
<1>3. ASSUME NEW n \in Nat, P(n) PROVE P(n + 1)
    <2>1. CASE n + 1 <= bound
        <3>1. n <= bound /\ n + 1 \in 1..Len(a) /\ n + 1 \in 1..Len(b) /\ n + 1 \in 1..bound
            BY <1>3, <2>1, SMT
        <3>2. RMTrace(state, a)[n] = RMTrace(state, b)[n]
            BY <1>3, <3>1 DEF P
        <3>3. a[n + 1] = b[n + 1] BY <3>1, SMT
        <3> QED BY <3>1, <3>2, <3>3, RMTraceRecurrence, SMT DEF P
    <2>2. CASE ~(n + 1 <= bound) BY <2>2 DEF P
    <2> QED BY <2>1, <2>2, SMT
<1>4. \A n \in Nat : P(n) BY <1>2, <1>3, NatInduction, Isa
<1> QED BY <1>4, SMT DEF P

THEOREM RMApplyAppend == ASSUME NEW state \in RMStates, NEW batch \in Seq(RMMutations),
    NEW mutation \in RMMutations
    PROVE RMApply(state, Append(batch, mutation)) = RMMutate(RMApply(state, batch), mutation)
<1>1. Append(batch, mutation) \in Seq(RMMutations) /\ Len(Append(batch, mutation)) = Len(batch) + 1 /\
    Append(batch, mutation)[Len(batch) + 1] = mutation /\
    (\A n \in 1..Len(batch) : Append(batch, mutation)[n] = batch[n])
    BY SMT
<1>2. RMTrace(state, Append(batch, mutation))[Len(batch)] = RMTrace(state, batch)[Len(batch)]
    BY <1>1, RMTracePrefix, SMT
<1> QED BY <1>1, <1>2, RMTraceRecurrence, SMT DEF RMApply

THEOREM RMApplyConcatenation == ASSUME NEW state \in RMStates,
    NEW a \in Seq(RMMutations), NEW b \in Seq(RMMutations)
    PROVE RMApply(state, a \o b) = RMApply(RMApply(state, a), b)
<1>1. DEFINE P(batch) == \A initial \in RMStates, prefix \in Seq(RMMutations) :
    RMApply(initial, prefix \o batch) = RMApply(RMApply(initial, prefix), batch)
<1> HIDE DEF P
<1>2. P(<<>>) BY RMApplyEmpty, RMApplyType, SMT DEF P
<1>3. ASSUME NEW batch \in Seq(RMMutations), NEW mutation \in RMMutations, P(batch)
    PROVE P(Append(batch, mutation))
    <2>1. SUFFICES ASSUME NEW initial \in RMStates, NEW prefix \in Seq(RMMutations)
        PROVE RMApply(initial, prefix \o Append(batch, mutation)) =
            RMApply(RMApply(initial, prefix), Append(batch, mutation))
        BY DEF P
    <2>2. <<mutation>> \in Seq(RMMutations)
        BY <1>3, SMT
    <2>3. prefix \o Append(batch, mutation) = Append(prefix \o batch, mutation)
        BY <1>3, <2>1, <2>2, SMT
    <2>4. RMApply(initial, prefix \o batch) = RMApply(RMApply(initial, prefix), batch)
        BY <1>3, <2>1, SMT DEF P
    <2>5. RMApply(initial, Append(prefix \o batch, mutation)) =
        RMMutate(RMApply(initial, prefix \o batch), mutation)
        BY <1>3, <2>1, RMApplyAppend, SMT
    <2>6. RMApply(RMApply(initial, prefix), Append(batch, mutation)) =
        RMMutate(RMApply(RMApply(initial, prefix), batch), mutation)
        BY <1>3, <2>1, RMApplyAppend, RMApplyType, SMT
    <2> QED BY <2>3, <2>4, <2>5, <2>6, SMT
<1>4. \A batch \in Seq(RMMutations) : P(batch)
    BY <1>2, <1>3, RMSequencesInductionAppend, Isa
<1> QED BY <1>4, SMT DEF P

THEOREM RMRawBatchEmpty == RMRawBatch(<<>>)
BY SMT DEF RMRawBatch

THEOREM RMRawBatchConcatenation == ASSUME NEW a \in Seq(RMMutations), NEW b \in Seq(RMMutations),
    RMRawBatch(a), RMRawBatch(b) PROVE RMRawBatch(a \o b)
BY SMT DEF RMRawBatch

THEOREM RMMutateSystem == ASSUME NEW state \in RMStates, NEW mutation \in RMMutations,
    mutation.key \in RMRawKeys, NEW key \in RMSystemKeys
    PROVE RMMutate(state, mutation)[key] = state[key]
BY RMLegalInputs, SMT DEF RMMutate, RMStates, RMMutations

THEOREM RMApplySystem == ASSUME NEW state \in RMStates, NEW batch \in Seq(RMMutations), RMRawBatch(batch)
    PROVE \A key \in RMSystemKeys : RMApply(state, batch)[key] = state[key]
<1>1. DEFINE P(n) == n <= Len(batch) =>
    (\A key \in RMSystemKeys : RMTrace(state, batch)[n][key] = state[key])
<1> HIDE DEF P
<1>2. P(0) BY RMTraceRecurrence, SMT DEF P
<1>3. ASSUME NEW n \in Nat, P(n) PROVE P(n + 1)
    <2>1. CASE n + 1 <= Len(batch)
        <3>1. n \in 0..Len(batch) /\ n + 1 \in 1..Len(batch)
            BY <1>3, <2>1, SMT
        <3>2. RMTrace(state, batch)[n] \in RMStates /\ batch[n + 1] \in RMMutations /\ batch[n + 1].key \in RMRawKeys
            BY <3>1, RMTraceType, SMT DEF RMRawBatch
        <3>3. \A key \in RMSystemKeys : RMTrace(state, batch)[n][key] = state[key]
            BY <1>3, <3>1 DEF P
        <3> QED BY <3>1, <3>2, <3>3, RMTraceRecurrence, RMMutateSystem, SMT DEF P
    <2>2. CASE ~(n + 1 <= Len(batch)) BY <2>2 DEF P
    <2> QED BY <2>1, <2>2, SMT
<1>4. \A n \in Nat : P(n) BY <1>2, <1>3, NatInduction, Isa
<1> QED BY <1>4, SMT DEF P, RMApply
=============================================================================
