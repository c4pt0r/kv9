------------------------- MODULE ReadyPublication -------------------------
EXTENDS Naturals
CONSTANT RPMaxIndex
ASSUME RPLegalInputs == RPMaxIndex \in Nat \ {0}

\* A per-peer publication model. Consensus establishes the incoming commit cuts;
\* successful storage sync and atomic positioned engine writes are assumptions.
VARIABLES rpDiskEnd, rpDiskCommit, rpApplied, rpDelivered, rpAcknowledged,
          rpPhase, rpEnd, rpEarly, rpLight, rpPublished
rpVars == <<rpDiskEnd, rpDiskCommit, rpApplied, rpDelivered, rpAcknowledged,
            rpPhase, rpEnd, rpEarly, rpLight, rpPublished>>
RPPhases == {"idle", "entries", "hardstate", "advance", "light", "publish", "failed"}
RPPersisting == {"entries", "hardstate", "light"}
RPInit ==
    /\ rpDiskEnd = 0 /\ rpDiskCommit = 0
    /\ rpApplied = 0 /\ rpDelivered = 0 /\ rpAcknowledged = 0
    /\ rpPhase = "idle" /\ rpEnd = 0 /\ rpEarly = 0 /\ rpLight = 0 /\ rpPublished = FALSE

RPCollect(e, c) ==
    /\ rpPhase = "idle"
    /\ e \in rpDiskCommit..RPMaxIndex /\ c \in rpDiskCommit..e
    /\ rpEnd' = e /\ rpEarly' = c /\ rpPhase' = "entries" /\ rpPublished' = FALSE
    /\ UNCHANGED <<rpDiskEnd, rpDiskCommit, rpApplied, rpDelivered, rpAcknowledged, rpLight>>
RPPersistEntries ==
    /\ rpPhase = "entries" /\ rpDiskEnd' = rpEnd /\ rpPhase' = "hardstate"
    /\ UNCHANGED <<rpDiskCommit, rpApplied, rpDelivered, rpAcknowledged, rpEnd, rpEarly, rpLight, rpPublished>>
RPPersistHardState ==
    /\ rpPhase = "hardstate" /\ rpDiskCommit' = rpEarly /\ rpPhase' = "advance"
    /\ UNCHANGED <<rpDiskEnd, rpApplied, rpDelivered, rpAcknowledged, rpEnd, rpEarly, rpLight, rpPublished>>
RPAdvance(c) ==
    /\ rpPhase = "advance" /\ c \in rpDiskCommit..rpDiskEnd
    /\ rpLight' = c
    /\ rpPhase' = IF c = rpDiskCommit THEN "publish" ELSE "light"
    /\ UNCHANGED <<rpDiskEnd, rpDiskCommit, rpApplied, rpDelivered, rpAcknowledged, rpEnd, rpEarly, rpPublished>>
RPPersistLight ==
    /\ rpPhase = "light" /\ rpDiskCommit' = rpLight /\ rpPhase' = "publish"
    /\ UNCHANGED <<rpDiskEnd, rpApplied, rpDelivered, rpAcknowledged, rpEnd, rpEarly, rpLight, rpPublished>>
RPPublish ==
    /\ rpPhase = "publish" /\ rpDelivered' = rpLight /\ rpPhase' = "idle" /\ rpPublished' = TRUE
    /\ UNCHANGED <<rpDiskEnd, rpDiskCommit, rpApplied, rpAcknowledged, rpEnd, rpEarly, rpLight>>
RPApply(c) ==
    /\ rpPhase # "failed" /\ c \in (rpApplied + 1)..rpDelivered
    /\ rpApplied' = c
    /\ UNCHANGED <<rpDiskEnd, rpDiskCommit, rpDelivered, rpAcknowledged, rpPhase, rpEnd, rpEarly, rpLight, rpPublished>>
