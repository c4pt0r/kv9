----------------------- MODULE CheckpointPublicationMC -----------------------
EXTENDS CheckpointPublication
Images == [i \in 1..CPLimit |-> IF i = 2 THEN 1 ELSE 2]
Preds == [i \in 1..CPLimit |-> IF i <= 3 THEN 0 ELSE 1]
Cuts == [m \in CPImages |-> 1]
Terms == [i \in 1..CPLimit |-> IF i = 1 THEN 1 ELSE 2]
SkewedTerms == [i \in 1..CPLimit |-> IF i = 2 THEN 3 ELSE Terms[i]]
WrongImages == [i \in 1..CPLimit |-> 2]
Fresh == [i \in 1..CPLimit |-> TRUE]
=============================================================================
