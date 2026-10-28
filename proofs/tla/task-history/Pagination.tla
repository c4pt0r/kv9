----------------------------- MODULE Pagination -----------------------------
EXTENDS Naturals, Sequences

CONSTANTS History, Future, PageSize
ASSUME /\ PageSize > 0
       /\ History \in Seq(Nat)
       /\ Future \in Seq(Nat)

VARIABLES snapshot, live, cursor, emitted, buffer, done
vars == <<snapshot, live, cursor, emitted, buffer, done>>

Init == /\ snapshot = History
        /\ live = History
        /\ cursor = 0
        /\ emitted = <<>>
        /\ buffer = <<>>
        /\ done = FALSE

Limit == IF cursor + PageSize <= Len(snapshot)
         THEN cursor + PageSize ELSE Len(snapshot)
Load == /\ buffer = <<>> /\ ~done
        /\ buffer' = SubSeq(snapshot, cursor + 1, Limit)
        /\ cursor' = Limit
        /\ done' = (Limit - cursor < PageSize)
        /\ UNCHANGED <<snapshot, live, emitted>>
Yield == /\ buffer # <<>>
         /\ emitted' = Append(emitted, Head(buffer))
         /\ buffer' = Tail(buffer)
         /\ UNCHANGED <<snapshot, live, cursor, done>>
CommitElsewhere == /\ live' = Future
                   /\ UNCHANGED <<snapshot, cursor, emitted, buffer, done>>
Next == Load \/ Yield \/ CommitElsewhere
Spec == Init /\ [][Next]_vars /\ WF_vars(Load) /\ WF_vars(Yield)

Safe == /\ snapshot = History
        /\ cursor \in 0..Len(snapshot)
        /\ Len(buffer) <= PageSize
        /\ emitted \o buffer = SubSeq(snapshot, 1, cursor)
        /\ (done => cursor = Len(snapshot))
        /\ ((done /\ buffer = <<>>) => emitted = snapshot)
Completes == <>(done /\ buffer = <<>>)
=============================================================================
