------------------------- MODULE EndpointWritersMC -------------------------
EXTENDS EndpointWriters
EWMCBehind(capturing) == /\ ewGeneration = 1 /\ ewAddress = 2
                         /\ ewAdmission = "Revoked" /\ ewAdmitted = EWInitial
                         /\ ewInstalledGeneration = 0 /\ ewInstalledAddress = EWInitial
                         /\ ewCapturing = capturing /\ ewCaptureGeneration = 0 /\ ewCaptureAddress = EWInitial
EWMCFromOld == EWMCBehind(TRUE) /\ [][EWStableNext]_ewVars /\ EWStableFairness
EWMCFromIdle == EWMCBehind(FALSE) /\ [][EWStableNext]_ewVars /\ EWStableFairness
=============================================================================