RPAcknowledge ==
    /\ rpPhase # "failed" /\ rpAcknowledged < rpApplied /\ rpAcknowledged' = rpApplied
    /\ UNCHANGED <<rpDiskEnd, rpDiskCommit, rpApplied, rpDelivered, rpPhase, rpEnd, rpEarly, rpLight, rpPublished>>
RPFailBefore ==
    /\ rpPhase \in RPPersisting /\ rpPhase' = "failed"
    /\ UNCHANGED <<rpDiskEnd, rpDiskCommit, rpApplied, rpDelivered, rpAcknowledged, rpEnd, rpEarly, rpLight, rpPublished>>
RPFailAfter ==
    /\ rpPhase \in RPPersisting /\ rpPhase' = "failed"
    \* An entry batch may leave any retained partial prefix. Its committed
    \* prefix is preserved by the upstream append contract.
    /\ IF rpPhase = "entries"
          THEN rpDiskEnd' \in rpDiskCommit..(IF rpDiskEnd > rpEnd THEN rpDiskEnd ELSE rpEnd)
          ELSE rpDiskEnd' = rpDiskEnd
    /\ rpDiskCommit' = CASE rpPhase = "hardstate" -> rpEarly
                         [] rpPhase = "light" -> rpLight
                         [] OTHER -> rpDiskCommit
    /\ UNCHANGED <<rpApplied, rpDelivered, rpAcknowledged, rpEnd, rpEarly, rpLight, rpPublished>>
RPCrash ==
    /\ rpPhase' = "idle" /\ rpDelivered' = rpApplied /\ rpPublished' = FALSE
    /\ rpEnd' = rpDiskEnd /\ rpEarly' = rpDiskCommit /\ rpLight' = rpDiskCommit
    /\ UNCHANGED <<rpDiskEnd, rpDiskCommit, rpApplied, rpAcknowledged>>
RPNext ==
    \/ \E e, c \in 0..RPMaxIndex : RPCollect(e, c)
    \/ RPPersistEntries \/ RPPersistHardState
    \/ \E c \in 0..RPMaxIndex : RPAdvance(c)
    \/ RPPersistLight \/ RPPublish
    \/ \E c \in 0..RPMaxIndex : RPApply(c)
    \/ RPAcknowledge \/ RPFailBefore \/ RPFailAfter \/ RPCrash
RPSpec == RPInit /\ [][RPNext]_rpVars

RPType ==
    /\ RPMaxIndex \in Nat \ {0}
    /\ rpPhase \in RPPhases /\ rpPublished \in BOOLEAN
    /\ \A x \in {rpDiskEnd, rpDiskCommit, rpApplied, rpDelivered, rpAcknowledged,
                  rpEnd, rpEarly, rpLight} : x \in 0..RPMaxIndex
RPCoverage == rpAcknowledged <= rpApplied /\ rpApplied <= rpDelivered
               /\ rpDelivered <= rpDiskCommit /\ rpDiskCommit <= rpDiskEnd
RPStage ==
    /\ rpPhase = "entries" => rpDiskCommit <= rpEarly /\ rpEarly <= rpEnd
    /\ rpPhase = "hardstate" => rpDiskCommit <= rpEarly /\ rpEarly <= rpDiskEnd
    /\ rpPhase = "advance" => rpEarly = rpDiskCommit
    /\ rpPhase = "light" => rpDiskCommit <= rpLight /\ rpLight <= rpDiskEnd
    /\ rpPhase = "publish" => rpLight = rpDiskCommit
RPNoFailedPublication == rpPhase = "failed" => ~rpPublished
RPUnpublished == rpPhase # "idle" => ~rpPublished
RPInvariant == RPType /\ RPCoverage /\ RPStage /\ RPUnpublished
RPFailedStop == rpPhase = "failed" =>
    /\ UNCHANGED <<rpApplied, rpAcknowledged, rpPublished>> /\ rpDelivered' <= rpDelivered
RPFatalFreeze == [][RPFailedStop]_rpVars
=============================================================================
