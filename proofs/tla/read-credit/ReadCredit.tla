----------------------------- MODULE ReadCredit -----------------------------
EXTENDS ReadAdmission

\* One invocation projects to the existing admission protocol. rcOccupied
\* abstracts actual upstream pending_read_count() >= the configured bound,
\* not registry lifetime. Multiple contexts can underlie this full/free bit;
\* changes that do not cross the threshold can project to stuttering.
\* Other protocol requests may fill the window. Upstream advance (including
\* configuration-quorum reevaluation) or reset can release it. Neither this
\* abstraction nor its projection proves upstream Raft certification.
\* An admitted singleton request can return a ReadState immediately without
\* entering the pending queue. Admission therefore permits either occupancy
\* post-state, overapproximating both singleton and multi-voter behavior.
VARIABLES rcOccupied, rcCanceled
rcVars == <<raVars, rcOccupied, rcCanceled>>
RCInit == RAInit /\ rcOccupied \in BOOLEAN /\ rcCanceled = FALSE
RCInvariant == RAInvariant /\ rcOccupied \in BOOLEAN /\ rcCanceled \in BOOLEAN

RCPoll == /\ raPhase = "Waiting" /\ ~rcCanceled
          /\ IF ~raLeader \/ raCommitTerm # raTerm \/ ~rcOccupied
             THEN /\ RAPoll
                  /\ IF raPhase' = "Submitted"
                     THEN rcOccupied' \in BOOLEAN
                     ELSE UNCHANGED rcOccupied
             ELSE UNCHANGED <<raVars, rcOccupied>>
          /\ UNCHANGED rcCanceled
RCReadStep == /\ RACommit \/ RACertify \/ (\E c \in {RAOwn, RAOther} : RADeliver(c))
                 \/ RACatchUp \/ (RAComplete /\ ~rcCanceled) \/ RATick \/ RATimeout
              /\ UNCHANGED <<rcOccupied, rcCanceled>>
RCReset == /\ (\E l \in BOOLEAN : RAAdvance(l)) \/ RALose
           /\ rcOccupied' = FALSE /\ UNCHANGED rcCanceled
RCOtherAdmission == /\ ~rcOccupied /\ rcOccupied' = TRUE
                    /\ UNCHANGED <<raVars, rcCanceled>>
RCRelease == /\ rcOccupied /\ rcOccupied' = FALSE
             /\ UNCHANGED <<raVars, rcCanceled>>
\* Cancellation stops this caller's observation/admission; it retracts no
\* upstream protocol request. Its original protocol state simply projects
\* to stuttering. Deadline expiration also leaves occupancy unchanged.
RCCancel == /\ ~rcCanceled /\ rcCanceled' = TRUE
            /\ UNCHANGED <<raVars, rcOccupied>>
RCNext == RCPoll \/ RCReadStep \/ RCReset \/ RCOtherAdmission \/ RCRelease \/ RCCancel
RCSpec == RCInit /\ [][RCNext]_rcVars
RCAdmissionHasCredit == (RCPoll /\ raPhase' = "Submitted") => ~rcOccupied /\ ~rcCanceled
RCAdmissionEffects == [][RCAdmissionHasCredit]_rcVars
RCCancellationEffects == [][RCCancel => UNCHANGED rcOccupied]_rcVars

\* Conditional admission progress in a stable, current-term-ready window:
\* no competing refill, cancellation, election, or expiring budget. Fair
\* upstream release plus fair polling makes the slot persistently available.
\* This does not claim per-caller fairness under arbitrary competing traffic,
\* full read completion, or any wall-clock response bound.
RCWaiting == RCInvariant /\ raLeader /\ raCommitTerm = raTerm
             /\ raPhase = "Waiting" /\ ~rcCanceled /\ raBudget > 0
RCBlocked == RCWaiting /\ rcOccupied
RCFree == RCWaiting /\ ~rcOccupied
RCAdmitted == raSubmits = 1
RCProgressNext == RCPoll \/ RCRelease
RCProgressFairness == WF_rcVars(RCRelease) /\ WF_rcVars(RCPoll)
RCProgressSpec == RCWaiting /\ [][RCProgressNext]_rcVars
                  /\ RCProgressFairness
=============================================================================
