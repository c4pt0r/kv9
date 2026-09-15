------------------------- MODULE ReceiptUpperBound -------------------------
EXTENDS Integers, Sequences

CONSTANTS UBCapacity, UBIndexes, UBTerms, UBOutcomes, UBQueries

UBLegalInputs ==
    /\ UBCapacity \in Nat \ {0}
    /\ UBIndexes \subseteq Nat
    /\ UBIndexes # {}
    /\ UBQueries \subseteq Nat
    /\ UBQueries # {}
    /\ UBTerms # {}
    /\ UBOutcomes # {}

ASSUME UBInputAssumption == UBLegalInputs

UBReceipts == [index : UBIndexes, term : UBTerms, outcome : UBOutcomes]
UBPushSequence(s, e) ==
    LET appended == Append(s, e)
    IN IF Len(appended) > UBCapacity THEN Tail(appended) ELSE appended

UBAdvance(b, e) == IF e.index > b THEN e.index ELSE b
UBBounded(s, b) == \A i \in 1..Len(s) : s[i].index <= b
UBMatches(s, key) == {i \in 1..Len(s) : s[i].index = key}
UBFirst(s, key) ==
    LET matches == UBMatches(s, key)
    IN IF matches = {} THEN [found |-> FALSE]
       ELSE LET first == CHOOSE i \in matches : \A j \in matches : i <= j
            IN [found |-> TRUE, receipt |-> s[first]]
UBLookup(s, b, key) == IF key > b THEN [found |-> FALSE] ELSE UBFirst(s, key)

VARIABLES ubOriginal, ubEntries, ubMaximum
ubVars == <<ubOriginal, ubEntries, ubMaximum>>
UBType ==
    /\ ubOriginal \in Seq(UBReceipts)
    /\ ubEntries \in Seq(UBReceipts)
    /\ Len(ubOriginal) <= UBCapacity
    /\ Len(ubEntries) <= UBCapacity
    /\ ubMaximum \in Nat
UBInvariant == UBType /\ ubOriginal = ubEntries /\ UBBounded(ubEntries, ubMaximum)
UBInit == ubOriginal = <<>> /\ ubEntries = <<>> /\ ubMaximum = 0
UBPush(e) ==
    /\ ubOriginal' = UBPushSequence(ubOriginal, e)
    /\ ubEntries' = UBPushSequence(ubEntries, e)
    /\ ubMaximum' = UBAdvance(ubMaximum, e)
UBNext == \E e \in UBReceipts : UBPush(e)
UBSpec == UBInit /\ [][UBNext]_ubVars
UBLookupRefinement ==
    \A key \in UBQueries : UBLookup(ubEntries, ubMaximum, key) = UBFirst(ubOriginal, key)

=============================================================================
