----------------------- MODULE EndpointRecoveryMC -----------------------
EXTENDS EndpointRecovery
ERVersions == [i \in 0..ERLimit |-> i \div 2]
ERAddressesAt == [i \in 0..ERLimit |-> 1 + ((i \div 2) % 2)]
ERMCStableInit == ERStableInit /\ erApplied = 0 /\ erConfigured = ERAddress[ERLimit]
                  /\ erSaved = TRUE /\ erSavedAt = 0
                  /\ erSavedAddress = ERAddress[0] /\ erServing = FALSE
ERMCStableSpec == ERMCStableInit /\ [][ERStableNext \/ UNCHANGED erVars]_erVars /\ ERFair
=============================================================================
