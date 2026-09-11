------------------------- MODULE AppliedReceipts -------------------------
EXTENDS Integers, Sequences

CONSTANTS ARCapacity, ARIndexes, ARTerms, AROutcomes

ARLegalInputs ==
    /\ ARCapacity \in Nat \ {0}
    /\ ARIndexes \subseteq Nat
    /\ ARIndexes # {}
    /\ ARTerms # {}
    /\ AROutcomes # {}

ASSUME ARInputAssumption == ARLegalInputs

ARReceipts == [index : ARIndexes, term : ARTerms, outcome : AROutcomes]

(* The two expressions follow the original append-then-trim vector and the
   candidate evict-then-append deque, respectively. These are logical sequences;
   Rust allocation and VecDeque's physical storage are outside this model. *)
ARVectorPush(s, e) ==
    LET appended == Append(s, e)
    IN IF Len(appended) > ARCapacity THEN Tail(appended) ELSE appended

ARDequePush(s, e) ==
    Append(IF Len(s) = ARCapacity THEN SubSeq(s, 1, Len(s) - 1) ELSE s, e)

ARStrict(s) ==
    \A i, j \in 1..Len(s) : i < j => s[i].index < s[j].index

ARCertificate(s, c, e) ==
    c /\ (IF Len(s) = 0 THEN TRUE ELSE s[Len(s)].index < e.index)

ARMatches(s, key) == {i \in 1..Len(s) : s[i].index = key}

ARFirst(s, key) ==
    LET matches == ARMatches(s, key)
    IN IF matches = {} THEN [found |-> FALSE]
       ELSE LET first == CHOOSE i \in matches : \A j \in matches : i <= j
            IN [found |-> TRUE, receipt |-> s[first]]

(* This is the sorted-library-search postcondition, not a model of the Rust
   standard library's binary-search implementation. Without uniqueness it may
   select any matching position, so the checked certificate is essential. *)
ARAny(s, key) ==
    LET matches == ARMatches(s, key)
    IN IF matches = {} THEN [found |-> FALSE]
       ELSE [found |-> TRUE, receipt |-> s[CHOOSE i \in matches : TRUE]]

ARLookup(s, c, key) == IF c THEN ARAny(s, key) ELSE ARFirst(s, key)

VARIABLES arVector, arDeque, arOrdered
arVars == <<arVector, arDeque, arOrdered>>

ARType ==
    /\ arVector \in Seq(ARReceipts)
    /\ arDeque \in Seq(ARReceipts)
    /\ Len(arVector) <= ARCapacity
    /\ Len(arDeque) <= ARCapacity
    /\ arOrdered \in BOOLEAN

ARRefinement == arDeque = arVector
AROrdering == arOrdered => ARStrict(arDeque)
ARInvariant == ARType /\ ARRefinement /\ AROrdering

ARInit == arVector = <<>> /\ arDeque = <<>> /\ arOrdered = TRUE

ARPush(e) ==
    /\ arVector' = ARVectorPush(arVector, e)
    /\ arDeque' = ARDequePush(arDeque, e)
    /\ arOrdered' = ARCertificate(arDeque, arOrdered, e)

ARNext == \E e \in ARReceipts : ARPush(e)
ARSpec == ARInit /\ [][ARNext]_arVars

ARLookupRefinement ==
    \A key \in ARIndexes : ARLookup(arDeque, arOrdered, key) = ARFirst(arVector, key)

=============================================================================
