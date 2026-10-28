------------------------ MODULE CheckpointDrain ------------------------
EXTENDS Naturals
CONSTANT MaxJobs, MaxPuts
VARIABLES jobs, active, issued, resolved, unknown, closed, report
vars == <<jobs, active, issued, resolved, unknown, closed, report>>
Init == /\ jobs = 0 /\ active = 0 /\ issued = 0 /\ resolved = 0
        /\ unknown = 0 /\ closed = FALSE /\ report = <<>>
Admit == /\ ~closed /\ jobs < MaxJobs /\ jobs' = jobs + 1
         /\ UNCHANGED <<active, issued, resolved, unknown, closed, report>>
Issue == /\ active < jobs /\ issued < MaxPuts
         /\ issued' = issued + 1 /\ active' = active + 1
         /\ UNCHANGED <<jobs, resolved, unknown, closed, report>>
Reply == /\ active > 0 /\ active' = active - 1 /\ resolved' = resolved + 1
         /\ UNCHANGED <<jobs, issued, unknown, closed, report>>
Abandon == /\ active > 0 /\ active' = active - 1 /\ unknown' = unknown + 1
           /\ UNCHANGED <<jobs, issued, resolved, closed, report>>
EndJob == /\ active < jobs /\ jobs' = jobs - 1
          /\ UNCHANGED <<active, issued, resolved, unknown, closed, report>>
Close == /\ closed' = TRUE /\ UNCHANGED <<jobs, active, issued, resolved, unknown, report>>
Report == /\ closed /\ jobs = 0 /\ report' = <<issued, resolved>>
          /\ UNCHANGED <<jobs, active, issued, resolved, unknown, closed>>
Next == Admit \/ Issue \/ Reply \/ Abandon \/ EndJob \/ Close \/ Report
Spec == Init /\ [][Next]_vars
Invariant == /\ active <= jobs /\ issued = resolved + unknown + active
             /\ (report # <<>> => closed /\ jobs = 0 /\ active = 0 /\ report = <<issued, resolved>>)
             /\ (report # <<>> /\ report[1] = report[2] => unknown = 0)
\* Allowed replies require single-attempt backend semantics. Unknown is sticky.
\* Global release, backend qualification and other generations are outside scope.
=======================================================================
