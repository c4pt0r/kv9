----------------------- MODULE CheckpointIntent -----------------------
EXTENDS Naturals
CONSTANTS Operations, Descriptors, Canonical, Missing
ASSUME /\ Operations # {} /\ Descriptors # {}
       /\ Missing \notin Descriptors
       /\ Canonical \in [Operations -> Descriptors]
\* Each operation binds the exact root, scope, predecessor, canonical manifest
\* and full SST closure. Hash binding, atomic Raft apply and fresh receipts are
\* composition premises, not conclusions of this descriptor archive model.
VARIABLES local, protected, archived, confirmed, used, settled, cleared
vars == <<local, protected, archived, confirmed, used, settled, cleared>>
Init == /\ local = {} /\ protected = {}
        /\ archived = [o \in Operations |-> Missing]
        /\ confirmed = {} /\ used = {} /\ settled = {} /\ cleared = {}
Plan(o) == /\ local' = local \cup {o}
           /\ UNCHANGED <<protected, archived, confirmed, used, settled, cleared>>
Protect(o) == /\ o \in local /\ protected' = protected \cup {o}
              /\ UNCHANGED <<local, archived, confirmed, used, settled, cleared>>
Record(o, bytes) == /\ o \in protected /\ bytes = Canonical[o]
                   /\ archived[o] \in {Missing, bytes}
                   /\ archived' = [archived EXCEPT ![o] = bytes]
                   /\ UNCHANGED <<local, protected, confirmed, used, settled, cleared>>
Receipt(o) == /\ archived[o] = Canonical[o] /\ o \in protected
              /\ confirmed' = confirmed \cup {o}
              /\ UNCHANGED <<local, protected, archived, used, settled, cleared>>
Use(o) == /\ o \in local \cap confirmed \cap protected
          /\ used' = used \cup {o}
          /\ UNCHANGED <<local, protected, archived, confirmed, settled, cleared>>
Settle(o) == /\ o \in used /\ settled' = settled \cup {o}
             /\ UNCHANGED <<local, protected, archived, confirmed, used, cleared>>
Clear(o) == /\ o \in local \cap settled \cap confirmed
            /\ local' = local \ {o} /\ cleared' = cleared \cup {o}
            /\ UNCHANGED <<protected, archived, confirmed, used, settled>>
Restart == /\ confirmed' = {} /\ settled' = {}
           /\ UNCHANGED <<local, protected, archived, used, cleared>>
Next == \/ \E o \in Operations : Plan(o) \/ Protect(o) \/ Receipt(o) \/ Use(o) \/ Settle(o) \/ Clear(o)
        \/ \E o \in Operations, bytes \in Descriptors : Record(o, bytes)
        \/ Restart
Spec == Init /\ [][Next]_vars
TypeOK == /\ local \subseteq Operations /\ protected \subseteq Operations
          /\ archived \in [Operations -> Descriptors \cup {Missing}]
          /\ confirmed \subseteq Operations /\ used \subseteq Operations
          /\ settled \subseteq Operations /\ cleared \subseteq Operations
Exact == \A o \in Operations : archived[o] \in {Missing, Canonical[o]}
Discoverable == \A o \in confirmed \cup used \cup cleared : archived[o] = Canonical[o] /\ o \in protected
Invariant == TypeOK /\ Exact /\ Discoverable
\* No deletion, publisher-closure inference, or negative-release action exists.
\* Conditional progress requires surviving protection/bytes, a metadata quorum,
\* eventual successful retries/receipts and eventual settlement by its own
\* protocol. Continuous failures or repeated restarts do not promise progress.
=======================================================================
