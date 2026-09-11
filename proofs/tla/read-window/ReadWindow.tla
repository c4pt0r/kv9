----------------------------- MODULE ReadWindow -----------------------------
EXTENDS Naturals

\* Counted local admission window. Each guarded submission can add at most
\* one upstream context; singleton immediate confirmation can leave no entry.
\* Upstream acknowledgement/configuration reevaluation can remove a prefix,
\* and a term reset can clear the queue. Caller cancellation never removes it.
\* These are source-mapping premises, not a proof of raft-rs queue internals.
\* Remote MsgReadIndex currently enters a different upstream step path. The
\* always-bounded theorem requires no such unmodeled queue-increasing step.
CONSTANT RWCapacity
VARIABLE rwPending
RWLegal == RWCapacity \in Nat \ {0}
RWInvariant == RWLegal /\ rwPending \in 0..RWCapacity
RWInit == RWLegal /\ rwPending = 0
RWAdmit == /\ rwPending < RWCapacity
           /\ rwPending' \in {rwPending, rwPending + 1}
RWRelease == rwPending' \in 0..rwPending
RWReset == rwPending' = 0
RWCancel == UNCHANGED rwPending
RWNext == RWAdmit \/ RWRelease \/ RWReset \/ RWCancel
RWSpec == RWInit /\ [][RWNext]_rwPending
RWAdmissionEffect == RWAdmit => rwPending < RWCapacity
RWAdmissionEffects == [][RWAdmissionEffect]_rwPending
RWCancellationEffects == [][RWCancel => UNCHANGED rwPending]_rwPending
=============================================================================
