------------------------ MODULE LeaseAuthorityMC ------------------------
EXTENDS LeaseAuthority, TLC
CONSTANTS LATimeMax, LACommitMax
LASymmetry == Permutations(LANodes \ {LALeader})

\* Finite exploration restrictions, not premises of the deductive proof.
LAMCNext == LANext /\ laNow' <= LATimeMax /\ laCommit' <= LACommitMax
LAMCSpec == LAInit /\ [][LAMCNext]_laVars

LANoSuccessfulReadWitness == \A r \in LAReads : laReadPhase[r] # "success"
LANoReplacementWriteWitness == ~laReplacement \/ laCommit = 1
LANoRenewalWitness == \A r1, r2 \in laPublished : r1 = r2
LANoExpiredCertificateWitness == \A r \in laPublished : laNow < laEnd[r]
LANoRestartPromiseWitness == \A q \in laLive :
    laRecovery[q] = 0 \/ (\A r \in LARounds : q \notin laGranted[r])
LANoApplyGapWitness == laApplied = laKnown
LANoOldGenerationWitness == \A r \in laPublished : laRoundGen[r] = laGeneration
=============================================================================
