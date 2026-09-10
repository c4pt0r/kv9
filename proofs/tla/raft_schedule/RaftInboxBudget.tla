------------------------- MODULE RaftInboxBudget -------------------------
EXTENDS Naturals
CONSTANTS RBMaxMessages, RBMaxBytes, RBDrainMessages, RBDrainBytes
ASSUME RBLegalInputs ==
    /\ RBMaxMessages \in Nat \ {0} /\ RBMaxBytes \in Nat \ {0}
    /\ RBDrainMessages \in 1..RBMaxMessages /\ RBDrainBytes \in Nat \ {0}
\* Counter refinement of one locked VecDeque. A pop weight is that of the
\* actual front element; container FIFO and exact encoded-weight ledger are
\* premises, not a proof of Rust's container or protobuf implementation.
VARIABLES rbCount, rbBytes, rbPhase, rbInitialCount, rbInitialBytes, rbRemovedCount, rbRemovedBytes
rbVars == <<rbCount, rbBytes, rbPhase, rbInitialCount, rbInitialBytes, rbRemovedCount, rbRemovedBytes>>
RBInit ==
    /\ rbCount = 0 /\ rbBytes = 0 /\ rbPhase = "idle"
    /\ rbInitialCount = 0 /\ rbInitialBytes = 0 /\ rbRemovedCount = 0 /\ rbRemovedBytes = 0
RBAdmit(w) ==
    /\ rbPhase = "idle" /\ w \in Nat
    /\ rbCount < RBMaxMessages /\ w <= RBMaxBytes - rbBytes
    /\ rbCount' = rbCount + 1 /\ rbBytes' = rbBytes + w
    /\ UNCHANGED <<rbPhase, rbInitialCount, rbInitialBytes, rbRemovedCount, rbRemovedBytes>>
RBBegin ==
    /\ rbPhase = "idle" /\ rbPhase' = "drain"
    /\ rbInitialCount' = rbCount /\ rbInitialBytes' = rbBytes
    /\ rbRemovedCount' = 0 /\ rbRemovedBytes' = 0
    /\ UNCHANGED <<rbCount, rbBytes>>
RBCanPop == rbCount > 0 /\ rbRemovedCount < RBDrainMessages /\ rbRemovedBytes < RBDrainBytes
RBPop(w) ==
    /\ rbPhase = "drain" /\ RBCanPop /\ w \in 0..rbBytes
    /\ (rbCount = 1 => w = rbBytes)
    /\ rbCount' = rbCount - 1 /\ rbBytes' = rbBytes - w
    /\ rbRemovedCount' = rbRemovedCount + 1 /\ rbRemovedBytes' = rbRemovedBytes + w
    /\ UNCHANGED <<rbPhase, rbInitialCount, rbInitialBytes>>
RBFinish == rbPhase = "drain" /\ ~RBCanPop /\ rbPhase' = "idle"
    /\ UNCHANGED <<rbCount, rbBytes, rbInitialCount, rbInitialBytes, rbRemovedCount, rbRemovedBytes>>
RBQuiesce == UNCHANGED rbVars
RBNext == (\E w \in 0..RBMaxBytes : RBAdmit(w) \/ RBPop(w)) \/ RBBegin \/ RBFinish \/ RBQuiesce
RBSpec == RBInit /\ [][RBNext]_rbVars
RBType ==
    /\ rbCount \in Nat /\ rbBytes \in Nat /\ rbPhase \in {"idle", "drain"}
    /\ rbInitialCount \in Nat /\ rbInitialBytes \in Nat
    /\ rbRemovedCount \in Nat /\ rbRemovedBytes \in Nat
RBInvariant ==
    /\ RBType /\ rbCount <= RBMaxMessages /\ rbBytes <= RBMaxBytes
    /\ (rbCount = 0 => rbBytes = 0)
    /\ rbInitialCount <= RBMaxMessages /\ rbInitialBytes <= RBMaxBytes
    /\ rbRemovedCount <= RBDrainMessages /\ rbRemovedCount <= rbInitialCount
    /\ rbRemovedBytes <= rbInitialBytes /\ rbRemovedBytes < RBDrainBytes + RBMaxBytes
    /\ (rbPhase = "drain" =>
        rbCount + rbRemovedCount = rbInitialCount /\ rbBytes + rbRemovedBytes = rbInitialBytes)
RBBoundedPop == rbPhase = "drain" /\ rbPhase' = "drain" /\ rbRemovedCount' # rbRemovedCount =>
    rbRemovedCount' = rbRemovedCount + 1 /\ rbCount' = rbCount - 1 /\ rbRemovedBytes < RBDrainBytes
RBSafety == [][RBBoundedPop]_rbVars
RBNoZeroWeightWitness == ~(rbCount > 0 /\ rbBytes = 0)
RBNoOvershootWitness == rbRemovedBytes <= RBDrainBytes
RBNoRetainedWitness == ~(rbPhase = "idle" /\ rbCount > 0 /\ rbRemovedCount > 0)
=============================================================================
