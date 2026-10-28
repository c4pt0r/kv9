---------------------- MODULE CheckpointAdmission ----------------------
EXTENDS Naturals
CONSTANT Executors
ASSUME Executors # {}
VARIABLES closed, phase, generation, entries, publications,
          fenceEntries, fencePublications, covered
vars == <<closed, phase, generation, entries, publications,
          fenceEntries, fencePublications, covered>>
Phases == {"Absent", "Held", "Published", "Quiesced", "Released"}
Init == /\ closed = FALSE /\ covered = TRUE
        /\ phase = [e \in Executors |-> "Absent"]
        /\ generation = [e \in Executors |-> 0]
        /\ entries = {} /\ publications = {}
        /\ fenceEntries = {} /\ fencePublications = {}
Eligible(e) == phase[e] \in {"Absent", "Released"}
Enter(e, g) == /\ ~closed /\ Eligible(e) /\ g = generation[e] + 1
               /\ generation' = [generation EXCEPT ![e] = g]
               /\ phase' = [phase EXCEPT ![e] = "Held"]
               /\ entries' = entries \cup {<<e, g>>}
               /\ UNCHANGED <<closed, publications, fenceEntries, fencePublications, covered>>
Publish(e) == /\ ~closed /\ phase[e] \in {"Held", "Published"}
              /\ phase' = [phase EXCEPT ![e] = "Published"]
              /\ publications' = publications \cup {<<e, generation[e]>>}
              /\ UNCHANGED <<closed, generation, entries, fenceEntries, fencePublications, covered>>
Close == /\ closed' = TRUE /\ fenceEntries' = entries
         /\ fencePublications' = publications
         /\ UNCHANGED <<phase, generation, entries, publications, covered>>
Cancel(e, g) == /\ Eligible(e) /\ g = generation[e] + 1
                /\ phase' = [phase EXCEPT ![e] = "Released"]
                /\ generation' = [generation EXCEPT ![e] = g]
                /\ UNCHANGED <<closed, entries, publications, fenceEntries, fencePublications, covered>>
Drain(e) == /\ phase[e] \in {"Held", "Published"} /\ covered
            /\ phase' = [phase EXCEPT ![e] = "Quiesced"]
            /\ UNCHANGED <<closed, generation, entries, publications, fenceEntries, fencePublications, covered>>
Release(e) == /\ phase[e] = "Quiesced"
              /\ phase' = [phase EXCEPT ![e] = "Released"]
              /\ UNCHANGED <<closed, generation, entries, publications, fenceEntries, fencePublications, covered>>
Next == Close \/ (\E e \in Executors : Enter(e, generation[e] + 1)
               \/ Publish(e) \/ Cancel(e, generation[e] + 1) \/ Drain(e) \/ Release(e))
Spec == Init /\ [][Next]_vars
TypeOK == /\ closed \in BOOLEAN /\ covered \in BOOLEAN
          /\ phase \in [Executors -> Phases] /\ generation \in [Executors -> Nat]
          /\ entries \subseteq (Executors \X Nat)
          /\ publications \subseteq entries
          /\ fenceEntries \subseteq entries /\ fencePublications \subseteq publications
Invariant == TypeOK /\ covered
             /\ (closed => (entries = fenceEntries /\ publications = fencePublications))
\* Exact retries, lost receipts and restart stutter on durable projection.
\* Global coverage has no release action. Closed is not remote I/O cancellation.
\* No liveness claim without a live quorum, retry fairness and actual local drain.
=======================================================================
