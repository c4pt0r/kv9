---------------------- MODULE NativeBatchAlgebraProof ----------------------
EXTENDS RawMutationProof

THEOREM NBApplyUntouched ==
    ASSUME NEW state \in RMStates, NEW batch \in Seq(RMMutations), NEW key \in RMKeys,
        \A i \in 1..Len(batch) : batch[i].key # key
    PROVE RMApply(state, batch)[key] = state[key]
<1> DEFINE P(n) == n <= Len(batch) => RMTrace(state, batch)[n][key] = state[key]
<1> HIDE DEF P
<1>1. P(0) BY RMTraceRecurrence, SMT DEF P
<1>2. ASSUME NEW n \in Nat, P(n) PROVE P(n + 1)
    <2>1. CASE n + 1 <= Len(batch)
        <3>1. n \in 0..Len(batch) /\ n + 1 \in 1..Len(batch)
            BY <1>2, <2>1, SMT
        <3>2. RMTrace(state, batch)[n] \in RMStates /\ batch[n + 1] \in RMMutations /\
            batch[n + 1].key # key
            BY <3>1, RMTraceType, SMT
        <3> QED BY <1>2, <3>1, <3>2, RMTraceRecurrence, SMT DEF P, RMMutate, RMStates
    <2>2. CASE ~(n + 1 <= Len(batch)) BY <2>2 DEF P
    <2> QED BY <2>1, <2>2, SMT
<1>3. \A n \in Nat : P(n) BY <1>1, <1>2, NatInduction, Isa
<1> QED BY <1>3, SMT DEF P, RMApply

THEOREM NBApplyLastPair ==
    ASSUME NEW state \in RMStates, NEW batch \in Seq(RMMutations), NEW key \in RMKeys,
        NEW j \in 1..Len(batch), batch[j].key = key,
        \A i \in (j + 1)..Len(batch) : batch[i].key # key
    PROVE RMApply(state, batch)[key] = batch[j].value
<1> DEFINE P(n) == j <= n /\ n <= Len(batch) => RMTrace(state, batch)[n][key] = batch[j].value
<1> HIDE DEF P
<1>1. P(0) BY SMT DEF P
<1>2. ASSUME NEW n \in Nat, P(n) PROVE P(n + 1)
    <2>1. CASE j <= n + 1 /\ n + 1 <= Len(batch)
        <3>1. n \in 0..Len(batch) /\ n + 1 \in 1..Len(batch) /\
            RMTrace(state, batch)[n] \in RMStates /\ batch[n + 1] \in RMMutations
            BY <1>2, <2>1, RMTraceType, SMT
        <3>2. CASE n + 1 = j
            BY <3>1, <3>2, RMTraceRecurrence, SMT DEF P, RMMutate, RMStates
        <3>3. CASE n + 1 # j
            <4>1. j <= n /\ n + 1 \in (j + 1)..Len(batch) /\ batch[n + 1].key # key
                BY <1>2, <2>1, <3>3, SMT
            <4> QED BY <1>2, <3>1, <4>1, RMTraceRecurrence, SMT DEF P, RMMutate, RMStates
        <3> QED BY <3>2, <3>3, SMT
    <2>2. CASE ~(j <= n + 1 /\ n + 1 <= Len(batch)) BY <2>2 DEF P
    <2> QED BY <2>1, <2>2, SMT
<1>3. \A n \in Nat : P(n) BY <1>1, <1>2, NatInduction, Isa
<1> QED BY <1>3, SMT DEF P, RMApply
=============================================================================
