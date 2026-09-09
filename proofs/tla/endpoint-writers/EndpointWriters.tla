-------------------------- MODULE EndpointWriters --------------------------
EXTENDS Naturals
CONSTANT EWAddresses, EWInitial, EWLimit
ASSUME EWLegalInputs == EWAddresses # {} /\ EWAddresses \subseteq Nat \ {0}
                        /\ EWInitial \in EWAddresses /\ EWLimit \in Nat \ {0}

\* One existing immutable node/store binding. EndpointCAS supplies operator
\* authorization; credential checking supplies registration authorization.
\* Remote applies may advance the catalog while a local snapshot is held.
VARIABLES ewGeneration, ewAddress, ewAdmission, ewAdmitted,
          ewInstalledGeneration, ewInstalledAddress,
          ewCapturing, ewCaptureGeneration, ewCaptureAddress
EWDirectory == <<ewGeneration, ewAddress>>
EWAuthority == <<ewAdmission, ewAdmitted>>
EWInstalled == <<ewInstalledGeneration, ewInstalledAddress>>
EWCapture == <<ewCapturing, ewCaptureGeneration, ewCaptureAddress>>
ewVars == <<EWDirectory, EWAuthority, EWInstalled, EWCapture>>
EWInit == /\ ewGeneration = 0 /\ ewAddress = EWInitial
          /\ ewAdmission = "None" /\ ewAdmitted = EWInitial
          /\ ewInstalledGeneration = 0 /\ ewInstalledAddress = EWInitial
          /\ ewCapturing = FALSE /\ ewCaptureGeneration = 0 /\ ewCaptureAddress = EWInitial

EWNewAdmission(a) == /\ a \in EWAddresses /\ ewAdmission \in {"None", "Superseded", "Revoked"}
                     /\ ewAdmission' = "Pending" /\ ewAdmitted' = a
                     /\ UNCHANGED <<EWDirectory, EWInstalled, EWCapture>>
EWDelta(a, registration) == IF registration /\ a = ewAddress THEN 0 ELSE 1
EWCancel(admission) == IF admission \in {"None", "Revoked"} THEN admission ELSE "Superseded"
EWChange(a, registration, local) ==
    /\ a \in EWAddresses /\ registration \in BOOLEAN /\ local \in BOOLEAN
    /\ (registration => ewAdmission = "Pending" /\ a = ewAdmitted)
    /\ (local => ewCapturing /\ ewCaptureGeneration = ewGeneration /\ ewCaptureAddress = ewAddress)
    /\ ewGeneration + EWDelta(a, registration) <= EWLimit
    /\ ewGeneration' = ewGeneration + EWDelta(a, registration) /\ ewAddress' = a
    /\ ewAdmission' = IF registration THEN "Consumed" ELSE EWCancel(ewAdmission)
    /\ UNCHANGED <<ewAdmitted, EWInstalled, ewCapturing>>
    /\ IF local THEN /\ ewCaptureGeneration' = ewGeneration' /\ ewCaptureAddress' = a
                ELSE UNCHANGED <<ewCaptureGeneration, ewCaptureAddress>>
\* A local applied snapshot can lag a remotely certified committed route.
\* All certificates name an immutable node/store and an ordered generation.
EWStart == /\ ~ewCapturing /\ ewCapturing' = TRUE
           /\ ewCaptureGeneration' = ewGeneration /\ ewCaptureAddress' = ewAddress
           /\ UNCHANGED <<EWDirectory, EWAuthority, EWInstalled>>
EWCaptureWins == ewCaptureGeneration >= ewInstalledGeneration
EWInstall == /\ ewCapturing /\ ewCapturing' = FALSE
             /\ ewInstalledGeneration' = IF EWCaptureWins THEN ewCaptureGeneration ELSE ewInstalledGeneration
             /\ ewInstalledAddress' = IF EWCaptureWins THEN ewCaptureAddress ELSE ewInstalledAddress
             /\ UNCHANGED <<EWDirectory, EWAuthority, ewCaptureGeneration, ewCaptureAddress>>
\* A certified committed update may arrive while an older local snapshot is
\* held. The transport generation floor, not a local catalog mutex, fences it.
EWEager(a) == /\ a \in EWAddresses /\ ewGeneration < EWLimit
              /\ ewGeneration' = ewGeneration + 1 /\ ewAddress' = a
              /\ ewAdmission' = EWCancel(ewAdmission)
              /\ ewInstalledGeneration' = ewGeneration + 1 /\ ewInstalledAddress' = a
              /\ UNCHANGED <<ewAdmitted, EWCapture>>
