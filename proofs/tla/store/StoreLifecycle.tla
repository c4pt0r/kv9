-------------------------- MODULE StoreLifecycle --------------------------
EXTENDS Naturals
CONSTANT SLIncarnations
ASSUME SLLegalInputs == SLIncarnations # {} /\ SLIncarnations \subseteq Nat \ {0}

\* One root voter and one data directory. Used/ever/retired are history ghosts,
\* never consulted by startup. Preparation allocates an independent fresh id.
\* Durable log identity abstracts checked recovery of the Raft log/position;
\* a root certificate alone does not reconstruct a missing log.
VARIABLES slInc, slRoot, slUsed, slEver, slRetired, slPhase, slWritten,
          slLog, slLogWritten, slLive, slFailed, slOwner, slCert
slVars == <<slInc, slRoot, slUsed, slEver, slRetired, slPhase, slWritten,
            slLog, slLogWritten, slLive, slFailed, slOwner, slCert>>
SLInit == /\ slInc = 0 /\ slRoot = 0 /\ slUsed = {} /\ slEver = {} /\ slRetired = {}
          /\ slPhase = 0 /\ slWritten = 0 /\ slLog = 0 /\ slLogWritten = 0
          /\ slLive = TRUE /\ slFailed = FALSE /\ slOwner = FALSE /\ slCert = FALSE
SLHealthy == slLive /\ ~slFailed
\* The preparation response follows durable publication. Unpublished temporary
\* files do not authorize root creation or participation.
SLPrepare(i) == /\ SLHealthy /\ slInc = 0 /\ i \in SLIncarnations \ slUsed
                /\ slInc' = i /\ slUsed' = slUsed \cup {i} /\ slPhase' = 1 /\ slWritten' = 1
                /\ UNCHANGED <<slRoot, slEver, slRetired, slLog, slLogWritten,
                               slLive, slFailed, slOwner, slCert>>
SLRootBind == /\ SLHealthy /\ slRoot = 0 /\ slPhase = 1 /\ slRoot' = slInc
              /\ UNCHANGED <<slInc, slUsed, slEver, slRetired, slPhase, slWritten,
                             slLog, slLogWritten, slLive, slFailed, slOwner, slCert>>
SLWriteBinding == /\ SLHealthy /\ slPhase = 1 /\ slRoot = slInc /\ slWritten = 1
                  /\ slWritten' = 2
                  /\ UNCHANGED <<slInc, slRoot, slUsed, slEver, slRetired, slPhase,
                                 slLog, slLogWritten, slLive, slFailed, slOwner, slCert>>
SLPublish == /\ SLHealthy /\ slWritten > slPhase /\ slPhase' = slWritten
             /\ UNCHANGED <<slInc, slRoot, slUsed, slEver, slRetired, slWritten,
                            slLog, slLogWritten, slLive, slFailed, slOwner, slCert>>
SLCreateLog == /\ SLHealthy /\ slPhase = 2 /\ slLog = 0 /\ slLogWritten = 0
               /\ slLogWritten' = slInc
               /\ UNCHANGED <<slInc, slRoot, slUsed, slEver, slRetired, slPhase,
                              slWritten, slLog, slLive, slFailed, slOwner, slCert>>
SLSyncLog == /\ SLHealthy /\ slLogWritten = slInc /\ slInc # 0 /\ slLog = 0
             /\ slLog' = slLogWritten
             /\ UNCHANGED <<slInc, slRoot, slUsed, slEver, slRetired, slPhase,
                            slWritten, slLogWritten, slLive, slFailed, slOwner, slCert>>
SLWriteActivation == /\ SLHealthy /\ slPhase = 2 /\ slWritten = 2 /\ slLog = slInc
                     /\ slWritten' = 3
                     /\ UNCHANGED <<slInc, slRoot, slUsed, slEver, slRetired, slPhase,
                                    slLog, slLogWritten, slLive, slFailed, slOwner, slCert>>
SLStart == /\ SLHealthy /\ slPhase = 3 /\ slLog = slInc /\ slInc = slRoot /\ slInc # 0 /\ ~slOwner
           /\ slOwner' = TRUE /\ slEver' = slEver \cup {slInc}
           /\ UNCHANGED <<slInc, slRoot, slUsed, slRetired, slPhase, slWritten,
                          slLog, slLogWritten, slLive, slFailed, slCert>>
SLCertify == /\ slOwner /\ slCert' = TRUE
             /\ UNCHANGED <<slInc, slRoot, slUsed, slEver, slRetired, slPhase, slWritten,
                            slLog, slLogWritten, slLive, slFailed, slOwner>>
SLFail == /\ slLive /\ slFailed' = TRUE /\ slOwner' = FALSE
          /\ UNCHANGED <<slInc, slRoot, slUsed, slEver, slRetired, slPhase, slWritten,
                         slLog, slLogWritten, slLive, slCert>>
SLCrash == /\ slLive /\ slLive' = FALSE /\ slOwner' = FALSE
           /\ UNCHANGED <<slInc, slRoot, slUsed, slEver, slRetired, slPhase, slWritten,
                          slLog, slLogWritten, slFailed, slCert>>
\* Reopen may retain unsynced bytes. It stabilizes the selected visible record
\* and log before either can authorize a new owner; synced bytes never roll back.
SLRestart(p, l) == /\ (~slLive \/ slFailed) /\ p \in {slPhase, slWritten} /\ l \in {slLog, slLogWritten}
                  /\ slPhase' = p /\ slWritten' = p /\ slLog' = l /\ slLogWritten' = l
                  /\ slLive' = TRUE /\ slFailed' = FALSE /\ slOwner' = FALSE
                  /\ UNCHANGED <<slInc, slRoot, slUsed, slEver, slRetired, slCert>>
