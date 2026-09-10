--------------------------- MODULE RawGroupMC ---------------------------
EXTENDS RawGroup
CONSTANT RGMCBarrierKind
RGMCRegions == [i \in RGEntries |-> 100 + i]
RGMCInitial == [k \in RMKeys |-> IF k = 2 THEN 1 ELSE 0]
RGMCBase == [term |-> 0, index |-> 0]
RGMCPositions == [i \in RGEntries |-> [term |-> IF i = 1 THEN 1 ELSE 2, index |-> 2 * i]]
RGMCBadPositions == [i \in RGEntries |-> [term |-> IF i = 2 THEN 1 ELSE 2, index |-> i]]
RGMCWrites == [i \in RGEntries |-> "write"]
RGMCMixed == [i \in RGEntries |-> IF i = 2 THEN "fenced" ELSE "write"]
RGMCFenced == [i \in RGEntries |-> "fenced"]
RGMCBarrier == [i \in RGEntries |-> IF i = 3 THEN RGMCBarrierKind ELSE "write"]
RGMCHidden == [i \in RGEntries |-> IF i = 3 THEN "fenced" ELSE "write"]
RGMCOps == [i \in RGEntries |-> <<[key |-> 1, value |-> IF i = 1 THEN 1 ELSE IF i = 2 THEN 0 ELSE 2]>>]
RGMCHiddenOps == [i \in RGEntries |-> <<[key |-> IF i = 3 THEN 2 ELSE 1, value |-> 2]>>]
RGMCWeights == [i \in RGEntries |-> 1]
RGMCOversize == [i \in RGEntries |-> IF i = 1 THEN 9 ELSE 1]
RGMCByteBound == [i \in RGEntries |-> IF i = 3 THEN 9 ELSE 1]
RGMCSystemRead == [i \in RGEntries |-> 2]
RGMCRawRead == [i \in RGEntries |-> 1]
RGMCAccept == [i \in RGEntries |-> [v \in RMValues \cup {0} |-> TRUE]]
RGMCStale == [i \in RGEntries |-> [v \in RMValues \cup {0} |-> FALSE]]
RGMCLive == RGInit /\ [][RGNext]_rgVars /\ RGFairness
=============================================================================
