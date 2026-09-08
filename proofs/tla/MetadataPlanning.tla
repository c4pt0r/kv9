------------------------- MODULE MetadataPlanning -------------------------
EXTENDS Naturals, Sequences, FiniteSets

\* A catalog integration model over an abstract Raft log, not a Raft proof.
\* Elections preserve committed entries and may discard any uncommitted suffix.
CONSTANTS Nodes, Requests, RequestName, MaxTerm
ASSUME /\ IsFiniteSet(Nodes) /\ Nodes # {} /\ Nodes \subseteq Nat \ {0}
       /\ IsFiniteSet(Requests) /\ Requests # {} /\ Requests \subseteq Nat \ {0}
       /\ DOMAIN RequestName = Requests
       /\ MaxTerm \in Nat \ {0}
VARIABLES term, leader, log, committed, applied,
          phase, host, planningTerm, plannedId, barrierAt, writeAt, writeTerm, succeeded, unknownPending

vars == <<term, leader, log, committed, applied,
          phase, host, planningTerm, plannedId, barrierAt, writeAt, writeTerm, succeeded, unknownPending>>
Active == {"barrier", "plan", "ready", "write"}
Entry(kind, request, epoch, id) ==
    [kind |-> kind, request |-> request, epoch |-> epoch, id |-> id]
WritesThrough(cut) == {i \in 1..cut : log[i].kind = "write"}
NamesThrough(cut) == {RequestName[log[i].request] : i \in WritesThrough(cut)}
LastWrite(cut) == CHOOSE i \in WritesThrough(cut) :
    \A j \in WritesThrough(cut) : j <= i
\* A blind batch replaces the allocator with plannedId + 1. Do not compute
\* Cardinality(writes) here: that would silently repair duplicate allocation.
NextId(cut) == IF WritesThrough(cut) = {} THEN 1
               ELSE log[LastWrite(cut)].id + 1
Exact(at, kind, request, epoch) ==
    /\ at \in 1..Len(log)
    /\ log[at].kind = kind
    /\ log[at].request = request
    /\ log[at].epoch = epoch

Init ==
    /\ term = 1
    /\ leader \in Nodes
    /\ log = <<>>
    /\ committed = 0
    /\ applied = [n \in Nodes |-> 0]
    /\ phase = [r \in Requests |-> "new"]
    /\ host = [r \in Requests |-> 0]
    /\ planningTerm = [r \in Requests |-> 0]
    /\ plannedId = [r \in Requests |-> 0]
    /\ barrierAt = [r \in Requests |-> 0]
    /\ writeAt = [r \in Requests |-> 0]
    /\ writeTerm = [r \in Requests |-> 0]
    /\ succeeded = {}
    /\ unknownPending = {}

Begin(r) ==
    /\ phase[r] = "new"
    /\ \A other \in Requests :
           host[other] = leader => phase[other] \notin Active
    /\ host' = [host EXCEPT ![r] = leader]
    /\ planningTerm' = [planningTerm EXCEPT ![r] = term]
    /\ barrierAt' = [barrierAt EXCEPT ![r] = Len(log) + 1]
    /\ log' = Append(log, Entry("barrier", r, term, 0))
    /\ phase' = [phase EXCEPT ![r] = "barrier"]
    /\ UNCHANGED <<term, leader, committed, applied, plannedId, writeAt, writeTerm, succeeded, unknownPending>>

Barrier(r) ==
    /\ phase[r] = "barrier"
    /\ Exact(barrierAt[r], "barrier", r, planningTerm[r])
    /\ applied[host[r]] >= barrierAt[r]
    /\ phase' = [phase EXCEPT ![r] = "plan"]
    /\ UNCHANGED <<term, leader, log, committed, applied, host,
                   planningTerm, plannedId, barrierAt, writeAt, writeTerm, succeeded, unknownPending>>

Plan(r) ==
    /\ phase[r] = "plan"
    /\ IF RequestName[r] \in NamesThrough(applied[host[r]])
          THEN /\ phase' = [phase EXCEPT ![r] = "done"]
               /\ UNCHANGED plannedId
          ELSE /\ plannedId' = [plannedId EXCEPT ![r] = NextId(applied[host[r]])]
               /\ phase' = [phase EXCEPT ![r] = "ready"]
    /\ UNCHANGED <<term, leader, log, committed, applied, host,
                   planningTerm, barrierAt, writeAt, writeTerm, succeeded, unknownPending>>

Submit(r) ==
    /\ phase[r] = "ready"
    /\ host[r] = leader
    /\ planningTerm[r] = term
    /\ log' = Append(log, Entry("write", r, term, plannedId[r]))
    /\ writeAt' = [writeAt EXCEPT ![r] = Len(log) + 1]
    /\ writeTerm' = [writeTerm EXCEPT ![r] = term]
    /\ phase' = [phase EXCEPT ![r] = "write"]
    /\ UNCHANGED <<term, leader, committed, applied, host, planningTerm,
                   plannedId, barrierAt, succeeded, unknownPending>>