SLLoseLog == /\ ~slLive /\ slLog' = 0 /\ slLogWritten' = 0
             /\ slRetired' = slRetired \cup (slEver \cap {slInc})
             /\ UNCHANGED <<slInc, slRoot, slUsed, slEver, slPhase, slWritten,
                            slLive, slFailed, slOwner, slCert>>
SLLoseLifecycle == /\ ~slLive /\ slPhase' = 0 /\ slWritten' = 0
                   /\ UNCHANGED <<slInc, slRoot, slUsed, slEver, slRetired, slLog,
                                  slLogWritten, slLive, slFailed, slOwner, slCert>>
SLWholeLoss == /\ ~slLive /\ slInc' = 0 /\ slPhase' = 0 /\ slWritten' = 0
               /\ slLog' = 0 /\ slLogWritten' = 0 /\ slCert' = FALSE
               /\ slRetired' = slRetired \cup (slEver \cap {slInc})
               /\ UNCHANGED <<slRoot, slUsed, slEver, slLive, slFailed, slOwner>>
SLLegacyWrite == /\ SLHealthy /\ slPhase = 0 /\ slCert /\ slInc # 0
                 /\ slLog = slInc /\ slRoot = slInc /\ slWritten' = 3
                 /\ UNCHANGED <<slInc, slRoot, slUsed, slEver, slRetired, slPhase,
                                slLog, slLogWritten, slLive, slFailed, slOwner, slCert>>
SLQuiesce == UNCHANGED slVars
SLNext == (\E i \in SLIncarnations : SLPrepare(i))
          \/ (\E p \in 0..3, l \in SLIncarnations \cup {0} : SLRestart(p, l))
          \/ SLRootBind \/ SLWriteBinding \/ SLPublish \/ SLCreateLog \/ SLSyncLog
          \/ SLWriteActivation \/ SLStart \/ SLCertify \/ SLFail \/ SLCrash
          \/ SLLoseLog \/ SLLoseLifecycle \/ SLWholeLoss \/ SLLegacyWrite \/ SLQuiesce
SLSpec == SLInit /\ [][SLNext]_slVars
SLType == /\ slInc \in SLIncarnations \cup {0} /\ slRoot \in SLIncarnations \cup {0}
          /\ slUsed \in SUBSET SLIncarnations /\ slEver \in SUBSET slUsed /\ slRetired \in SUBSET slEver
          /\ slPhase \in 0..3 /\ slWritten \in slPhase..3
          /\ slLog \in {0, slInc} /\ slLogWritten \in {0, slInc}
          /\ (slLog # 0 => slLogWritten = slLog)
          /\ slLive \in BOOLEAN /\ slFailed \in BOOLEAN /\ slOwner \in BOOLEAN /\ slCert \in BOOLEAN
SLIdentity == /\ (slInc # 0 => slInc \in slUsed) /\ (slRoot # 0 => slRoot \in slUsed)
              /\ (slInc = 0 => slPhase = 0 /\ slWritten = 0 /\ ~slCert)
              /\ (slPhase >= 2 => slRoot = slInc /\ slInc # 0)
              /\ (slWritten >= 2 => slRoot = slInc /\ slInc # 0)
              /\ (slCert => slInc \in slEver /\ slRoot = slInc)
SLHistory == slInc \in slEver => slPhase \in {0, 3} /\ slWritten \in {0, 3}
SLNoRecreatedLog == slInc \in slRetired => slLog = 0 /\ slLogWritten = 0
SLPermission == slOwner => SLHealthy /\ slPhase = 3 /\ slLog = slInc /\ slInc = slRoot /\ slInc # 0 /\ slInc \notin slRetired /\ slInc \in slEver
SLInvariant == SLType /\ SLIdentity /\ SLHistory /\ SLNoRecreatedLog /\ SLPermission
SLRootStep == slRoot # 0 => slRoot' = slRoot
SLRootAlways == [][SLRootStep]_slVars
SLActivationStep == (slWritten' = 3 /\ slWritten # 3) =>
                    slLog = slInc /\ slInc # 0 /\ (slPhase = 2 \/ slCert)
SLActivationAlways == [][SLActivationStep]_slVars

\* Conditional progress after the matching store has a durable log. Filesystem
\* success is an explicit cut; no progress during unlimited faults is claimed.
SLStableNext == SLRootBind \/ SLWriteBinding \/ SLPublish \/ SLCreateLog \/ SLSyncLog
                \/ SLWriteActivation \/ SLStart \/ SLCertify \/ SLQuiesce
SLFairSpec == SLInvariant /\ [][SLStableNext]_slVars
              /\ WF_slVars(SLWriteActivation) /\ WF_slVars(SLPublish) /\ WF_slVars(SLStart)
SLReady == SLInvariant /\ SLHealthy /\ slInc # 0 /\ slRoot = slInc /\ slLog = slInc /\ slPhase \in {2, 3}
SLWrittenReady == SLReady /\ slWritten = 3
SLActiveReady == SLReady /\ slPhase = 3
SLStarted == SLActiveReady /\ slOwner
SLProgress == SLReady ~> slOwner
SLNoStartedOwner == ~slOwner
SLNoCertifiedRecovery == ~(slPhase = 0 /\ slCert /\ slLog # 0)
=============================================================================
