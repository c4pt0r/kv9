-------------------------- MODULE NativeBatchMC --------------------------
EXTENDS NativeBatch
NBZeroState == [key \in RMKeys |-> 0]
NBPairs == <<[key |-> 1, value |-> 1], [key |-> 2, value |-> 2], [key |-> 1, value |-> 2]>>
NBKeys == <<2, 1, 2, 1>>
NBAt == <<1, 7>>
NBBackgrounds == {<<mutation>> : mutation \in RMMutations} \cup
    {<<[key |-> 1, value |-> 1], [key |-> 2, value |-> 1]>>}
NBNoSuccess == ~(nbReadPhase = "done" /\ nbReceipt = NBPosition)
NBNoLateEffect == [][crPhase = "unknown" => crApplied' = crApplied]_nbVars
NBNoRetriedSuccess == ~(crSent > 1 /\ nbReceipt = NBPosition)
NBNoCompletedOldView == ~(nbReadPhase = "done" /\ nbView # nbStore /\ nbBg > 0)
NBNoCollectionOverlap == [][(nbReadPhase = "collecting" /\ nbBg' > nbBg)
                            => nbStore' = nbStore]_nbVars
=============================================================================
