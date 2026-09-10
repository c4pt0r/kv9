------------------------- MODULE RaftOutboundMC -------------------------
EXTENDS RaftOutbound
ROSameAddress == [d \in RODestinations |-> IF d = 2 THEN 2 ELSE 1]
ROSameSession == [i \in ROItems |-> 1]
ROStaleSuffix == [i \in ROItems |-> IF i = 1 THEN 1 ELSE 3]
ROSmallWeights == [i \in ROItems |-> IF i = 1 THEN 1 ELSE 0]
ROBigWeights == [i \in ROItems |-> IF i = 1 THEN 1 ELSE 4]
ROUnitWeights == [i \in ROItems |-> 1]
ROMCLive == ROInit /\ [][RONext]_roVars /\ ROFairness
=============================================================================
