--------------------------- MODULE NativeBatch ---------------------------
EXTENDS RawMutation, ClientRetry
CONSTANTS NBInitial, NBWriteBatch, NBReadKeys, NBPosition, NBBgLimit, NBBackgroundBatches
ASSUME NBLegalInputs ==
    /\ NBInitial \in RMStates
    /\ NBWriteBatch \in Seq(RMMutations) /\ Len(NBWriteBatch) > 0
    /\ RMRawBatch(NBWriteBatch)
    /\ \A i \in 1..Len(NBWriteBatch) : NBWriteBatch[i].value \in RMValues
    /\ NBReadKeys \in Seq(RMKeys) /\ Len(NBReadKeys) > 0
    /\ \A i \in 1..Len(NBReadKeys) : NBReadKeys[i] \in RMRawKeys
    /\ NBPosition \in (Nat \ {0}) \X (Nat \ {0}) /\ NBBgLimit \in Nat
    /\ NBBackgroundBatches \subseteq Seq(RMMutations)

\* One immutable native write and one native read, with arbitrary finite
\* intervening atomic point or batch commands. Physical key identity includes keyspace.
\* The store represents committed state: Raft authority, epoch adjudication,
\* and atomic positioned application are explicit lower-layer premises.
\* A captured read view stays stable while the live store keeps changing.
VARIABLES nbStore, nbCommand, nbBefore, nbAfter, nbReceipt,
          nbReadPhase, nbView, nbResult, nbBg
nbLocal == <<nbStore, nbCommand, nbBefore, nbAfter, nbReceipt,
             nbReadPhase, nbView, nbResult, nbBg>>
nbVars == <<crVars, nbLocal>>
NBProjection(view, keys) == [i \in 1..Len(keys) |-> view[keys[i]]]
NBInit ==
    /\ CRInit /\ nbStore = NBInitial /\ nbCommand = <<>>
    /\ nbBefore = NBInitial /\ nbAfter = NBInitial /\ nbReceipt = <<0, 0>>
    /\ nbReadPhase = "idle" /\ nbView = NBInitial /\ nbResult = <<>> /\ nbBg = 0

NBDispatch ==
    /\ CRDispatch /\ nbCommand' = NBWriteBatch
    /\ UNCHANGED <<nbStore, nbBefore, nbAfter, nbReceipt, nbReadPhase, nbView, nbResult, nbBg>>
NBEffect(i) ==
    /\ CREffect(i)
    /\ nbBefore' = nbStore /\ nbAfter' = RMApply(nbStore, nbCommand)
    /\ nbStore' = RMApply(nbStore, nbCommand)
    /\ UNCHANGED <<nbCommand, nbReceipt, nbReadPhase, nbView, nbResult, nbBg>>
NBSuccess ==
    /\ CRSuccess /\ nbReceipt' = NBPosition
    /\ UNCHANGED <<nbStore, nbCommand, nbBefore, nbAfter, nbReadPhase, nbView, nbResult, nbBg>>
NBControl ==
    /\ (CRRefuse \/ CRUnknown \/ CRStop \/ CRTick)
    /\ UNCHANGED nbLocal
NBBackground(batch) ==
    /\ batch \in NBBackgroundBatches /\ nbBg < NBBgLimit /\ nbBg' = nbBg + 1
    /\ nbStore' = RMApply(nbStore, batch)
    /\ UNCHANGED <<crVars, nbCommand, nbBefore, nbAfter, nbReceipt, nbReadPhase, nbView, nbResult>>
NBReadStart ==
    /\ nbReadPhase = "idle" /\ nbReadPhase' = "waiting"
    /\ UNCHANGED <<crVars, nbStore, nbCommand, nbBefore, nbAfter, nbReceipt, nbView, nbResult, nbBg>>
NBReadCapture ==
    /\ nbReadPhase = "waiting" /\ nbReadPhase' = "collecting"
    /\ nbView' = nbStore /\ nbResult' = <<>>
    /\ UNCHANGED <<crVars, nbStore, nbCommand, nbBefore, nbAfter, nbReceipt, nbBg>>
