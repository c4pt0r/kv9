---------------------------- MODULE Transfer ----------------------------
EXTENDS Naturals
CONSTANT SourceClock, ExpectedRows, ExpectedDigest, Seed, MaxClock
VARIABLES copied, chain, clock, complete, bound
vars == <<copied, chain, clock, complete, bound>>
Init == /\ copied = 0 /\ chain = Seed /\ clock = SourceClock /\ ~complete /\ ~bound
Page(rows, hash) == /\ ~complete /\ copied + rows <= ExpectedRows
                    /\ copied' = copied + rows /\ chain' = hash
                    /\ UNCHANGED <<clock, complete, bound>>
Finish == /\ copied = ExpectedRows /\ chain = ExpectedDigest
          /\ complete' = TRUE /\ bound' = TRUE /\ UNCHANGED <<copied, chain, clock>>
Observe(now) == /\ complete /\ clock' = IF now > clock THEN now ELSE clock
                /\ UNCHANGED <<copied, chain, complete, bound>>
Next == Finish \/ (\E rows \in 1..2, hash \in 0..2 : Page(rows, hash))
               \/ (\E now \in 0..MaxClock : Observe(now))
Spec == Init /\ [][Next]_vars
Invariant == /\ copied <= ExpectedRows /\ clock >= SourceClock
             /\ (~complete => clock = SourceClock)
             /\ (complete => bound /\ copied = ExpectedRows /\ chain = ExpectedDigest)
             /\ (bound => complete)
ClockMonotone == [][clock' >= clock]_vars
CompletionPermanent == [][complete => complete']_vars
CompletedCursorImmutable == [][complete => copied' = copied /\ chain' = chain]_vars
\* One authenticated source/target specification. Hash collision resistance,
\* row codec/CF atomicity, source authentication and format rollout are external.
=======================================================================