EWCertified == /\ ewInstalledGeneration' = ewGeneration /\ ewInstalledAddress' = ewAddress
               /\ UNCHANGED <<EWDirectory, EWAuthority, EWCapture>>
EWDecommission == /\ ewAdmission \in {"Pending", "Consumed", "Superseded"}
                  /\ ewAdmission' = "Revoked"
                  /\ UNCHANGED <<EWDirectory, ewAdmitted, EWInstalled, EWCapture>>
EWNext == EWStart \/ EWInstall \/ (\E a \in EWAddresses : EWNewAdmission(a))
          \/ EWCertified \/ EWDecommission
          \/ (\E a \in EWAddresses : EWEager(a))
          \/ (\E a \in EWAddresses, registration, local \in BOOLEAN : EWChange(a, registration, local))
EWSpec == EWInit /\ [][EWNext]_ewVars

EWType == /\ ewGeneration \in 0..EWLimit /\ ewAddress \in EWAddresses
          /\ ewAdmission \in {"None", "Pending", "Consumed", "Superseded", "Revoked"} /\ ewAdmitted \in EWAddresses
          /\ ewInstalledGeneration \in 0..EWLimit /\ ewInstalledAddress \in EWAddresses
          /\ ewCapturing \in BOOLEAN /\ ewCaptureGeneration \in 0..EWLimit /\ ewCaptureAddress \in EWAddresses
EWOrder == /\ ewInstalledGeneration <= ewGeneration
           /\ (ewInstalledGeneration = ewGeneration => ewInstalledAddress = ewAddress)
           /\ (ewCapturing =>
                 /\ ewCaptureGeneration <= ewGeneration
                 /\ (ewCaptureGeneration = ewGeneration => ewCaptureAddress = ewAddress)
                 /\ (ewCaptureGeneration = ewInstalledGeneration => ewCaptureAddress = ewInstalledAddress))
EWInvariant == EWType /\ EWOrder
EWVersion == /\ ewGeneration' \in {ewGeneration, ewGeneration + 1}
             /\ (ewGeneration' = ewGeneration => ewAddress' = ewAddress)
EWInstallOrder == ewInstalledGeneration' >= ewInstalledGeneration
EWRevocation == /\ (\A a \in EWAddresses, local \in BOOLEAN :
                     EWChange(a, FALSE, local) => ewAdmission' \in {"None", "Superseded", "Revoked"})
                /\ (\A a \in EWAddresses : EWEager(a) => ewAdmission' \in {"None", "Superseded", "Revoked"})
EWPreserveMember ==
    /\ (\A a \in EWAddresses, local \in BOOLEAN :
         EWChange(a, FALSE, local) => (ewAdmission' = "Revoked") = (ewAdmission = "Revoked"))
    /\ (\A a \in EWAddresses : EWEager(a) => (ewAdmission' = "Revoked") = (ewAdmission = "Revoked"))
EWRegistration == \A a \in EWAddresses, local \in BOOLEAN :
                    EWChange(a, TRUE, local) =>
                       ewAdmission = "Pending" /\ a = ewAdmitted /\ ewAdmission' = "Consumed"
EWStepEffects == EWVersion /\ EWInstallOrder /\ EWRevocation /\ EWRegistration /\ EWPreserveMember
EWEffects == [][EWStepEffects]_ewVars

\* Once committed routing stops changing, fair lock acquisition and completion
\* converge to that route, even if the initially held capture is obsolete.
EWStableNext == EWStart \/ EWInstall
EWStableFairness == WF_ewVars(EWStart) /\ WF_ewVars(EWInstall)
EWStableSpec == EWInvariant /\ [][EWStableNext]_ewVars /\ EWStableFairness
EWCurrent == EWInvariant /\ ewInstalledGeneration = ewGeneration /\ ewInstalledAddress = ewAddress
EWOld == EWInvariant /\ ewCapturing /\ ewCaptureGeneration < ewGeneration
EWWaiting == EWInvariant /\ ~ewCapturing /\ ewInstalledGeneration < ewGeneration
EWFresh == EWInvariant /\ ewCapturing /\ ewCaptureGeneration = ewGeneration
EWConverges == <>EWCurrent
EWNoRevocation == ewAdmission # "Revoked"
EWNoRemoteAdvanceDuringCapture == ~(ewCapturing /\ ewCaptureGeneration < ewGeneration)
=============================================================================
