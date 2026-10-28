---------------------- MODULE CheckpointUnpublished ----------------------
EXTENDS Naturals
CONSTANT Tokens
ASSUME Tokens # {}
\* Each token is one immutable root, pending, executor ID and generation.
\* Another generation is another token; no action resets an old projection.
VARIABLES closed, phase, published, receipts, used, retired, covered
vars == <<closed, phase, published, receipts, used, retired, covered>>
Phases == {"Absent", "Held", "Published", "Quiesced", "Released"}
Init == /\ closed = FALSE /\ covered = TRUE
        /\ phase = [t \in Tokens |-> "Absent"]
        /\ published = {} /\ receipts = {} /\ used = {} /\ retired = {}
Enter(t) == /\ ~closed /\ phase[t] = "Absent"
            /\ phase' = [phase EXCEPT ![t] = "Held"]
            /\ UNCHANGED <<closed, published, receipts, used, retired, covered>>
Publish(t) == /\ ~closed /\ phase[t] \in {"Held", "Published"}
              /\ phase' = [phase EXCEPT ![t] = "Published"]
              /\ published' = published \cup {t}
              /\ UNCHANGED <<closed, receipts, used, retired, covered>>
Receipt(t) == /\ t \in published /\ receipts' = receipts \cup {t}
              /\ UNCHANGED <<closed, phase, published, used, retired, covered>>
Use(t) == /\ t \in receipts /\ used' = used \cup {t}
          /\ UNCHANGED <<closed, phase, published, receipts, retired, covered>>
Close == /\ closed' = TRUE
         /\ UNCHANGED <<phase, published, receipts, used, retired, covered>>
Retire(t) == /\ closed /\ phase[t] = "Held"
             /\ phase' = [phase EXCEPT ![t] = "Released"]
             /\ retired' = retired \cup {t}
             /\ UNCHANGED <<closed, published, receipts, used, covered>>
Drain(t) == /\ phase[t] \in {"Held", "Published"}
            /\ phase' = [phase EXCEPT ![t] = "Quiesced"]
            /\ UNCHANGED <<closed, published, receipts, used, retired, covered>>
Release(t) == /\ phase[t] = "Quiesced"
              /\ phase' = [phase EXCEPT ![t] = "Released"]
              /\ UNCHANGED <<closed, published, receipts, used, retired, covered>>
Cancel(t) == /\ phase[t] = "Absent"
             /\ phase' = [phase EXCEPT ![t] = "Released"]
             /\ UNCHANGED <<closed, published, receipts, used, retired, covered>>
Restart == /\ receipts' = {} /\ UNCHANGED <<closed, phase, published, used, retired, covered>>
Next == Close \/ Restart \/ (\E t \in Tokens : Enter(t) \/ Publish(t) \/ Receipt(t)
        \/ Use(t) \/ Retire(t) \/ Drain(t) \/ Release(t) \/ Cancel(t))
Spec == Init /\ [][Next]_vars
TypeOK == /\ closed \in BOOLEAN /\ covered \in BOOLEAN /\ phase \in [Tokens -> Phases]
          /\ published \subseteq Tokens /\ receipts \subseteq Tokens
          /\ used \subseteq Tokens /\ retired \subseteq Tokens
Invariant == /\ TypeOK /\ covered /\ (receipts \cup used) \subseteq published
             /\ \A t \in Tokens : phase[t] \in {"Absent", "Held"} => t \notin published
             /\ \A t \in retired : closed /\ phase[t] = "Released"
                                   /\ t \notin (published \cup receipts \cup used)
\* A delayed receipt from a published token may arrive AFTER close or release.
\* It cannot arrive for a retired token. Global coverage is never removed.
=======================================================================
