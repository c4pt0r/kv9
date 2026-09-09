----------------------- MODULE ReadyPublicationProof -----------------------
EXTENDS ReadyPublication, TLAPS
THEOREM RPInvariantInit == RPInit => RPInvariant
BY RPLegalInputs, SMT DEF RPInit, RPInvariant, RPUnpublished, RPType, RPPhases, RPCoverage, RPStage

THEOREM RPInvariantStep == ASSUME RPInvariant, [RPNext]_rpVars PROVE RPInvariant'
<1>1. \A e, c \in 0..RPMaxIndex : RPCollect(e, c) => RPInvariant'
    BY SMT DEF RPCollect, RPInvariant, RPUnpublished, RPType, RPPhases, RPCoverage, RPStage
<1>2. RPPersistEntries \/ RPPersistHardState \/ RPPersistLight => RPInvariant'
    BY SMT DEF RPPersistEntries, RPPersistHardState, RPPersistLight, RPInvariant, RPUnpublished, RPType,
       RPPhases, RPCoverage, RPStage
<1>3. \A c \in 0..RPMaxIndex : RPAdvance(c) => RPInvariant'
    BY SMT DEF RPAdvance, RPInvariant, RPUnpublished, RPType, RPPhases, RPCoverage, RPStage
<1>4. RPPublish \/ RPAcknowledge \/ (\E c \in 0..RPMaxIndex : RPApply(c)) => RPInvariant'
    BY SMT DEF RPPublish, RPAcknowledge, RPApply, RPInvariant, RPUnpublished, RPType, RPPhases, RPCoverage,
       RPStage
<1>5. RPFailBefore => RPInvariant'
    BY SMT DEF RPFailBefore, RPPersisting, RPInvariant, RPUnpublished, RPType, RPPhases, RPCoverage, RPStage
<1>6. RPFailAfter => RPInvariant'
    <2>1. SUFFICES ASSUME RPFailAfter PROVE RPInvariant'
        OBVIOUS
    <2>2. /\ rpPhase' = "failed"
           /\ UNCHANGED <<rpApplied, rpDelivered, rpAcknowledged, rpEnd, rpEarly, rpLight, rpPublished>>
        BY <2>1, SMT DEF RPFailAfter
    <2>3. /\ rpDiskCommit' \in rpDiskCommit..rpDiskEnd'
           /\ rpDiskEnd' \in 0..RPMaxIndex
        BY <2>1, SMT DEF RPFailAfter, RPPersisting, RPInvariant, RPType, RPCoverage, RPStage
    <2>4. ~rpPublished
        BY <2>1, SMT DEF RPFailAfter, RPPersisting, RPInvariant, RPUnpublished
    <2> QED BY <2>2, <2>3, <2>4, SMT DEF RPInvariant, RPUnpublished, RPType, RPPhases, RPCoverage, RPStage
<1>7. RPCrash => RPInvariant'
    BY SMT DEF RPCrash, RPInvariant, RPUnpublished, RPType, RPPhases, RPCoverage, RPStage
<1>8. UNCHANGED rpVars => RPInvariant'
    BY SMT DEF RPInvariant, RPUnpublished, RPType, RPCoverage, RPStage, rpVars
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, <1>7, <1>8, SMT DEF RPNext

THEOREM RPCoverageAlways == RPSpec => []RPCoverage
<1>1. RPSpec => []RPInvariant BY RPInvariantInit, RPInvariantStep, PTL DEF RPSpec
<1>2. RPInvariant => RPCoverage BY SMT DEF RPInvariant
<1> QED BY <1>1, <1>2, PTL

THEOREM RPFailedStep ==
    ASSUME RPInvariant, RPNext
    PROVE RPFailedStop
BY SMT DEF RPInvariant, RPUnpublished, RPType, RPCoverage, RPFailedStop, RPNext,
           RPCollect, RPPersistEntries, RPPersistHardState, RPAdvance,
           RPPersistLight, RPPublish, RPApply, RPAcknowledge,
           RPFailBefore, RPFailAfter, RPCrash, RPPersisting

THEOREM RPFailedAlways == RPSpec => [][RPFailedStop]_rpVars
<1>1. RPSpec => []RPInvariant BY RPInvariantInit, RPInvariantStep, PTL DEF RPSpec
<1>2. RPInvariant /\ [RPNext]_rpVars => [RPFailedStop]_rpVars
    BY RPFailedStep, SMT
<1> QED BY <1>1, <1>2, PTL DEF RPSpec
THEOREM RPNoFailedPublicationAlways == RPSpec => []RPNoFailedPublication
<1>1. RPSpec => []RPInvariant BY RPInvariantInit, RPInvariantStep, PTL DEF RPSpec
<1>2. RPInvariant => RPNoFailedPublication BY SMT DEF RPInvariant, RPUnpublished, RPNoFailedPublication
<1> QED BY <1>1, <1>2, PTL
=============================================================================
