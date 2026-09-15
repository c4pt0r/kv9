-------------------------- MODULE PinRetentionProof --------------------------
EXTENDS PinRetention, TLAPS
THEOREM PinInvariantInit == PinInit => PinInvariant
BY PinInputs, SMT DEF PinInit, PinInvariant, PinType, PinBinding,
   PinRetiredUnowned, PinDeletion, PinReleaseIdentity, PinPhases, PinInactive
THEOREM PinInvariantStep == ASSUME PinInvariant, [PinNext]_pinVars PROVE PinInvariant'
<1>1. \A o \in PinOwners, g \in 1..PinMaxGeneration : PinAcquire(o, g) => PinInvariant'
    BY PinInputs, SMT DEF PinAcquire, PinAcquireGuard, PinInvariant, PinType,
       PinBinding, PinRetiredUnowned, PinDeletion, PinReleaseIdentity, PinPhases, PinInactive
<1>2. \A o \in PinOwners, g \in 1..PinMaxGeneration : PinPublish(o, g) => PinInvariant'
    BY PinInputs, SMT DEF PinPublish, PinInvariant, PinType, PinBinding,
       PinRetiredUnowned, PinDeletion, PinReleaseIdentity, PinPhases, PinInactive
<1>3. \A o \in PinOwners, g \in 1..PinMaxGeneration : PinQuiesce(o, g) => PinInvariant'
    BY PinInputs, SMT DEF PinQuiesce, PinInvariant, PinType, PinBinding,
       PinRetiredUnowned, PinDeletion, PinReleaseIdentity, PinPhases, PinInactive, PinActive
<1>4. \A o \in PinOwners, g \in 1..PinMaxGeneration : PinRelease(o, g) => PinInvariant'
    BY PinInputs, SMT DEF PinRelease, PinInvariant, PinType, PinBinding,
       PinRetiredUnowned, PinDeletion, PinReleaseIdentity, PinPhases, PinInactive
<1>5. \A s, t \in PinOwners, sg, tg \in 1..PinMaxGeneration : PinShare(s, sg, t, tg) => PinInvariant'
    BY <1>1 DEF PinShare
<1>6. PinRetire \/ PinDelete \/ UNCHANGED pinVars => PinInvariant'
    BY PinInputs, SMT DEF PinRetire, PinDelete, PinInvariant, PinType, PinBinding,
       PinRetiredUnowned, PinDeletion, PinReleaseIdentity, PinPhases, PinInactive, pinVars
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, SMT DEF PinNext
THEOREM PinInvariantAlways == PinSpec => []PinInvariant
BY PinInvariantInit, PinInvariantStep, PTL DEF PinSpec
THEOREM PinRetirementPermanent ==
    ASSUME PinInvariant, [PinNext]_pinVars PROVE pinRetired => pinRetired'
BY SMT DEF PinNext, PinAcquire, PinAcquireGuard, PinPublish, PinQuiesce,
   PinRelease, PinShare, PinRetire, PinDelete, pinVars
THEOREM PinPublicationGuard ==
    ASSUME NEW o \in PinOwners, NEW g \in 1..PinMaxGeneration, PinInvariant, PinPublish(o, g)
    PROVE pinPhase[o] = "Held" /\ g = pinGeneration[o] /\ ~pinRetired
BY SMT DEF PinPublish, PinInvariant, PinRetiredUnowned, PinInactive
THEOREM PinShareOverlap ==
    ASSUME NEW s \in PinOwners, NEW t \in PinOwners,
           NEW sg \in 1..PinMaxGeneration, NEW tg \in 1..PinMaxGeneration,
           PinInvariant, PinShare(s, sg, t, tg)
    PROVE pinPhase'[s] = pinPhase[s] /\ pinPhase'[s] \in PinActive /\ pinPhase'[t] = "Held"
BY SMT DEF PinShare, PinAcquire, PinInvariant, PinType
THEOREM PinReleaseFence ==
    ASSUME NEW o \in PinOwners, NEW g \in 1..PinMaxGeneration, PinInvariant, PinRelease(o, g)
    PROVE g = pinGeneration[o] /\ pinGeneration' = pinGeneration
          /\ \A other \in PinOwners \ {o} : pinPhase'[other] = pinPhase[other]
BY SMT DEF PinRelease, PinInvariant, PinType
THEOREM PinGenerationMonotonic ==
    ASSUME PinInvariant, [PinNext]_pinVars
    PROVE \A o \in PinOwners : pinGeneration'[o] >= pinGeneration[o]
BY SMT DEF PinNext, PinAcquire, PinAcquireGuard, PinPublish, PinQuiesce,
   PinRelease, PinShare, PinRetire, PinDelete, PinInvariant, PinType, pinVars
THEOREM PinDeletedUnreferenced ==
    PinInvariant => (pinDeleted => \A o \in PinOwners : pinPhase[o] \in PinInactive)
BY DEF PinInvariant, PinDeletion, PinRetiredUnowned
THEOREM PinRetiredDeletionProgress == PinFairSpec => PinDeleteProgress
<1>1. PinInvariant /\ pinRetired /\ ~pinDeleted /\ [PinNext]_pinVars =>
          (pinRetired /\ ~pinDeleted)' \/ pinDeleted'
    BY PinRetirementPermanent, SMT
<1>2. PinInvariant /\ pinRetired /\ ~pinDeleted /\ PinDelete => pinDeleted'
    BY DEF PinDelete
<1>3. PinInvariant /\ pinRetired /\ ~pinDeleted => ENABLED <<PinDelete>>_pinVars
    BY ExpandENABLED, SMT DEF PinDelete, pinVars
<1>4. PinFairSpec => ((pinRetired /\ ~pinDeleted) ~> pinDeleted)
    BY PinInvariantAlways, <1>1, <1>2, <1>3, PTL DEF PinFairSpec, PinSpec
<1> QED BY <1>4, PTL DEF PinDeleteProgress
=============================================================================
