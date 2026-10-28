------------------------- MODULE CheckpointSweep -------------------------
EXTENDS Naturals, FiniteSets
CONSTANT Attempts, Owners, Page
ASSUME /\ Attempts > 0 /\ Owners > 0 /\ Page > 0
A == 1..Attempts
O == 1..Owners
VARIABLES held, published, closed, frontier, done, seq, discovery, damaged, covered
vars == <<held, published, closed, frontier, done, seq, discovery, damaged, covered>>
\* All owners in this finite projection are indexed. Missing legacy backfill
\* and remote PUT termination are not inferred from this model.
Init == /\ held \in [A -> SUBSET O]
        /\ published \in [A -> SUBSET O]
        /\ \A a \in A : held[a] \intersect published[a] = {}
        /\ closed \in [A -> BOOLEAN]
        /\ frontier = [a \in A |-> 0] /\ seq = [a \in A |-> 0]
        /\ done = [a \in A |-> FALSE] /\ covered = TRUE
        /\ discovery = 0 /\ damaged \subseteq A
Discover == /\ discovery' = IF discovery = Attempts THEN 0 ELSE discovery + 1
            /\ UNCHANGED <<held, published, closed, frontier, done, seq, damaged, covered>>
Close(a) == /\ closed' = [closed EXCEPT ![a] = TRUE]
            /\ UNCHANGED <<held, published, frontier, done, seq, discovery, damaged, covered>>
Retire(a, i) == /\ discovery = a /\ a \notin damaged /\ closed[a]
                /\ i \in held[a] /\ frontier[a] < i /\ i <= frontier[a] + Page
                /\ held' = [held EXCEPT ![a] = @ \ {i}]
                /\ UNCHANGED <<published, closed, frontier, done, seq, discovery, damaged, covered>>
Advance(a) == LET end == IF frontier[a] + Page >= Owners THEN Owners ELSE frontier[a] + Page
              IN /\ discovery = a /\ a \notin damaged /\ closed[a] /\ ~done[a]
                 /\ (held[a] \intersect (frontier[a]+1..end)) = {}
                 /\ frontier' = [frontier EXCEPT ![a] = end]
                 /\ seq' = [seq EXCEPT ![a] = @ + 1]
                 /\ done' = [done EXCEPT ![a] = (end = Owners)]
                 /\ UNCHANGED <<held, published, closed, discovery, damaged, covered>>
\* Volatile discovery is lost; committed per-attempt progress is retained.
Restart == /\ discovery' = 0
           /\ UNCHANGED <<held, published, closed, frontier, done, seq, damaged, covered>>
Next == Discover \/ Restart \/ (\E a \in A : Close(a) \/ Advance(a) \/ (\E i \in O : Retire(a, i)))
Spec == Init /\ [][Next]_vars
TypeOK == /\ held \in [A -> SUBSET O] /\ published \in [A -> SUBSET O]
          /\ closed \in [A -> BOOLEAN] /\ frontier \in [A -> 0..Owners]
          /\ done \in [A -> BOOLEAN] /\ seq \in [A -> Nat]
          /\ discovery \in 0..Attempts /\ damaged \subseteq A /\ covered \in BOOLEAN
Invariant == /\ TypeOK /\ covered
             /\ \A a \in A : (held[a] \intersect (1..frontier[a])) = {}
             /\ \A a \in A : done[a] => closed[a] /\ held[a] = {}
\* Published may remain after Done. No Pending/Version/object deletion action.
\* No fairness/liveness claim: an endlessly changing leader can delay discovery.
===========================================================================