NBReadItem ==
    /\ nbReadPhase = "collecting" /\ Len(nbResult) < Len(NBReadKeys)
    /\ nbResult' = Append(nbResult, nbView[NBReadKeys[Len(nbResult) + 1]])
    /\ UNCHANGED <<crVars, nbStore, nbCommand, nbBefore, nbAfter, nbReceipt, nbReadPhase, nbView, nbBg>>
NBReadPublish ==
    /\ nbReadPhase = "collecting" /\ Len(nbResult) = Len(NBReadKeys)
    /\ nbReadPhase' = "done"
    /\ UNCHANGED <<crVars, nbStore, nbCommand, nbBefore, nbAfter, nbReceipt, nbView, nbResult, nbBg>>
NBReadFail ==
    /\ nbReadPhase \in {"waiting", "collecting"} /\ nbReadPhase' = "failed"
    /\ UNCHANGED <<crVars, nbStore, nbCommand, nbBefore, nbAfter, nbReceipt, nbView, nbResult, nbBg>>
NBQuiesce == UNCHANGED nbVars
NBNext ==
    \/ NBDispatch \/ (\E i \in 1..CRMaxAttempts : NBEffect(i)) \/ NBSuccess \/ NBControl
    \/ (\E batch \in NBBackgroundBatches : NBBackground(batch))
    \/ NBReadStart \/ NBReadCapture \/ NBReadItem \/ NBReadPublish \/ NBReadFail \/ NBQuiesce
NBSpec == NBInit /\ [][NBNext]_nbVars

NBType ==
    /\ nbStore \in RMStates /\ nbBefore \in RMStates /\ nbAfter \in RMStates /\ nbView \in RMStates
    /\ nbCommand \in Seq(RMMutations) /\ nbReceipt \in {<<0, 0>>, NBPosition}
    /\ nbReadPhase \in {"idle", "waiting", "collecting", "done", "failed"}
    /\ nbResult \in Seq(RMValues \cup {0}) /\ Len(nbResult) <= Len(NBReadKeys)
    /\ nbBg \in 0..NBBgLimit
NBCommandBinding == IF crSent = 0 THEN nbCommand = <<>> ELSE nbCommand = NBWriteBatch
NBReadPrefix == \A i \in 1..Len(nbResult) : nbResult[i] = nbView[NBReadKeys[i]]
NBReadLength == nbReadPhase = "done" => Len(nbResult) = Len(NBReadKeys)
NBWriteBinding == crApplied # {} => nbAfter = RMApply(nbBefore, NBWriteBatch)
NBAckBinding == nbReceipt # <<0, 0>> =>
    crPhase = "terminal" /\ crApplied # {} /\ nbReceipt = NBPosition
NBInvariant == CRInvariant /\ NBType /\ NBCommandBinding /\ NBReadPrefix /\
    NBReadLength /\ NBWriteBinding /\ NBAckBinding
NBReadAtomic == nbReadPhase = "done" => nbResult = NBProjection(nbView, NBReadKeys)
NBReadDuplicates == nbReadPhase = "done" =>
    \A i, j \in 1..Len(NBReadKeys) : NBReadKeys[i] = NBReadKeys[j] => nbResult[i] = nbResult[j]
NBWholeEffectStep == crApplied' # crApplied =>
    nbBefore' = nbStore /\ nbAfter' = RMApply(nbStore, NBWriteBatch) /\
    nbStore' = RMApply(nbStore, NBWriteBatch)
NBWholeEffectSafety == [][NBWholeEffectStep]_nbVars
NBOrderedImage ==
    crApplied # {} =>
    \A key \in RMKeys :
        /\ ((\A i \in 1..Len(NBWriteBatch) : NBWriteBatch[i].key # key) => nbAfter[key] = nbBefore[key])
        /\ (\A j \in 1..Len(NBWriteBatch) :
            (NBWriteBatch[j].key = key /\ (\A i \in (j + 1)..Len(NBWriteBatch) : NBWriteBatch[i].key # key)) =>
                nbAfter[key] = NBWriteBatch[j].value)
NBSystemImage == crApplied # {} =>
    \A key \in RMSystemKeys : nbAfter[key] = nbBefore[key]
=============================================================================
