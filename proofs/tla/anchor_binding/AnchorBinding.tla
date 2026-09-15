--------------------------- MODULE AnchorBinding ---------------------------
EXTENDS Naturals
CONSTANTS AMax, ARootOK, AConfigurationOK, APublicationOK, AFrameOK
ASSUME AInputs == /\ AMax \in Nat \ {0}
                  /\ ARootOK \in [1..AMax -> BOOLEAN]
                  /\ AConfigurationOK \in [1..AMax -> BOOLEAN]
                  /\ APublicationOK \in [1..AMax -> BOOLEAN]
                  /\ AFrameOK \in [1..AMax -> BOOLEAN]
\* Image IDs name complete immutable selected descriptors (root, scope, c/k/p,
\* configuration and manifest). Lower protocols establish the three OK facts
\* from actual restored state and retained committed history. FrameOK abstracts
\* bounded canonical encoding and nested checks, not checksum authentication.
VARIABLES aSelected, aBase, aPublication, aDecoded, aCap, aPhase, aComplete
aVars == <<aSelected, aBase, aPublication, aDecoded, aCap, aPhase, aComplete>>
AInit == /\ aSelected = 0 /\ aBase = 0 /\ aPublication = 0 /\ aDecoded = 0 /\ aCap = 0
         /\ aPhase = "idle" /\ aComplete = FALSE
ABegin == /\ aPhase = "idle" /\ aSelected' \in 1..AMax /\ aPhase' = "open"
          /\ UNCHANGED <<aBase, aPublication, aDecoded, aCap, aComplete>>
ABase == /\ aPhase = "open"
         /\ aBase' = IF ARootOK[aSelected] THEN aSelected ELSE 0
         /\ aPhase' = IF ARootOK[aSelected] THEN "scan" ELSE "refused"
         /\ UNCHANGED <<aSelected, aPublication, aDecoded, aCap, aComplete>>
ARecovered == /\ aPhase = "scan"
              /\ LET ok == AConfigurationOK[aSelected] /\ APublicationOK[aSelected] IN
                    /\ aPublication' = IF ok THEN aSelected ELSE 0
                    /\ aComplete' = ok
                    /\ aPhase' = IF ok THEN "complete" ELSE "refused"
              /\ UNCHANGED <<aSelected, aBase, aDecoded, aCap>>
ABind == /\ aPhase = "complete"
         /\ aCap' = IF AFrameOK[aSelected] THEN aSelected ELSE 0
         /\ aPhase' = IF AFrameOK[aSelected] THEN "bound" ELSE "refused"
         /\ UNCHANGED <<aSelected, aBase, aPublication, aDecoded, aComplete>>
ADecode == /\ aDecoded' \in {i \in 1..AMax : AFrameOK[i]}
           /\ UNCHANGED <<aSelected, aBase, aPublication, aCap, aPhase, aComplete>>
AFail == /\ aPhase \in {"open", "scan", "complete"} /\ aPhase' = "refused"
         /\ UNCHANGED <<aSelected, aBase, aPublication, aDecoded, aCap, aComplete>>
ANext == ABegin \/ ABase \/ ARecovered \/ ABind \/ ADecode \/ AFail
ASpec == AInit /\ [][ANext]_aVars
AType == /\ aSelected \in 0..AMax /\ aBase \in 0..AMax /\ aPublication \in 0..AMax
         /\ aDecoded \in 0..AMax /\ aCap \in 0..AMax /\ aComplete \in BOOLEAN
         /\ aPhase \in {"idle", "open", "scan", "complete", "bound", "refused"}
ASameOpen == /\ aBase # 0 => aBase = aSelected /\ ARootOK[aSelected]
             /\ aPublication # 0 => aPublication = aBase /\ aBase = aSelected
                  /\ AConfigurationOK[aSelected] /\ APublicationOK[aSelected]
APhases == /\ aPhase = "idle" => aBase = 0 /\ aPublication = 0 /\ aCap = 0 /\ ~aComplete
           /\ aPhase # "idle" => aSelected \in 1..AMax
           /\ aPhase = "open" => aBase = 0 /\ aPublication = 0 /\ aCap = 0 /\ ~aComplete
           /\ aPhase = "scan" => aBase = aSelected /\ aPublication = 0 /\ aCap = 0 /\ ~aComplete
           /\ aComplete => aPublication # 0
           /\ aPhase \in {"complete", "bound"} => aComplete
           /\ aPhase = "bound" => aCap # 0
ACapSafety == aCap # 0 => aCap = aSelected /\ aCap = aBase /\ aCap = aPublication
               /\ aComplete /\ ARootOK[aCap] /\ AConfigurationOK[aCap]
               /\ APublicationOK[aCap] /\ AFrameOK[aCap]
AInvariant == AType /\ ASameOpen /\ APhases /\ ACapSafety
\* Fixed bytes: header22 + lengths(root4, lists16, manifest4) + digests96
\* + epochs16 + image/publication32 + generation8 + change32 + two tags2.
\* Grouped to avoid deeply nested untyped arithmetic in the SMT translation.
AFrameSize(r, m, n1, n2, n3, n4, o) ==
    232 + (r + m + o) + 8 * (n1 + n2 + n3 + n4)
=============================================================================
