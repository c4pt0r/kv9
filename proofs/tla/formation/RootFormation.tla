--------------------------- MODULE RootFormation ---------------------------
EXTENDS Naturals
CONSTANTS RFRoots, RFRoot, RFMaxTerm
ASSUME RFLegalInputs == /\ RFRoots # {} /\ RFRoots \subseteq Nat \ {0}
                       /\ RFRoot \in RFRoots /\ RFMaxTerm \in Nat \ {0}

\* One current leader's formation protocol over a retained abstract Raft log.
\* Election preserves committed history, and Drain commits/applies the retained
\* prefix through a current-term barrier. These are explicit consensus assumptions.
\* Root/log/catalog durability and exact-store authority are interface assumptions
\* discharged separately by the storage and store-lifecycle contracts.
VARIABLES rfTerm, rfPlan, rfLog, rfCommitted, rfApplied, rfBarrier, rfPending,
          rfLive, rfLocal, rfOwned, rfCanForm, rfMarker, rfUser
rfVars == <<rfTerm, rfPlan, rfLog, rfCommitted, rfApplied, rfBarrier, rfPending,
            rfLive, rfLocal, rfOwned, rfCanForm, rfMarker, rfUser>>
\* Even the first synchronized ConfState is non-pristine Raft history.
RFRaftSeen == TRUE
RFInit == /\ rfTerm = 1 /\ rfPlan = 0 /\ rfLog = 0 /\ rfCommitted = 0 /\ rfApplied = 0
          /\ rfBarrier = FALSE /\ rfPending = FALSE /\ rfLive = TRUE
          /\ rfLocal = RFRoot /\ rfOwned = TRUE /\ rfCanForm = TRUE
          /\ rfMarker = FALSE /\ rfUser = FALSE
RFRecover(local, owned) ==
    /\ ~rfLive /\ local \in RFRoots /\ owned \in BOOLEAN
    /\ rfLocal' = local /\ rfOwned' = owned
    /\ rfLive' = (local = RFRoot /\ owned)
    /\ rfCanForm' = (local = RFRoot /\ owned /\ ~rfMarker)
    /\ rfBarrier' = FALSE /\ rfPlan' = 0 /\ rfPending' = FALSE
    /\ UNCHANGED <<rfTerm, rfLog, rfCommitted, rfApplied, rfMarker, rfUser>>
RFCrash == /\ rfLive /\ rfLive' = FALSE /\ rfCanForm' = FALSE
           /\ rfBarrier' = FALSE /\ rfPlan' = 0 /\ rfPending' = FALSE
           /\ UNCHANGED <<rfTerm, rfLog, rfCommitted, rfApplied, rfLocal, rfOwned, rfMarker, rfUser>>
RFElect(keep) == /\ rfLive /\ rfTerm < RFMaxTerm /\ keep \in {rfCommitted, rfLog}
                /\ rfTerm' = rfTerm + 1 /\ rfLog' = keep /\ rfBarrier' = FALSE
                /\ UNCHANGED <<rfPlan, rfCommitted, rfApplied, rfPending, rfLive,
                               rfLocal, rfOwned, rfCanForm, rfMarker, rfUser>>
RFDrain == /\ rfLive /\ (~rfBarrier \/ rfPending)
           /\ rfCommitted' = rfLog /\ rfApplied' = rfLog /\ rfBarrier' = TRUE
           /\ rfPlan' = 0 /\ rfPending' = FALSE
           /\ UNCHANGED <<rfTerm, rfLog, rfLive, rfLocal, rfOwned, rfCanForm, rfMarker, rfUser>>
RFPlan == /\ rfLive /\ rfCanForm /\ rfPlan = 0 /\ ~rfPending
          /\ rfBarrier /\ rfApplied = 0 /\ rfPlan' = rfTerm
          /\ UNCHANGED <<rfTerm, rfLog, rfCommitted, rfApplied, rfBarrier, rfPending,
                         rfLive, rfLocal, rfOwned, rfCanForm, rfMarker, rfUser>>
RFAppend == /\ rfLive /\ rfPlan # 0 /\ rfPlan = rfTerm /\ ~rfPending
            /\ rfLog' = rfLocal /\ rfPending' = TRUE /\ rfUser' = FALSE
            /\ UNCHANGED <<rfTerm, rfPlan, rfCommitted, rfApplied, rfBarrier, rfLive,
                           rfLocal, rfOwned, rfCanForm, rfMarker>>
RFCommit == /\ rfLive /\ rfLog = RFRoot /\ rfCommitted' = RFRoot
            /\ UNCHANGED <<rfTerm, rfPlan, rfLog, rfApplied, rfBarrier, rfPending,
                           rfLive, rfLocal, rfOwned, rfCanForm, rfMarker, rfUser>>
