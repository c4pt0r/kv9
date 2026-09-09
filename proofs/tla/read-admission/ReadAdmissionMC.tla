--------------------------- MODULE ReadAdmissionMC ---------------------------
EXTENDS ReadAdmission
CONSTANT RATermBound
RAMCAdvance(l) == raTerm < RATermBound /\ RAAdvance(l)
RAMCNext == RAPoll \/ RACommit \/ (\E l \in BOOLEAN : RAMCAdvance(l))
            \/ RALose \/ RACertify \/ (\E c \in {RAOwn, RAOther} : RADeliver(c))
            \/ RACatchUp \/ RAComplete \/ RATick \/ RATimeout \/ UNCHANGED raVars
RAMCSpec == RAInit /\ [][RAMCNext]_raVars
\* Expanding the added stutter yields the same [][RAStableNext]_raVars.
\* TLC requires a single next-state conjunct and an explicit terminal successor.
RAMCSuccess == RAInit /\ [][RAStableNext \/ UNCHANGED raVars]_raVars /\ RAStableFairness
=============================================================================
