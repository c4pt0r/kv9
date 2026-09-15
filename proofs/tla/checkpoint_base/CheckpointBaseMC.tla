------------------------- MODULE CheckpointBaseMC -------------------------
EXTENDS CheckpointBase
Roots == [i \in 0..CBMax |-> CBExpectedRoot]
RepairedRoots == [i \in 0..CBMax |-> IF i = 0 THEN CBExpectedRoot + 1 ELSE CBExpectedRoot]
Epochs == [i \in 0..CBMax |-> i + 1]
Valid == [i \in 0..CBMax |-> TRUE]
RepairedSchema == [i \in 0..CBMax |-> i # 0]
=============================================================================
