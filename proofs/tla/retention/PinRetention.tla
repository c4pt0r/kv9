----------------------------- MODULE PinRetention -----------------------------
EXTENDS Naturals
CONSTANT PinOwners, PinMaxGeneration
ASSUME PinInputs == PinOwners # {} /\ PinMaxGeneration \in Nat \ {0}
\* One resource instance under an enclosing committed ledger. Owner descriptors,
\* root/content validation, filesystem durability and quiescence certification
\* are interface premises; this component grants no physical delete capability.
VARIABLES pinPhase, pinGeneration, pinRetired, pinDeleted, pinLastRelease
pinVars == <<pinPhase, pinGeneration, pinRetired, pinDeleted, pinLastRelease>>
PinPhases == {"Absent", "Held", "Published", "Quiesced", "Released"}
PinInactive == {"Absent", "Released"}
PinActive == {"Held", "Published"}
PinInit == /\ pinPhase = [o \in PinOwners |-> "Absent"]
           /\ pinGeneration = [o \in PinOwners |-> 0]
           /\ pinRetired = FALSE /\ pinDeleted = FALSE
           /\ pinLastRelease = <<0, 0>>
PinAcquireGuard(o, g) == /\ ~pinRetired
                        /\ pinPhase[o] \in PinInactive
                        /\ g = pinGeneration[o] + 1
                        /\ g <= PinMaxGeneration
PinAcquire(o, g) == /\ PinAcquireGuard(o, g)
                   /\ pinPhase' = [pinPhase EXCEPT ![o] = "Held"]
                   /\ pinGeneration' = [pinGeneration EXCEPT ![o] = g]
                   /\ UNCHANGED <<pinRetired, pinDeleted, pinLastRelease>>
PinPublish(o, g) == /\ pinPhase[o] = "Held" /\ g = pinGeneration[o]
                   /\ pinPhase' = [pinPhase EXCEPT ![o] = "Published"]
                   /\ UNCHANGED <<pinGeneration, pinRetired, pinDeleted, pinLastRelease>>
PinQuiesce(o, g) == /\ pinPhase[o] \in PinActive /\ g = pinGeneration[o]
                   /\ pinPhase' = [pinPhase EXCEPT ![o] = "Quiesced"]
                   /\ UNCHANGED <<pinGeneration, pinRetired, pinDeleted, pinLastRelease>>
PinRelease(o, g) == /\ pinPhase[o] = "Quiesced" /\ g = pinGeneration[o]
                   /\ pinPhase' = [pinPhase EXCEPT ![o] = "Released"]
                   /\ pinLastRelease' = <<g, pinGeneration[o]>>
                   /\ UNCHANGED <<pinGeneration, pinRetired, pinDeleted>>
PinShare(source, sg, target, tg) ==
    /\ source # target /\ pinPhase[source] \in PinActive
    /\ sg = pinGeneration[source] /\ PinAcquire(target, tg)
PinRetire == /\ ~pinRetired
             /\ \A o \in PinOwners : pinPhase[o] \in PinInactive
             /\ pinRetired' = TRUE
             /\ UNCHANGED <<pinPhase, pinGeneration, pinDeleted, pinLastRelease>>
\* Ghost physical execution may be delayed arbitrarily. The Rust component
\* represents only retirement; an external durable executor must justify this step.
PinDelete == /\ pinRetired /\ ~pinDeleted /\ pinDeleted' = TRUE
             /\ UNCHANGED <<pinPhase, pinGeneration, pinRetired, pinLastRelease>>
PinNext == (\E o \in PinOwners, g \in 1..PinMaxGeneration :
               PinAcquire(o, g) \/ PinPublish(o, g) \/ PinQuiesce(o, g) \/ PinRelease(o, g))
           \/ (\E s, t \in PinOwners, sg, tg \in 1..PinMaxGeneration : PinShare(s, sg, t, tg))
           \/ PinRetire \/ PinDelete
PinSpec == PinInit /\ [][PinNext]_pinVars
PinType == /\ pinPhase \in [PinOwners -> PinPhases]
           /\ pinGeneration \in [PinOwners -> 0..PinMaxGeneration]
           /\ pinRetired \in BOOLEAN /\ pinDeleted \in BOOLEAN
           /\ pinLastRelease \in [1..2 -> 0..PinMaxGeneration]
PinBinding == \A o \in PinOwners : (pinPhase[o] = "Absent" <=> pinGeneration[o] = 0)
PinRetiredUnowned == pinRetired => \A o \in PinOwners : pinPhase[o] \in PinInactive
PinDeletion == pinDeleted => pinRetired
PinReleaseIdentity == pinLastRelease[1] = pinLastRelease[2]
PinInvariant == PinType /\ PinBinding /\ PinRetiredUnowned /\ PinDeletion /\ PinReleaseIdentity
PinFairSpec == PinSpec /\ WF_pinVars(PinDelete)
PinDeleteProgress == pinRetired ~> pinDeleted
=============================================================================
