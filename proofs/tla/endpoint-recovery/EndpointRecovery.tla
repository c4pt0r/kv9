-------------------------- MODULE EndpointRecovery --------------------------
EXTENDS Naturals
CONSTANT ERAddresses, ERLimit, ERVersion, ERAddress
\* An arbitrary consensus-ordered history of one immutable Active node/store.
\* Equal catalog generations name one address, including across noop entries.
ASSUME ERLegalInputs ==
    /\ ERAddresses # {} /\ ERAddresses \subseteq Nat \ {0}
    /\ ERLimit \in Nat \ {0}
    /\ ERVersion \in [0..ERLimit -> 0..ERLimit]
    /\ ERAddress \in [0..ERLimit -> ERAddresses]
    /\ ERVersion[0] = 0
    /\ \A i, j \in 0..ERLimit :
          /\ (i <= j => ERVersion[i] <= ERVersion[j])
          /\ (ERVersion[i] = ERVersion[j] => ERAddress[i] = ERAddress[j])

VARIABLES erCommit, erApplied, erConfigured, erPending, erSaved,
          erSavedAt, erSavedAddress, erServing
erVars == <<erCommit, erApplied, erConfigured, erPending, erSaved,
            erSavedAt, erSavedAddress, erServing>>
ERRecord == <<erSaved, erSavedAt, erSavedAddress>>
ERInit == /\ erCommit = 0 /\ erApplied = 0 /\ erConfigured = ERAddress[0]
          /\ erPending = 0 /\ erSaved = FALSE /\ erSavedAt = 0
          /\ erSavedAddress = ERAddress[0] /\ erServing = FALSE
ERSavedUsable == /\ erSaved /\ erSavedAddress = erConfigured
                 /\ erApplied >= erSavedAt /\ ERAddress[erApplied] = erConfigured
\* Another authorized operator may move the directory while this node is down
\* or waiting for an earlier confirmation. Raft supplies one committed prefix.
ERAdvance == /\ erCommit < ERLimit /\ erCommit' = erCommit + 1
             /\ UNCHANGED <<erApplied, erConfigured, erPending, ERRecord, erServing>>
ERApply == /\ erApplied < erCommit /\ erApplied' = erCommit
           /\ UNCHANGED <<erCommit, erConfigured, erPending, ERRecord, erServing>>
\* A successful confirmation is a NEW committed Command::Noop, not the
\* operator's original mutation position. Failure/timeouts take stutter steps.
ERConfirm == /\ ~erServing /\ ~ERSavedUsable /\ erPending = 0
             /\ erCommit < ERLimit /\ ERAddress[erCommit] = erConfigured
             /\ ERVersion[erCommit + 1] = ERVersion[erCommit]
             /\ erCommit' = erCommit + 1 /\ erPending' = erCommit + 1
             /\ UNCHANGED <<erApplied, erConfigured, ERRecord, erServing>>
\* wait_applied's exact-receipt contract is the refinement boundary. A larger
\* observed position without that contract cannot enable publication.
ERPublish == /\ erPending > 0 /\ erApplied >= erPending
             /\ ERVersion[erApplied] = ERVersion[erPending]
             /\ ERAddress[erApplied] = erConfigured
             /\ erSaved' = TRUE /\ erSavedAt' = erPending
             /\ erSavedAddress' = erConfigured /\ erPending' = 0
             /\ UNCHANGED <<erCommit, erApplied, erConfigured, erServing>>
ERInitial == /\ ~erSaved /\ ERVersion[erApplied] = 0
             /\ ERAddress[erApplied] = erConfigured
             /\ erSaved' = TRUE /\ erSavedAt' = 0 /\ erSavedAddress' = erConfigured
             /\ UNCHANGED <<erCommit, erApplied, erConfigured, erPending, erServing>>
ERServe == /\ ~erServing /\ ERSavedUsable /\ erServing' = TRUE
           /\ UNCHANGED <<erCommit, erApplied, erConfigured, erPending, ERRecord>>
ERReject == /\ erServing /\ ~ERSavedUsable /\ erServing' = FALSE
            /\ UNCHANGED <<erCommit, erApplied, erConfigured, erPending, ERRecord>>