Finish(r) ==
    /\ phase[r] = "write"
    /\ Exact(writeAt[r], "write", r, writeTerm[r])
    /\ applied[host[r]] >= writeAt[r]
    /\ phase' = [phase EXCEPT ![r] = "done"]
    /\ succeeded' = succeeded \cup {r}
    /\ UNCHANGED <<term, leader, log, committed, applied, host,
                   planningTerm, plannedId, barrierAt, writeAt, writeTerm, unknownPending>>

\* Releasing the local mutex does NOT retract an appended command. A timed-out
\* write can still commit, apply, and affect a later planner's snapshot.
Timeout(r) ==
    /\ phase[r] \in Active
    /\ unknownPending' = IF phase[r] = "write" /\ writeAt[r] > committed
                           THEN unknownPending \cup {r} ELSE unknownPending
    /\ phase' = [phase EXCEPT ![r] = "done"]
    /\ UNCHANGED <<term, leader, log, committed, applied, host,
                   planningTerm, plannedId, barrierAt, writeAt, writeTerm, succeeded>>

Elect(n, keep) ==
    /\ term < MaxTerm
    /\ keep \in committed..Len(log)
    /\ term' = term + 1
    /\ leader' = n
    /\ log' = Append(SubSeq(log, 1, keep), Entry("election", 0, term + 1, 0))
    /\ UNCHANGED <<committed, applied, phase, host, planningTerm,
                   plannedId, barrierAt, writeAt, writeTerm, succeeded, unknownPending>>

Commit ==
    \E cut \in (committed + 1)..Len(log) :
        /\ log[cut].epoch = term
        /\ committed' = cut
        /\ UNCHANGED <<term, leader, log, applied, phase, host, planningTerm,
                       plannedId, barrierAt, writeAt, writeTerm, succeeded, unknownPending>>

Apply(n) ==
    /\ applied[n] < committed
    /\ applied' = [applied EXCEPT ![n] = @ + 1]
    /\ UNCHANGED <<term, leader, log, committed, phase, host, planningTerm,
                   plannedId, barrierAt, writeAt, writeTerm, succeeded, unknownPending>>

ElectAny == \E n \in Nodes, keep \in committed..Len(log) : Elect(n, keep)

Next ==
    \/ \E r \in Requests : Begin(r) \/ Barrier(r) \/ Plan(r) \/ Submit(r)
                           \/ Finish(r) \/ Timeout(r)
    \/ ElectAny
    \/ Commit
    \/ \E n \in Nodes : Apply(n)

Spec == Init /\ [][Next]_vars
\* Finite requests and election budget, fair replication and local apply.
\* This proves neither successful client admission nor availability under
\* arbitrary infinite faults. There are no TLC state/action constraints.
FairSpec == Spec /\ WF_vars(Commit) /\ (\A n \in Nodes : WF_vars(Apply(n)))
Drained == committed = Len(log) /\ (\A n \in Nodes : applied[n] = committed)
EventuallyDrained == <>[]Drained

TypeOK ==
    /\ term \in 1..MaxTerm
    /\ leader \in Nodes
    /\ committed \in 0..Len(log)
    /\ applied \in [Nodes -> 0..committed]
    /\ phase \in [Requests -> Active \cup {"new", "done"}]
    /\ host \in [Requests -> Nodes \cup {0}]
    /\ planningTerm \in [Requests -> 0..MaxTerm]
    /\ writeTerm \in [Requests -> 0..MaxTerm]
    /\ plannedId \in [Requests -> 0..Cardinality(Requests)]
    /\ barrierAt \in [Requests -> 0..(2 * Cardinality(Requests) + MaxTerm)]
    /\ writeAt \in [Requests -> 0..(2 * Cardinality(Requests) + MaxTerm)]
    /\ succeeded \subseteq Requests
    /\ unknownPending \subseteq Requests
    /\ \A i \in 1..Len(log) :
           log[i] \in [kind : {"barrier", "write", "election"},
                       request : Requests \cup {0}, epoch : 1..term,
                       id : 0..Cardinality(Requests)]

UniqueThrough(cut) ==
    \A i, j \in WritesThrough(cut) : i # j =>
        /\ log[i].id # log[j].id
        /\ RequestName[log[i].request] # RequestName[log[j].request]
CatalogSafety == UniqueThrough(committed)
LogSafety == UniqueThrough(Len(log))
FreshPlans ==
    \A r \in Requests :
        (phase[r] = "ready" /\ planningTerm[r] = term) =>
            /\ plannedId[r] = NextId(Len(log))
            /\ RequestName[r] \notin NamesThrough(Len(log))
ReceiptSafety ==
    \A r \in succeeded :
        /\ writeAt[r] <= committed
        /\ Exact(writeAt[r], "write", r, writeTerm[r])
        /\ log[writeAt[r]].id = plannedId[r]

=============================================================================
