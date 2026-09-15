------------------------ MODULE RetentionLedger ------------------------
EXTENDS Naturals
CONSTANTS LOwners, LResources, LSubjects, LClosure, LSubject, LMaxGeneration
ASSUME LInputs == /\ LOwners # {} /\ LResources # {} /\ LSubjects # {}
                  /\ LClosure \in [LOwners -> SUBSET LResources]
                  /\ \A o \in LOwners : LClosure[o] # {}
                  /\ LSubject \in [LOwners -> LSubjects]
                  /\ LMaxGeneration \in Nat /\ LMaxGeneration > 0
\* Owner IDs denote immutable, root-bound descriptors and exact closures.
\* This is the committed-state projection after metadata planning has supplied
\* freshness and the engine has supplied atomic data/position application.
VARIABLES lOwners, lPins, lProtected, lStale
lVars == <<lOwners, lPins, lProtected, lStale>>
LAbsent == [generation |-> 0, phase |-> "absent"]
LRecord(g, p) == [generation |-> g, phase |-> p]
LRecords == [generation : 0..LMaxGeneration,
             phase : {"absent", "held", "published", "quiesced", "released"}]
LInit == /\ lOwners = [o \in LOwners |-> LAbsent]
         /\ lPins = [r \in LResources |-> [o \in LOwners |-> LAbsent]]
         /\ lProtected = {} /\ lStale = FALSE
LPinsAfter(o, record) ==
    [r \in LResources |-> IF r \in LClosure[o]
        THEN [lPins[r] EXCEPT ![o] = record] ELSE lPins[r]]
LSet(o, g, phase) ==
    /\ lOwners' = [lOwners EXCEPT ![o] = LRecord(g, phase)]
    /\ lPins' = LPinsAfter(o, LRecord(g, phase))
LAcquire(o, g) ==
    /\ lOwners[o].phase \in {"absent", "released"}
    /\ g = lOwners[o].generation + 1
    /\ LSet(o, g, "held")
    /\ UNCHANGED <<lProtected, lStale>>
LPublish(o, g) ==
    /\ lOwners[o].phase = "held" /\ g = lOwners[o].generation
    /\ LSet(o, lOwners[o].generation, "published")
    /\ lProtected' = lProtected \cup {<<LSubject[o], r>> : r \in LClosure[o]}
    /\ lStale' = (lStale \/ g # lOwners[o].generation)
LShare(from, to, g) ==
    /\ from # to /\ lOwners[from].phase \in {"held", "published"}
    /\ LSubject[from] = LSubject[to] /\ LClosure[from] = LClosure[to]
    /\ LAcquire(to, g)
LQuiesce(from, to, g) ==
    /\ from # to /\ lOwners[from].phase \in {"held", "published"}
    /\ g = lOwners[from].generation
    /\ lOwners[to].phase = "published"
    /\ LSubject[from] = LSubject[to] /\ LClosure[from] = LClosure[to]
    /\ LSet(from, lOwners[from].generation, "quiesced")
    /\ lStale' = (lStale \/ g # lOwners[from].generation)
    /\ UNCHANGED lProtected
LRelease(o, g) ==
    /\ lOwners[o].phase = "quiesced" /\ g = lOwners[o].generation
    /\ LSet(o, lOwners[o].generation, "released")
    /\ lStale' = (lStale \/ g # lOwners[o].generation)
    /\ UNCHANGED lProtected
LNext == \/ \E o \in LOwners, g \in 1..LMaxGeneration :
                 LAcquire(o,g) \/ LPublish(o,g) \/ LRelease(o,g)
         \/ \E from, to \in LOwners, g \in 1..LMaxGeneration :
                 LShare(from,to,g) \/ LQuiesce(from,to,g)
LSpec == LInit /\ [][LNext]_lVars
LType == /\ lOwners \in [LOwners -> LRecords]
         /\ lPins \in [LResources -> [LOwners -> LRecords]]
         /\ lProtected \subseteq LSubjects \X LResources /\ lStale \in BOOLEAN
LExactLinks == \A o \in LOwners, r \in LResources :
    lPins[r][o] = IF r \in LClosure[o] THEN lOwners[o] ELSE LAbsent
LShapes == \A o \in LOwners :
    (lOwners[o].phase = "absent") <=> (lOwners[o].generation = 0)
LPublicationCovered == \A pair \in lProtected :
    \E o \in LOwners : lOwners[o].phase = "published"
        /\ LSubject[o] = pair[1] /\ pair[2] \in LClosure[o]
LInvariant == LType /\ LExactLinks /\ LShapes /\ LPublicationCovered /\ ~lStale
=============================================================================
