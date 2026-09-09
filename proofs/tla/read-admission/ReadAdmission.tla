----------------------------- MODULE ReadAdmission -----------------------------
EXTENDS Naturals
CONSTANTS RAOwn, RAOther, RABudget
ASSUME RALegalInputs == RAOwn \in Nat \ {0} /\ RAOther \in Nat \ {0}
                        /\ RAOwn # RAOther /\ RABudget \in Nat \ {0}
\* One invocation with a unique context. Raft quorum certification and contiguous
\* application are explicit upstream assumptions, not a proof of consensus here.
VARIABLES raTerm, raCommitTerm, raLeader, raPhase, raContext, raSubmits,
          raAdmittedTerm, raAdmittedCommitTerm, raQuorum, raReceipt, raApplied, raBudget
RAEnv == <<raTerm, raCommitTerm, raLeader>>
RARequest == <<raContext, raSubmits, raAdmittedTerm, raAdmittedCommitTerm>>
raVars == <<RAEnv, raPhase, RARequest, raQuorum, raReceipt, raApplied, raBudget>>
RAPhases == {"Waiting", "Submitted", "Confirmed", "Done", "Refused"}
RAActive == raPhase \notin {"Done", "Refused"}
RAInit == /\ raTerm = 1 /\ raCommitTerm = 0 /\ raLeader = TRUE
          /\ raPhase = "Waiting" /\ raContext = RAOwn /\ raSubmits = 0
          /\ raAdmittedTerm = 0 /\ raAdmittedCommitTerm = 0
          /\ raQuorum = FALSE /\ raReceipt = 0 /\ raApplied = FALSE /\ raBudget = RABudget
RAPoll == /\ raPhase = "Waiting"
          /\ IF ~raLeader
             THEN /\ raPhase' = "Refused" /\ UNCHANGED RARequest
             ELSE IF raCommitTerm = raTerm
                  THEN /\ raPhase' = "Submitted" /\ raSubmits' = raSubmits + 1
                       /\ raAdmittedTerm' = raTerm /\ raAdmittedCommitTerm' = raCommitTerm
                       /\ UNCHANGED raContext
                  ELSE UNCHANGED <<raPhase, RARequest>>
          /\ UNCHANGED <<RAEnv, raQuorum, raReceipt, raApplied, raBudget>>
RACommit == /\ raLeader /\ raCommitTerm # raTerm /\ raCommitTerm' = raTerm
            /\ UNCHANGED <<raTerm, raLeader, raPhase, RARequest, raQuorum, raReceipt, raApplied, raBudget>>
RAAdvance(l) == /\ l \in BOOLEAN /\ raTerm' = raTerm + 1 /\ raLeader' = l
                /\ UNCHANGED <<raCommitTerm, raPhase, RARequest, raQuorum, raReceipt, raApplied, raBudget>>
RALose == /\ raLeader /\ raLeader' = FALSE
          /\ UNCHANGED <<raTerm, raCommitTerm, raPhase, RARequest, raQuorum, raReceipt, raApplied, raBudget>>
\* Quorum evidence can be published after a later term change, provided it was
\* actually established for this accepted context in its submission term.
RACertify == /\ raPhase = "Submitted" /\ raSubmits = 1 /\ raLeader
             /\ raTerm = raAdmittedTerm /\ raCommitTerm = raTerm
             /\ raQuorum' = TRUE
             /\ UNCHANGED <<RAEnv, raPhase, RARequest, raReceipt, raApplied, raBudget>>
RADeliver(c) == /\ raPhase = "Submitted" /\ c \in {RAOwn, RAOther}
                /\ (c = RAOther \/ raQuorum)
                /\ IF c = RAOwn
                   THEN /\ raPhase' = "Confirmed" /\ raReceipt' = c
                   ELSE UNCHANGED <<raPhase, raReceipt>>
                /\ UNCHANGED <<RAEnv, RARequest, raQuorum, raApplied, raBudget>>
\* raApplied abstracts the unified, contiguous watermark covering the index
\* carried by the exact receipt; readiness alone does not establish this fact.
RACatchUp == /\ raPhase = "Confirmed" /\ ~raApplied /\ raApplied' = TRUE
             /\ UNCHANGED <<RAEnv, raPhase, RARequest, raQuorum, raReceipt, raBudget>>
RAComplete == /\ raPhase = "Confirmed" /\ raApplied
              /\ raPhase' = "Done"
              /\ UNCHANGED <<RAEnv, RARequest, raQuorum, raReceipt, raApplied, raBudget>>