\* Superseded or expired receipts never become saved authority. A later
\* attempt may append another confirmation instead of inventing a receipt.
ERForget == /\ erPending > 0 /\ erPending' = 0
            /\ UNCHANGED <<erCommit, erApplied, erConfigured, ERRecord, erServing>>
\* Only a process restart changes configured advertisement. Successful fsync
\* publication and the durable applied prefix survive; pending RPC state does
\* not. Failed publication grants no authority, whether before/after rename.
ERCrash(a) == /\ a \in ERAddresses /\ erConfigured' = a
              /\ erPending' = 0 /\ erServing' = FALSE
              /\ UNCHANGED <<erCommit, erApplied, ERRecord>>
ERNext == ERAdvance \/ ERApply \/ ERConfirm \/ ERPublish \/ ERInitial \/ ERServe
          \/ ERReject \/ ERForget \/ (\E a \in ERAddresses : ERCrash(a))
ERSpec == ERInit /\ [][ERNext]_erVars
ERType == /\ erCommit \in 0..ERLimit /\ erApplied \in 0..erCommit
          /\ erConfigured \in ERAddresses /\ erPending \in 0..erCommit
          /\ erSaved \in BOOLEAN /\ erSavedAt \in 0..erApplied
          /\ erSavedAddress \in ERAddresses /\ erServing \in BOOLEAN
ERAuthority == /\ (erSaved => erSavedAddress = ERAddress[erSavedAt])
               /\ (erPending > 0 => ERAddress[erPending] = erConfigured)
               /\ (erServing => erSaved /\ erConfigured = erSavedAddress)
ERInvariant == ERType /\ ERAuthority
ERFreshReceipt == ERConfirm => erPending' = erCommit + 1 /\ erPending' > erCommit
ERPublication == ERPublish => erSavedAt' = erPending /\ erSavedAt' <= erApplied
                             /\ ERVersion[erApplied] = ERVersion[erPending]
ERAdmission == ERServe => ERSavedUsable
ERNoRollback == erApplied' >= erApplied
EREffects == [][ERFreshReceipt /\ ERPublication /\ ERAdmission /\ ERNoRollback]_erVars
ERNoPendingCatchup == ~(erPending > erApplied)
ERNoChangedRecovery == ~(erServing /\ erConfigured # ERAddress[0])

\* Conditional liveness: the last slot is an available noop under a stable
\* authority, with no further crashes, expirations or competing updates. Fair
\* confirmation includes reachable authenticated peers, quorum and completion;
\* apply/publication fairness includes healthy durable storage and scheduling.
ERStable == /\ ERInvariant /\ erCommit \in {ERLimit - 1, ERLimit}
            /\ ERVersion[ERLimit] = ERVersion[ERLimit - 1]
            /\ erConfigured = ERAddress[ERLimit]
            /\ erPending \in {0, ERLimit}
ERStableNext == ERApply \/ ERConfirm \/ ERPublish \/ ERServe
ERFair == WF_erVars(ERApply) /\ WF_erVars(ERConfirm)
          /\ WF_erVars(ERPublish) /\ WF_erVars(ERServe)
\* A confirmation is only needed if no usable saved record exists. The
\* initial stable state reserves its final slot for this attempt.
ERStableInit == ERStable /\ erCommit = ERLimit - 1 /\ erPending = 0
ERStableSpec == ERStableInit /\ [][ERStableNext]_erVars /\ ERFair
ERConverges == <>erServing
ERRecoveryOrder == ERStable /\ (erCommit = ERLimit => erPending = ERLimit \/ ERSavedUsable)
ERNeed == ERRecoveryOrder /\ ~erServing /\ ~ERSavedUsable /\ erPending = 0
ERCatching == ERRecoveryOrder /\ ~erServing /\ ~ERSavedUsable /\ erPending = ERLimit /\ erApplied < ERLimit
ERObserved == ERRecoveryOrder /\ ~erServing /\ ~ERSavedUsable /\ erPending = ERLimit /\ erApplied = ERLimit
ERReady == ERRecoveryOrder /\ ~erServing /\ ERSavedUsable
=============================================================================
