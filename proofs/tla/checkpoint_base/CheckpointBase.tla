--------------------------- MODULE CheckpointBase ---------------------------
EXTENDS Naturals
CONSTANTS CBMax, CBExpectedRoot, CBImageRoot, CBImageEpoch, CBImageValid,
          CBPublication
ASSUME CBInputs == /\ CBMax \in Nat \ {0} /\ CBExpectedRoot \in Nat
                   /\ CBImageRoot \in [0..CBMax -> Nat]
                   /\ CBImageEpoch \in [0..CBMax -> Nat \ {0}]
                   /\ CBImageValid \in [0..CBMax -> BOOLEAN]
                   /\ CBPublication \in BOOLEAN
\* Each image is immutable and atomically bound to its applied cut. Remote
\* restore returns the hash-verified selected image. CBPublication stands for
\* the enclosing retained committed winner/final-history validator, not for
\* proposal or object presence. These are explicit lower-layer premises.
VARIABLES cbLive, cbFrozen, cbScope, cbCurrent, cbPhase, cbMinted,
          cbChecked, cbComplete
cbVars == <<cbLive, cbFrozen, cbScope, cbCurrent, cbPhase, cbMinted,
            cbChecked, cbComplete>>
CBInit == /\ cbLive = 0 /\ cbFrozen = 0 /\ cbScope = 0 /\ cbCurrent = 0
          /\ cbPhase = "idle" /\ cbMinted = FALSE /\ cbChecked = FALSE /\ cbComplete = FALSE
CBAdvance == /\ cbLive < CBMax /\ cbLive' = cbLive + 1
             /\ UNCHANGED <<cbFrozen, cbScope, cbCurrent, cbPhase, cbMinted,
                            cbChecked, cbComplete>>
CBFreeze == /\ cbPhase = "idle" /\ cbFrozen' = cbLive /\ cbPhase' = "frozen"
            /\ UNCHANGED <<cbLive, cbScope, cbCurrent, cbMinted, cbChecked, cbComplete>>
CBMint == /\ cbPhase = "frozen" /\ cbScope' = CBImageEpoch[cbFrozen]
          /\ cbMinted' = TRUE /\ cbPhase' = "minted"
          /\ UNCHANGED <<cbLive, cbFrozen, cbCurrent, cbChecked, cbComplete>>
CBRestore == /\ cbPhase = "minted" /\ cbCurrent' = cbFrozen /\ cbPhase' = "restored"
             /\ UNCHANGED <<cbLive, cbFrozen, cbScope, cbMinted, cbChecked, cbComplete>>
CBGood == /\ CBImageRoot[cbFrozen] = CBExpectedRoot
          /\ CBImageValid[cbFrozen] /\ cbScope = CBImageEpoch[cbFrozen]
CBCheck == /\ cbPhase = "restored"
           /\ cbChecked' = CBGood
           /\ cbPhase' = IF CBGood THEN "checked" ELSE "refused"
           /\ UNCHANGED <<cbLive, cbFrozen, cbScope, cbCurrent, cbMinted, cbComplete>>
CBReplay == /\ cbPhase = "checked" /\ cbCurrent' = cbLive /\ cbPhase' = "replayed"
            /\ UNCHANGED <<cbLive, cbFrozen, cbScope, cbMinted, cbChecked, cbComplete>>
CBFinish == /\ cbPhase = "replayed" /\ cbComplete' = TRUE /\ cbPhase' = "complete"
            /\ UNCHANGED <<cbLive, cbFrozen, cbScope, cbCurrent, cbMinted, cbChecked>>
CBGrant == /\ cbPhase = "complete"
           /\ cbPhase' = IF CBPublication THEN "served" ELSE "refused"
           /\ UNCHANGED <<cbLive, cbFrozen, cbScope, cbCurrent, cbMinted, cbChecked, cbComplete>>
CBFail == /\ cbPhase \in {"frozen", "minted", "restored", "checked", "replayed", "complete"}
          /\ cbPhase' = "refused"
          /\ UNCHANGED <<cbLive, cbFrozen, cbScope, cbCurrent, cbMinted, cbChecked, cbComplete>>
CBNext == CBAdvance \/ CBFreeze \/ CBMint \/ CBRestore \/ CBCheck \/ CBReplay \/ CBFinish \/ CBGrant \/ CBFail
CBSpec == CBInit /\ [][CBNext]_cbVars
CBType == /\ cbLive \in 0..CBMax /\ cbFrozen \in 0..cbLive /\ cbCurrent \in 0..cbLive
          /\ cbScope \in Nat /\ cbMinted \in BOOLEAN /\ cbChecked \in BOOLEAN /\ cbComplete \in BOOLEAN
          /\ cbPhase \in {"idle", "frozen", "minted", "restored", "checked", "replayed", "complete", "served", "refused"}
CBSameImage == cbMinted => cbScope = CBImageEpoch[cbFrozen]
CBBaseGate == cbChecked => CBGood
CBPhaseGate == /\ cbPhase \in {"minted", "restored", "checked", "replayed", "complete", "served"} => cbMinted
               /\ cbPhase \in {"checked", "replayed", "complete", "served"} => cbChecked
               /\ cbPhase = "served" => cbComplete /\ CBPublication
               /\ cbPhase = "complete" => cbComplete
               /\ cbComplete => cbChecked
               /\ cbPhase \in {"idle", "frozen"} => ~cbMinted /\ ~cbChecked /\ ~cbComplete
CBInvariant == CBType /\ CBSameImage /\ CBBaseGate /\ CBPhaseGate
=============================================================================