RATick == /\ RAActive /\ raBudget > 0 /\ raBudget' = raBudget - 1
          /\ UNCHANGED <<RAEnv, raPhase, RARequest, raQuorum, raReceipt, raApplied>>
RATimeout == /\ RAActive /\ raBudget = 0 /\ raPhase' = "Refused"
             /\ UNCHANGED <<RAEnv, RARequest, raQuorum, raReceipt, raApplied, raBudget>>
RANext == RAPoll \/ RACommit \/ (\E l \in BOOLEAN : RAAdvance(l)) \/ RALose
          \/ RACertify \/ (\E c \in {RAOwn, RAOther} : RADeliver(c))
          \/ RACatchUp \/ RAComplete \/ RATick \/ RATimeout
RASpec == RAInit /\ [][RANext]_raVars
RAType == /\ raTerm \in Nat \ {0} /\ raCommitTerm \in 0..raTerm /\ raLeader \in BOOLEAN
          /\ raPhase \in RAPhases /\ raContext \in {RAOwn, RAOther} /\ raSubmits \in 0..1
          /\ raAdmittedTerm \in 0..raTerm /\ raAdmittedCommitTerm \in 0..raCommitTerm
          /\ raQuorum \in BOOLEAN /\ raReceipt \in {0, RAOwn, RAOther}
          /\ raApplied \in BOOLEAN /\ raBudget \in 0..RABudget
RABinding == /\ raContext = RAOwn
             /\ (raSubmits = 0 => /\ raAdmittedTerm = 0 /\ raAdmittedCommitTerm = 0
                                  /\ ~raQuorum /\ raReceipt = 0 /\ ~raApplied
                                  /\ raPhase \in {"Waiting", "Refused"})
             /\ (raSubmits = 1 => /\ raAdmittedTerm > 0 /\ raAdmittedTerm = raAdmittedCommitTerm
                                  /\ raPhase \in {"Submitted", "Confirmed", "Done", "Refused"})
             /\ (raQuorum => raSubmits = 1)
             /\ (raReceipt # 0 => /\ raReceipt = RAOwn /\ raQuorum
                                   /\ raPhase \in {"Confirmed", "Done", "Refused"})
             /\ (raPhase \in {"Confirmed", "Done"} => raReceipt = RAOwn /\ raQuorum /\ raSubmits = 1)
             /\ (raApplied => raReceipt = RAOwn)
             /\ (raPhase = "Done" => raApplied)
RAInvariant == RAType /\ RABinding
RAAdmission == (RAPoll /\ raPhase' = "Submitted") => raLeader /\ raCommitTerm = raTerm
RAEffects == [][RAAdmission /\ UNCHANGED raContext /\ raBudget' <= raBudget]_raVars
RASafeReturn == raPhase = "Done" => raReceipt = RAOwn /\ raQuorum /\ raApplied /\ raSubmits = 1
\* Conditional success requires stable leadership, eventual current-term commit,
\* fair admission/certification/delivery/application, and sufficient caller budget.
\* It does not assert a wall-clock latency bound or success during ongoing elections.
RAStableNext == RAPoll \/ RACommit \/ RACertify \/ RADeliver(RAOwn) \/ RADeliver(RAOther)
                \/ RACatchUp \/ RAComplete
RAStableInvariant == RAInvariant /\ raLeader /\ raTerm = 1
RAStableFairness == /\ WF_raVars(RACommit) /\ WF_raVars(RAPoll) /\ WF_raVars(RACertify)
                    /\ WF_raVars(RADeliver(RAOwn)) /\ WF_raVars(RACatchUp) /\ WF_raVars(RAComplete)
RAStableSpec == RAInit /\ [][RAStableNext]_raVars /\ RAStableFairness
RAStage0 == RAStableInvariant /\ raPhase = "Waiting" /\ raCommitTerm = 0
RAStage1 == RAStableInvariant /\ raPhase = "Waiting" /\ raCommitTerm = 1
RAStage2 == RAStableInvariant /\ raPhase = "Submitted" /\ ~raQuorum
RAStage3 == RAStableInvariant /\ raPhase = "Submitted" /\ raQuorum
RAStage4 == RAStableInvariant /\ raPhase = "Confirmed" /\ ~raApplied
RAStage5 == RAStableInvariant /\ raPhase = "Confirmed" /\ raApplied
RAStage6 == RAStableInvariant /\ raPhase = "Done"
RASuccess == <>(raPhase = "Done")
RANoReelection == raTerm < 3
RANoReturn == raPhase # "Done"
=============================================================================
