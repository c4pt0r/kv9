------------------------ MODULE SplitPopulation ------------------------
EXTENDS Naturals
VARIABLES owned, sealed, ttl, value, copiedAfterBinding
vars == <<owned, sealed, ttl, value, copiedAfterBinding>>
Init == /\ ~owned /\ ~sealed /\ ~ttl /\ value = 0 /\ ~copiedAfterBinding
Copy(v) == /\ ~owned /\ ~ttl /\ value' = v
           /\ copiedAfterBinding' = copiedAfterBinding \/ owned
           /\ UNCHANGED <<owned, sealed, ttl>>
Client(v) == /\ owned /\ ~sealed /\ value' = v
             /\ UNCHANGED <<owned, sealed, ttl, copiedAfterBinding>>
Bind == /\ owned' = TRUE /\ UNCHANGED <<sealed, ttl, value, copiedAfterBinding>>
Seal == /\ owned /\ sealed' = TRUE /\ UNCHANGED <<owned, ttl, value, copiedAfterBinding>>
Enable == /\ owned /\ ~sealed /\ ttl' = TRUE /\ UNCHANGED <<owned, sealed, value, copiedAfterBinding>>
Next == Bind \/ Seal \/ Enable \/ (\E v \in 0..2 : Copy(v) \/ Client(v))
Spec == Init /\ [][Next]_vars
Invariant == ~copiedAfterBinding
OwnershipNeverRegresses == [][owned => owned']_vars
SealedValueNeverChanges == [][(owned /\ sealed) => value' = value]_vars
\* Only the new copy command; legacy Write history and remote source authority
\* are excluded. Rejected copies stutter. No transfer completion/liveness claim.
=======================================================================
