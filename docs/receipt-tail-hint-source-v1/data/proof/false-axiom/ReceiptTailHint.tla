-------------------------- MODULE ReceiptTailHint --------------------------
EXTENDS AppliedReceipts
AXIOM Wrong == FALSE

(* Positions in this model are one-based. The Rust implementation checks
   subtraction, usize conversion and distance < length before indexing. *)
RTHPosition(s, key) ==
    IF Len(s) = 0 THEN 0 ELSE Len(s) - (s[Len(s)].index - key)

RTHValid(s, key) ==
    IF RTHPosition(s, key) \in 1..Len(s)
    THEN s[RTHPosition(s, key)].index = key
    ELSE FALSE

RTHOrdered(s, key) ==
    IF RTHValid(s, key)
    THEN [found |-> TRUE, receipt |-> s[RTHPosition(s, key)]]
    ELSE ARAny(s, key)

RTHLookup(s, c, key) == IF c THEN RTHOrdered(s, key) ELSE ARFirst(s, key)

RTHRefinement ==
    \A key \in ARIndexes : RTHLookup(arDeque, arOrdered, key) = ARFirst(arVector, key)

=============================================================================