RFMark == /\ rfLive /\ rfApplied = RFRoot /\ rfMarker' = TRUE /\ rfCanForm' = FALSE
          /\ UNCHANGED <<rfTerm, rfPlan, rfLog, rfCommitted, rfApplied, rfBarrier, rfPending,
                         rfLive, rfLocal, rfOwned, rfUser>>
RFWriteUser == /\ rfLive /\ rfMarker /\ rfApplied = RFRoot /\ rfUser' = TRUE
               /\ UNCHANGED <<rfTerm, rfPlan, rfLog, rfCommitted, rfApplied, rfBarrier, rfPending,
                              rfLive, rfLocal, rfOwned, rfCanForm, rfMarker>>
RFQuiesce == UNCHANGED rfVars
RFElectAny == \E keep \in {rfCommitted, rfLog} : RFElect(keep)
RFNext == (\E local \in RFRoots, owned \in BOOLEAN : RFRecover(local, owned))
          \/ RFElectAny
          \/ RFCrash \/ RFDrain \/ RFPlan \/ RFAppend \/ RFCommit \/ RFMark \/ RFWriteUser \/ RFQuiesce
RFSpec == RFInit /\ [][RFNext]_rfVars
RFType == /\ rfTerm \in 1..RFMaxTerm /\ rfPlan \in 0..rfTerm
          /\ rfLog \in {0, RFRoot} /\ rfCommitted \in {0, RFRoot} /\ rfApplied \in {0, RFRoot}
          /\ rfLocal \in RFRoots /\ rfOwned \in BOOLEAN /\ rfLive \in BOOLEAN
          /\ rfBarrier \in BOOLEAN /\ rfPending \in BOOLEAN /\ rfCanForm \in BOOLEAN
          /\ rfMarker \in BOOLEAN /\ rfUser \in BOOLEAN
RFHistory == /\ (rfCommitted = RFRoot => rfLog = RFRoot)
             /\ (rfApplied = RFRoot => rfCommitted = RFRoot)
             /\ (rfMarker => rfApplied = RFRoot)
             /\ (rfUser => rfCommitted = RFRoot)
RFAuthority == /\ (rfLive => rfLocal = RFRoot /\ rfOwned)
               /\ (rfCanForm => rfLive /\ ~rfMarker)
               /\ (~rfLive => rfPlan = 0 /\ ~rfPending /\ ~rfBarrier)
RFCuts == /\ (rfPending => rfPlan > 0)
          /\ (rfBarrier /\ ~rfPending => rfApplied = rfLog)
          /\ (rfBarrier /\ rfPlan > 0 => rfPlan = rfTerm)
          /\ (rfPlan = rfTerm /\ rfPending => rfLog = RFRoot)
RFFreshPlan == rfPlan = rfTerm /\ ~rfPending =>
              rfLive /\ rfCanForm /\ rfBarrier /\ rfApplied = 0 /\ rfLog = 0
RFInvariant == RFType /\ RFHistory /\ RFAuthority /\ RFCuts /\ RFFreshPlan
RFAppendFresh == RFAppend => rfLog = 0 /\ rfPlan = rfTerm /\ rfLive /\ rfOwned /\ rfLocal = RFRoot
RFUserPreserved == rfUser => rfUser'
RFStepSafety == RFAppendFresh /\ RFUserPreserved
RFSafety == [][RFStepSafety]_rfVars
RFServing == rfLive /\ rfMarker /\ rfApplied = RFRoot

\* A stable continuation of the exact original disk, with an available Raft
\* leader/quorum and successful I/O abstracted by fair Drain/Append actions.
RFRestore == RFRecover(RFRoot, TRUE)
RFStableNext == RFRestore \/ RFDrain \/ RFPlan \/ RFAppend \/ RFCommit \/ RFMark \/ RFWriteUser \/ RFQuiesce
RFCrashed == RFInvariant /\ ~rfLive /\ rfLocal = RFRoot /\ rfOwned
RFFairSpec == RFCrashed /\ [][RFStableNext]_rfVars
              /\ WF_rfVars(RFRestore) /\ WF_rfVars(RFDrain) /\ WF_rfVars(RFPlan)
              /\ WF_rfVars(RFAppend) /\ WF_rfVars(RFMark)
RFReady == RFInvariant /\ rfLive /\ (rfLog = 0 => rfCanForm)
RFBarrierReady == RFReady /\ rfBarrier
RFSeedReady == RFBarrierReady /\ (rfLog = RFRoot \/ rfPlan = rfTerm)
RFLogged == RFReady /\ rfLog = RFRoot
RFApplied == RFLogged /\ rfApplied = RFRoot
RFFinished == RFApplied /\ rfMarker
RFProgress == <>RFServing
RFNoUser == ~rfUser
RFNoUnappliedRoot == ~(rfCommitted = RFRoot /\ rfApplied = 0)
=============================================================================
