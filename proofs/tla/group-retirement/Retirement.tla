----------------------------- MODULE Retirement -----------------------------
EXTENDS Naturals
CONSTANT MaxReaders
VARIABLES registered, running, stopped, fenced, opened, locked, payload, external
vars == <<registered, running, stopped, fenced, opened, locked, payload, external>>

Init == /\ registered /\ ~running /\ ~stopped /\ ~fenced
        /\ opened /\ locked /\ payload /\ external = 0
Dispatch == /\ registered /\ ~stopped /\ running' = TRUE
            /\ UNCHANGED <<registered, stopped, fenced, opened, locked, payload, external>>
Finish == /\ running' = FALSE
          /\ UNCHANGED <<registered, stopped, fenced, opened, locked, payload, external>>
Borrow == /\ opened /\ external < MaxReaders /\ external' = external + 1
          /\ UNCHANGED <<registered, running, stopped, fenced, opened, locked, payload>>
Release == /\ external > 0 /\ external' = external - 1
           /\ UNCHANGED <<registered, running, stopped, fenced, opened, locked, payload>>
Stop == /\ stopped' = TRUE
        /\ UNCHANGED <<registered, running, fenced, opened, locked, payload, external>>
Publish == /\ stopped /\ fenced' = TRUE
           /\ UNCHANGED <<registered, running, stopped, opened, locked, payload, external>>
Unregister == /\ stopped /\ ~running /\ registered' = FALSE
              /\ UNCHANGED <<running, stopped, fenced, opened, locked, payload, external>>
Close == /\ ~registered /\ ~running /\ external = 0 /\ fenced
         /\ opened' = FALSE
         /\ UNCHANGED <<registered, running, stopped, fenced, locked, payload, external>>
Unlock == /\ ~opened /\ locked' = FALSE
          /\ UNCHANGED <<registered, running, stopped, fenced, opened, payload, external>>
Delete == /\ fenced /\ ~opened /\ payload' = FALSE
          /\ UNCHANGED <<registered, running, stopped, fenced, opened, locked, external>>
Crash == /\ registered' = FALSE /\ running' = FALSE /\ stopped' = TRUE
         /\ opened' = FALSE /\ locked' = FALSE /\ external' = 0
         /\ UNCHANGED <<fenced, payload>>
Recover == /\ ~fenced /\ payload /\ ~opened
           /\ registered' = TRUE /\ running' = FALSE /\ stopped' = FALSE
           /\ opened' = TRUE /\ locked' = TRUE /\ UNCHANGED <<fenced, payload, external>>
Next == Dispatch \/ Finish \/ Borrow \/ Release \/ Stop \/ Publish \/ Unregister
        \/ Close \/ Unlock \/ Delete \/ Crash \/ Recover
Spec == Init /\ [][Next]_vars
Safe == /\ (running => registered)
        /\ (registered => opened)
        /\ (external > 0 => opened)
        /\ (opened => locked /\ payload)
        /\ (~payload => fenced)
        /\ (fenced => stopped)
FenceMonotone == [][fenced => fenced']_vars
\* Retirement eventually drains if no new handles/crashes are introduced,
\* existing handles are released and maintenance/worker steps are fair.
DrainNext == Finish \/ Release \/ Stop \/ Publish \/ Unregister \/ Close \/ Unlock \/ Delete
DrainInit == /\ registered /\ running /\ ~stopped /\ ~fenced
             /\ opened /\ locked /\ payload /\ external = MaxReaders
DrainSpec == DrainInit /\ [][DrainNext]_vars /\ WF_vars(Finish) /\ WF_vars(Release)
             /\ WF_vars(Stop) /\ WF_vars(Publish) /\ WF_vars(Unregister)
             /\ WF_vars(Close) /\ WF_vars(Unlock) /\ WF_vars(Delete)
EventuallyDeleted == <> (~payload /\ ~locked)
=============================================================================
