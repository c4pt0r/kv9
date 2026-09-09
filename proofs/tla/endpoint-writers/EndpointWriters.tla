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

EWNewAdmission(a) == /\ a \in EWAddresses /\ ewAdmission \in {"None", "Revoked"}
                     /\ ewAdmission' = "Pending" /\ ewAdmitted' = a
                     /\ UNCHANGED <<EWDirectory, EWInstalled, EWCapture>>
EWDelta(a, registration) == IF registration /\ a = ewAddress THEN 0 ELSE 1
EWChange(a, registration, local) ==
    /\ a \in EWAddresses /\ registration \in BOOLEAN /\ local \in BOOLEAN
    /\ (registration => ewAdmission = "Pending" /\ a = ewAdmitted)
    /\ (local => ewCapturing /\ ewCaptureGeneration = ewGeneration /\ ewCaptureAddress = ewAddress)
    /\ ewGeneration + EWDelta(a, registration) <= EWLimit
    /\ ewGeneration' = ewGeneration + EWDelta(a, registration) /\ ewAddress' = a
    /\ ewAdmission' = IF registration THEN "Consumed" ELSE IF ewAdmission = "None" THEN "None" ELSE "Revoked"
    /\ UNCHANGED <<ewAdmitted, EWInstalled, ewCapturing>>
    /\ IF local THEN /\ ewCaptureGeneration' = ewGeneration' /\ ewCaptureAddress' = a
                ELSE UNCHANGED <<ewCaptureGeneration, ewCaptureAddress>>
\* All local installers retain the same lock from validated capture through
\* installation. A catalog-confirmed eager write can advance its held capture.
EWStart == /\ ~ewCapturing /\ ewCapturing' = TRUE
           /\ ewCaptureGeneration' = ewGeneration /\ ewCaptureAddress' = ewAddress
           /\ UNCHANGED <<EWDirectory, EWAuthority, EWInstalled>>
EWInstall == /\ ewCapturing /\ ewCapturing' = FALSE
             /\ ewInstalledGeneration' = ewCaptureGeneration
             /\ ewInstalledAddress' = ewCaptureAddress
             /\ UNCHANGED <<EWDirectory, EWAuthority, ewCaptureGeneration, ewCaptureAddress>>
\* Another local planner's committed update and eager install, serialized
\* against a held sync snapshot by that same process's catalog mutex.
EWEager(a) == /\ ~ewCapturing /\ a \in EWAddresses /\ ewGeneration < EWLimit
              /\ ewGeneration' = ewGeneration + 1 /\ ewAddress' = a
              /\ ewAdmission' = IF ewAdmission = "None" THEN "None" ELSE "Revoked"
              /\ ewInstalledGeneration' = ewGeneration + 1 /\ ewInstalledAddress' = a
              /\ UNCHANGED <<ewAdmitted, EWCapture>>
EWNext == EWStart \/ EWInstall \/ (\E a \in EWAddresses : EWNewAdmission(a))
          \/ (\E a \in EWAddresses : EWEager(a))
          \/ (\E a \in EWAddresses, registration, local \in BOOLEAN : EWChange(a, registration, local))
EWSpec == EWInit /\ [][EWNext]_ewVars

EWType == /\ ewGeneration \in 0..EWLimit /\ ewAddress \in EWAddresses
          /\ ewAdmission \in {"None", "Pending", "Consumed", "Revoked"} /\ ewAdmitted \in EWAddresses
          /\ ewInstalledGeneration \in 0..EWLimit /\ ewInstalledAddress \in EWAddresses
          /\ ewCapturing \in BOOLEAN /\ ewCaptureGeneration \in 0..EWLimit /\ ewCaptureAddress \in EWAddresses
EWOrder == /\ ewInstalledGeneration <= ewGeneration
           /\ (ewInstalledGeneration = ewGeneration => ewInstalledAddress = ewAddress)
           /\ (ewCapturing =>
                 /\ ewInstalledGeneration <= ewCaptureGeneration /\ ewCaptureGeneration <= ewGeneration
                 /\ (ewCaptureGeneration = ewGeneration => ewCaptureAddress = ewAddress)
                 /\ (ewCaptureGeneration = ewInstalledGeneration => ewCaptureAddress = ewInstalledAddress))
EWInvariant == EWType /\ EWOrder
EWVersion == /\ ewGeneration' \in {ewGeneration, ewGeneration + 1}
             /\ (ewGeneration' = ewGeneration => ewAddress' = ewAddress)
EWInstallOrder == ewInstalledGeneration' >= ewInstalledGeneration
EWRevocation == /\ (\A a \in EWAddresses, local \in BOOLEAN :
                     EWChange(a, FALSE, local) => ewAdmission' \in {"None", "Revoked"})
                /\ (\A a \in EWAddresses : EWEager(a) => ewAdmission' \in {"None", "Revoked"})
EWRegistration == \A a \in EWAddresses, local \in BOOLEAN :
                    EWChange(a, TRUE, local) =>
                       ewAdmission = "Pending" /\ a = ewAdmitted /\ ewAdmission' = "Consumed"
EWStepEffects == EWVersion /\ EWInstallOrder /\ EWRevocation /\ EWRegistration
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
