---------------------------- MODULE ReadCreditMC ----------------------------
EXTENDS ReadCredit
CONSTANT RCTermLimit
RCMCReset == RCReset /\ raTerm' <= RCTermLimit
RCMCNext == RCPoll \/ RCReadStep \/ RCMCReset \/ RCOtherAdmission \/ RCRelease \/ RCCancel
RCMCSpec == RCInit /\ [][RCMCNext]_rcVars

\* Include the real current-term-commit transition before the stable window.
\* Fair commit prevents an uncommitted initial stutter from masquerading as
\* the credit-release counterexample; its trace must reach a ready leader.
RCMCProgressInit == /\ raTerm = 1 /\ raCommitTerm = 0 /\ raLeader = TRUE
                    /\ raPhase = "Waiting" /\ raContext = RAOwn /\ raSubmits = 0
                    /\ raAdmittedTerm = 0 /\ raAdmittedCommitTerm = 0
                    /\ raQuorum = FALSE /\ raReceipt = 0 /\ raApplied = FALSE
                    /\ raBudget = RABudget /\ rcOccupied \in BOOLEAN
                    /\ rcCanceled = FALSE
RCMCCommit == RACommit /\ UNCHANGED <<rcOccupied, rcCanceled>>
RCMCProgressNext == RCMCCommit \/ RCProgressNext
RCMCProgressSpec == RCMCProgressInit /\ [][RCMCProgressNext]_rcVars
                    /\ WF_rcVars(RCMCCommit) /\ RCProgressFairness
RCMCAdmission == <>RCAdmitted
=============================================================================
