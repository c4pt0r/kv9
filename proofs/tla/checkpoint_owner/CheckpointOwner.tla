------------------------ MODULE CheckpointOwner ------------------------
EXTENDS Naturals
CONSTANTS COps, CNone
ASSUME CInputs == /\ COps # {} /\ CNone \notin COps
\* Operations denote exact immutable root/scope/predecessor/descriptor/bytes.
\* Ledger receipts, validated atomic journal recovery, and positive/negative
\* seam evidence are explicit lower-layer composition premises.
VARIABLES cJournal, cUsed, cCleared, cPending, cVersion,
          cRemote, cPrepared, cEffects, cPositive, cNegative
cVars == <<cJournal,cUsed,cCleared,cPending,cVersion,cRemote,cPrepared,
           cEffects,cPositive,cNegative>>
CProtected(o) == cPending[o] = 2 \/ cVersion[o] = 2
CInit == /\ cJournal = CNone /\ cUsed = {} /\ cCleared = {}
         /\ cPending = [o \in COps |-> 0] /\ cVersion = [o \in COps |-> 0]
         /\ cRemote = {} /\ cPrepared = {} /\ cEffects = {}
         /\ cPositive = {} /\ cNegative = {}
CPlan(o) == /\ cJournal = CNone /\ o \notin cUsed
            /\ cJournal' = o /\ cUsed' = cUsed \cup {o}
            /\ UNCHANGED <<cCleared,cPending,cVersion,cRemote,cPrepared,cEffects,cPositive,cNegative>>
CAcquire(o) == /\ cJournal = o /\ cPending[o] = 0
               /\ cPending' = [cPending EXCEPT ![o] = 1]
               /\ UNCHANGED <<cJournal,cUsed,cCleared,cVersion,cRemote,cPrepared,cEffects,cPositive,cNegative>>
CPin(o) == /\ cJournal = o /\ cPending[o] = 1
           /\ cPending' = [cPending EXCEPT ![o] = 2]
           /\ UNCHANGED <<cJournal,cUsed,cCleared,cVersion,cRemote,cPrepared,cEffects,cPositive,cNegative>>
CPut(o) == /\ cJournal = o /\ CProtected(o)
           /\ cRemote' = cRemote \cup {o}
           /\ UNCHANGED <<cJournal,cUsed,cCleared,cPending,cVersion,cPrepared,cEffects,cPositive,cNegative>>
CVerify(o) == /\ cJournal = o /\ o \in cRemote
              /\ cPrepared' = cPrepared \cup {o}
              /\ UNCHANGED <<cJournal,cUsed,cCleared,cPending,cVersion,cRemote,cEffects,cPositive,cNegative>>
CApply(o) == /\ cJournal = o /\ o \in cPrepared
             /\ cEffects' = cEffects \cup {o}
             /\ UNCHANGED <<cJournal,cUsed,cCleared,cPending,cVersion,cRemote,cPrepared,cPositive,cNegative>>
CPositiveEvidence(o) == /\ cJournal = o /\ o \in cEffects
                        /\ cPositive' = cPositive \cup {o}
                        /\ UNCHANGED <<cJournal,cUsed,cCleared,cPending,cVersion,cRemote,cPrepared,cEffects,cNegative>>
CNegativeEvidence(o) == /\ cJournal = o /\ o \in cPrepared /\ o \notin cEffects
                        /\ cNegative' = cNegative \cup {o}
                        /\ UNCHANGED <<cJournal,cUsed,cCleared,cPending,cVersion,cRemote,cPrepared,cEffects,cPositive>>
CShare(o) == /\ cJournal = o /\ o \in cPositive /\ cPending[o] = 2 /\ cVersion[o] = 0
             /\ cVersion' = [cVersion EXCEPT ![o] = 1]
             /\ UNCHANGED <<cJournal,cUsed,cCleared,cPending,cRemote,cPrepared,cEffects,cPositive,cNegative>>
CPublishVersion(o) == /\ cJournal = o /\ cVersion[o] = 1
                      /\ cVersion' = [cVersion EXCEPT ![o] = 2]
                      /\ UNCHANGED <<cJournal,cUsed,cCleared,cPending,cRemote,cPrepared,cEffects,cPositive,cNegative>>
CQuiesce(o) == /\ cJournal = o /\ cPending[o] = 2 /\ cVersion[o] = 2
               /\ cPending' = [cPending EXCEPT ![o] = 3]
               /\ UNCHANGED <<cJournal,cUsed,cCleared,cVersion,cRemote,cPrepared,cEffects,cPositive,cNegative>>
CRelease(o) == /\ cJournal = o /\ cPending[o] = 3 /\ cVersion[o] = 2
               /\ cPending' = [cPending EXCEPT ![o] = 4]
               /\ UNCHANGED <<cJournal,cUsed,cCleared,cVersion,cRemote,cPrepared,cEffects,cPositive,cNegative>>
CClear(o) == /\ cJournal = o
             /\ (o \in cNegative \/ (o \in cPositive /\ cPending[o] = 4 /\ cVersion[o] = 2))
             /\ cJournal' = CNone /\ cCleared' = cCleared \cup {o}
             /\ UNCHANGED <<cUsed,cPending,cVersion,cRemote,cPrepared,cEffects,cPositive,cNegative>>
CRestart == UNCHANGED cVars
CNext == CRestart \/ \E o \in COps : CPlan(o) \/ CAcquire(o) \/ CPin(o) \/ CPut(o)
         \/ CVerify(o) \/ CApply(o) \/ CPositiveEvidence(o) \/ CNegativeEvidence(o)
         \/ CShare(o) \/ CPublishVersion(o) \/ CQuiesce(o) \/ CRelease(o) \/ CClear(o)
CSpec == CInit /\ [][CNext]_cVars
CType == /\ cJournal \in COps \cup {CNone} /\ cUsed \subseteq COps /\ cCleared \subseteq cUsed
         /\ cPending \in [COps -> 0..4] /\ cVersion \in [COps -> 0..2]
         /\ cRemote \subseteq COps /\ cPrepared \subseteq cRemote /\ cEffects \subseteq cPrepared
         /\ cPositive \subseteq cEffects /\ cNegative \subseteq cPrepared
CSlot == cUsed \ cCleared = IF cJournal = CNone THEN {} ELSE {cJournal}
CCovered == \A o \in cRemote : CProtected(o)
CTransfer == \A o \in COps : cPending[o] \in {3,4} => cVersion[o] = 2
CSettled == \A o \in cCleared : o \in cNegative \/ (o \in cPositive /\ cPending[o] = 4 /\ cVersion[o] = 2)
CInvariant == CType /\ CSlot /\ CCovered /\ CTransfer /\ CSettled
=============================================================================
