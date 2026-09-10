--------------------------- MODULE RawMutation ---------------------------
EXTENDS Naturals, Sequences
CONSTANTS RMKeys, RMRawKeys, RMSystemKeys, RMValues
ASSUME RMLegalInputs ==
    /\ RMKeys # {} /\ RMKeys \subseteq Nat \ {0}
    /\ RMRawKeys \subseteq RMKeys /\ RMSystemKeys \subseteq RMKeys
    /\ RMRawKeys \cap RMSystemKeys = {}
    /\ RMValues # {} /\ RMValues \subseteq Nat \ {0}
\* Key identities include the physical CF/mode/keyspace classification.
\* Value zero denotes absence, so deletion and an ordinary put remain ordered.
RMStates == [RMKeys -> RMValues \cup {0}]
RMMutations == [key : RMKeys, value : RMValues \cup {0}]
RMMutate(state, mutation) == [state EXCEPT ![mutation.key] = mutation.value]
RMTrace(state, batch) ==
    LET trace[n \in 0..Len(batch)] ==
        IF n = 0 THEN state ELSE RMMutate(trace[n - 1], batch[n])
    IN trace
RMApply(state, batch) == RMTrace(state, batch)[Len(batch)]
RMRawBatch(batch) == \A i \in 1..Len(batch) : batch[i].key \in RMRawKeys
=============================================================================
