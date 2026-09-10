------------------------- MODULE SegmentDurability -------------------------
EXTENDS Naturals
CONSTANT SGMax, SGFrames, SGPositioned, SGIndex, SGTerm, SGPrevious,
         SGIdentities, SGExpected
ASSUME SGLegalInputs ==
    /\ SGMax \in Nat \ {0} /\ SGFrames # {} /\ SGFrames \subseteq Nat \ {0}
    /\ SGPositioned \subseteq SGFrames
    /\ SGIndex \in [SGFrames -> Nat] /\ SGTerm \in [SGFrames -> Nat]
    /\ SGPrevious \in SGPositioned \cup {0}
    /\ SGIdentities # {} /\ SGIdentities \subseteq Nat \ {0}
    /\ SGExpected \in SGIdentities

\* One exclusively owned file selected by external durable topology. A frame
\* token denotes the entire payload and its optional exact applied position.
\* Checked framing detects the modeled corruption; this is not a CRC proof.
\* Successful fsync preserves every earlier durable byte through a crash.
\* Counts describe COMPLETE frames, never bytes from the incomplete tail.
VARIABLES sgPhase, sgWritten, sgDurable, sgSummary, sgAck, sgData, sgPosition, sgPending, sgPin, sgTail, sgDurableTail, sgIdentity, sgBad, sgChecked, sgSynced, sgClosed, sgClosedEnd
sgVars == <<sgPhase, sgWritten, sgDurable, sgSummary, sgAck, sgData, sgPosition, sgPending, sgPin, sgTail, sgDurableTail, sgIdentity, sgBad, sgChecked, sgSynced, sgClosed, sgClosedEnd>>
SGPhases == {"ready", "write", "sync", "publish", "seal", "closed", "fenced",
             "down", "scan", "validated", "recover_sync", "recover_publish", "rejected"}
SGSlots == 1..SGMax
SGAdvance(a, b) == IF a \notin SGPositioned \/ b \notin SGPositioned THEN TRUE
                  ELSE SGIndex[b] > SGIndex[a] /\ SGTerm[b] >= SGTerm[a]
SGPinned(n, data) == \E k \in 1..n : data[k] \notin SGPositioned
SGCanAppend(f) == /\ f \in SGFrames /\ SGAdvance(SGPrevious, f)
                 /\ \A k \in 1..sgWritten : SGAdvance(sgPosition[k], f)

SGInit == sgPhase = "ready"
          /\ sgWritten = 0
          /\ sgDurable = 0
          /\ sgSummary = 0
          /\ sgAck = 0
          /\ sgData = [k \in SGSlots |-> 0]
          /\ sgPosition = [k \in SGSlots |-> 0]
          /\ sgPending = 0
          /\ sgPin = FALSE
          /\ sgTail = FALSE
          /\ sgDurableTail = FALSE
          /\ sgIdentity = SGExpected
          /\ sgBad = FALSE
          /\ sgChecked = TRUE
          /\ sgSynced = TRUE
          /\ sgClosed = FALSE
          /\ sgClosedEnd = 0

\* Validation is side-effect free. The full-prefix guard is equivalent to the
\* runtime's last-position guard on an ordered prefix (proved separately).
SGStart(f) == /\ sgPhase = "ready" /\ sgWritten < SGMax /\ SGCanAppend(f)
              /\ sgPhase' = "write"
             /\ sgPending' = f
             /\ UNCHANGED <<sgWritten, sgDurable, sgSummary, sgAck, sgData, sgPosition, sgPin, sgTail, sgDurableTail, sgIdentity, sgBad, sgChecked, sgSynced, sgClosed, sgClosedEnd>>
SGWrite == /\ sgPhase = "write"
           /\ sgData' = [sgData EXCEPT ![sgWritten + 1] = sgPending]
             /\ sgPosition' = [sgPosition EXCEPT ![sgWritten + 1] = sgPending]
             /\ sgWritten' = sgWritten + 1
             /\ sgPhase' = "sync"
             /\ sgSynced' = FALSE
             /\ UNCHANGED <<sgDurable, sgSummary, sgAck, sgPending, sgPin, sgTail, sgDurableTail, sgIdentity, sgBad, sgChecked, sgClosed, sgClosedEnd>>
SGSync == /\ sgPhase = "sync"
          /\ sgDurable' = sgWritten
             /\ sgSynced' = TRUE
             /\ sgPhase' = "publish"
             /\ UNCHANGED <<sgWritten, sgSummary, sgAck, sgData, sgPosition, sgPending, sgPin, sgTail, sgDurableTail, sgIdentity, sgBad, sgChecked, sgClosed, sgClosedEnd>>
SGPublish == /\ sgPhase = "publish"
             /\ sgSummary' = sgWritten
             /\ sgAck' = sgWritten
             /\ sgPin' = SGPinned(sgWritten, sgData)
             /\ sgPending' = 0
             /\ sgPhase' = "ready"
             /\ UNCHANGED <<sgWritten, sgDurable, sgData, sgPosition, sgTail, sgDurableTail, sgIdentity, sgBad, sgChecked, sgSynced, sgClosed, sgClosedEnd>>
SGSealStart == /\ sgPhase = "ready" /\ sgPhase' = "seal"
             /\ UNCHANGED <<sgWritten, sgDurable, sgSummary, sgAck, sgData, sgPosition, sgPending, sgPin, sgTail, sgDurableTail, sgIdentity, sgBad, sgChecked, sgSynced, sgClosed, sgClosedEnd>>
SGSealSync == /\ sgPhase = "seal"
              /\ sgClosed' = TRUE
             /\ sgClosedEnd' = sgWritten
             /\ sgPhase' = "closed"
             /\ sgSynced' = TRUE
             /\ UNCHANGED <<sgWritten, sgDurable, sgSummary, sgAck, sgData, sgPosition, sgPending, sgPin, sgTail, sgDurableTail, sgIdentity, sgBad, sgChecked>>
\* A failed write may have left an incomplete frame; failed fsync may have
\* persisted a complete frame. Neither result returns a reusable handle.
SGFail == /\ sgPhase \in {"write", "sync", "seal", "recover_sync"}
          /\ sgPhase' = "fenced"
             /\ sgSynced' = FALSE
             /\ sgTail' = (IF sgPhase = "write" THEN TRUE ELSE sgTail)
             /\ UNCHANGED <<sgWritten, sgDurable, sgSummary, sgAck, sgData, sgPosition, sgPending, sgPin, sgDurableTail, sgIdentity, sgBad, sgChecked, sgClosed, sgClosedEnd>>
\* Drop/crash is the only exit from a fenced handle. Any unsynchronized complete
\* suffix may survive or disappear. A partial final frame never contains an ack.
\* Failed repair sync may restore the previous durable incomplete tail.
SGCrash(d, t) == /\ sgPhase # "down" /\ d \in sgDurable..sgWritten /\ t \in BOOLEAN
                /\ (t => d < sgWritten \/ sgPhase = "write" \/ sgTail \/ sgDurableTail)
                /\ sgWritten' = d
             /\ sgDurable' = d
             /\ sgSummary' = 0
             /\ sgPin' = FALSE
             /\ sgPending' = 0
             /\ sgTail' = t
             /\ sgDurableTail' = t
             /\ sgPhase' = "down"
             /\ sgChecked' = FALSE
             /\ sgSynced' = FALSE
             /\ UNCHANGED <<sgAck, sgData, sgPosition, sgIdentity, sgBad, sgClosed, sgClosedEnd>>
\* Read observations include wrong selected-file identity and complete corrupt
\* records/closed-summary mismatch. They cannot grant a writer or publish data.
SGOpen(i, b) == /\ sgPhase = "down" /\ i \in SGIdentities /\ b \in BOOLEAN
               /\ sgIdentity' = i
             /\ sgBad' = b
             /\ sgPhase' = "scan"
             /\ UNCHANGED <<sgWritten, sgDurable, sgSummary, sgAck, sgData, sgPosition, sgPending, sgPin, sgTail, sgDurableTail, sgChecked, sgSynced, sgClosed, sgClosedEnd>>
SGValidate == /\ sgPhase = "scan" /\ sgIdentity = SGExpected /\ ~sgBad
              /\ sgChecked' = TRUE
             /\ sgPhase' = "validated"
             /\ UNCHANGED <<sgWritten, sgDurable, sgSummary, sgAck, sgData, sgPosition, sgPending, sgPin, sgTail, sgDurableTail, sgIdentity, sgBad, sgSynced, sgClosed, sgClosedEnd>>
SGReject == /\ sgPhase = "scan" /\ (sgIdentity # SGExpected \/ sgBad)
            /\ sgPhase' = "rejected"
             /\ UNCHANGED <<sgWritten, sgDurable, sgSummary, sgAck, sgData, sgPosition, sgPending, sgPin, sgTail, sgDurableTail, sgIdentity, sgBad, sgChecked, sgSynced, sgClosed, sgClosedEnd>>
SGRepair == /\ sgPhase = "validated" /\ ~sgClosed
            /\ sgTail' = FALSE
             /\ sgPhase' = "recover_sync"
             /\ UNCHANGED <<sgWritten, sgDurable, sgSummary, sgAck, sgData, sgPosition, sgPending, sgPin, sgDurableTail, sgIdentity, sgBad, sgChecked, sgSynced, sgClosed, sgClosedEnd>>
\* set_len alone is not recovery publication. File and namespace sync must
\* complete even when scanning found no torn tail.
SGRecoverySync == /\ sgPhase = "recover_sync"
                  /\ sgDurable' = sgWritten
             /\ sgDurableTail' = FALSE
             /\ sgSynced' = TRUE
             /\ sgPhase' = "recover_publish"
             /\ UNCHANGED <<sgWritten, sgSummary, sgAck, sgData, sgPosition, sgPending, sgPin, sgTail, sgIdentity, sgBad, sgChecked, sgClosed, sgClosedEnd>>
SGRecoveryPublish == /\ sgPhase = "recover_publish"
                     /\ sgSummary' = sgWritten
             /\ sgPin' = SGPinned(sgWritten, sgData)
             /\ sgPhase' = "ready"
             /\ UNCHANGED <<sgWritten, sgDurable, sgAck, sgData, sgPosition, sgPending, sgTail, sgDurableTail, sgIdentity, sgBad, sgChecked, sgSynced, sgClosed, sgClosedEnd>>
SGClosedReplay == /\ sgPhase = "validated" /\ sgClosed
                  /\ sgSummary' = sgWritten
             /\ sgPin' = SGPinned(sgWritten, sgData)
             /\ sgSynced' = TRUE
             /\ sgPhase' = "closed"
             /\ UNCHANGED <<sgWritten, sgDurable, sgAck, sgData, sgPosition, sgPending, sgTail, sgDurableTail, sgIdentity, sgBad, sgChecked, sgClosed, sgClosedEnd>>
SGQuiesce == UNCHANGED sgVars
SGNext == (\E f \in SGFrames : SGStart(f)) \/ SGWrite \/ SGSync \/ SGPublish
          \/ SGSealStart \/ SGSealSync \/ SGFail
          \/ (\E d \in 0..SGMax, t \in BOOLEAN : SGCrash(d, t))
          \/ (\E i \in SGIdentities, b \in BOOLEAN : SGOpen(i, b))
          \/ SGValidate \/ SGReject \/ SGRepair \/ SGRecoverySync
          \/ SGRecoveryPublish \/ SGClosedReplay \/ SGQuiesce
SGSpec == SGInit /\ [][SGNext]_sgVars

SGType == sgPhase \in SGPhases
          /\ sgWritten \in 0..SGMax
          /\ sgDurable \in 0..SGMax
          /\ sgSummary \in 0..SGMax
          /\ sgAck \in 0..SGMax
          /\ sgData \in [SGSlots -> SGFrames \cup {0}]
          /\ sgPosition \in [SGSlots -> SGFrames \cup {0}]
          /\ sgPending \in SGFrames \cup {0}
          /\ sgPin \in BOOLEAN
          /\ sgTail \in BOOLEAN
          /\ sgDurableTail \in BOOLEAN
          /\ sgIdentity \in SGIdentities
          /\ sgBad \in BOOLEAN
          /\ sgChecked \in BOOLEAN
          /\ sgSynced \in BOOLEAN
          /\ sgClosed \in BOOLEAN
          /\ sgClosedEnd \in 0..SGMax
SGPrefix == /\ sgAck <= sgDurable /\ sgSummary <= sgDurable /\ sgDurable <= sgWritten
            /\ \A k \in 1..sgWritten : sgData[k] \in SGFrames /\ sgData[k] = sgPosition[k]
SGOrder == /\ \A k \in 1..sgWritten : SGAdvance(SGPrevious, sgPosition[k])
           /\ \A i, j \in 1..sgWritten : i < j => SGAdvance(sgPosition[i], sgPosition[j])
SGSummary == sgPin = SGPinned(sgSummary, sgData)
SGAuthority ==
    /\ (sgPhase \in {"ready", "write", "sync", "publish", "seal", "closed", "validated", "recover_sync", "recover_publish"}
        => sgChecked /\ ~sgBad /\ sgIdentity = SGExpected)
    /\ (sgPhase \in {"ready", "closed"} => sgSynced /\ ~sgTail /\ ~sgDurableTail)
SGStage ==
    /\ (sgPhase \in {"write", "seal"} => ~sgTail /\ ~sgDurableTail /\ sgSynced)
    /\ (sgPhase = "recover_sync" => ~sgTail)
    /\ (sgPhase \in {"ready", "write", "seal", "closed"} => sgSummary = sgWritten /\ sgDurable = sgWritten)
    /\ (sgPhase = "write" => sgPending \in SGFrames /\ sgWritten < SGMax /\ SGCanAppend(sgPending))
    /\ (sgPhase \in {"sync", "publish"} => sgWritten = sgSummary + 1 /\ ~sgTail /\ ~sgDurableTail)
    /\ (sgPhase = "publish" => sgDurable = sgWritten /\ sgSynced)
    /\ (sgPhase \in {"down", "scan", "validated", "recover_sync", "recover_publish", "rejected"}
        => sgDurable = sgWritten)
    /\ (sgPhase = "recover_publish" => sgSynced /\ ~sgTail /\ ~sgDurableTail)
    /\ (sgClosed => sgWritten = sgClosedEnd /\ sgDurable = sgClosedEnd /\ ~sgTail /\ ~sgDurableTail
         /\ sgPhase \in {"closed", "down", "scan", "validated", "rejected"})
SGInvariant == SGType /\ SGPrefix /\ SGOrder /\ SGSummary /\ SGAuthority /\ SGStage
\* These lemmas expose the last-position implementation of the prefix guard.
SGLastOf(l, n, data) == /\ l \in 0..n
    /\ (l = 0 => \A k \in 1..n : data[k] \notin SGPositioned)
    /\ (l > 0 => data[l] \in SGPositioned /\ \A k \in 1..n : k > l => data[k] \notin SGPositioned)
SGFirstOf(f, n, data) == /\ f \in 0..n
    /\ (f = 0 => \A k \in 1..n : data[k] \notin SGPositioned)
    /\ (f > 0 => data[f] \in SGPositioned /\ \A k \in 1..n : k < f => data[k] \notin SGPositioned)
SGLatest(l) == SGLastOf(l, sgWritten, sgPosition)
SGFoldFirst(first, n, r) == IF first = 0 /\ r \in SGPositioned THEN n + 1 ELSE first
SGFoldLast(last, n, r) == IF r \in SGPositioned THEN n + 1 ELSE last
SGFoldPin(pin, r) == pin \/ r \notin SGPositioned
SGSummaryGood(n, first, last, pin, data) ==
    SGFirstOf(first, n, data) /\ SGLastOf(last, n, data) /\ pin = SGPinned(n, data)
SGFoldTranscript(data, sums) ==
    /\ data \in [SGSlots -> SGFrames]
    /\ sums \in [0..SGMax -> [first : 0..SGMax, last : 0..SGMax, pin : BOOLEAN]]
    /\ sums[0] = [first |-> 0, last |-> 0, pin |-> FALSE]
    /\ \A n \in 0..SGMax : n < SGMax => sums[n + 1] =
        [first |-> SGFoldFirst(sums[n].first, n, data[n + 1]),
         last |-> SGFoldLast(sums[n].last, n, data[n + 1]),
         pin |-> SGFoldPin(sums[n].pin, data[n + 1])]
\* The scalar-summary law is checked at actual complete prefixes, including
\* the next summary update after the most recent complete frame is synchronized.
SGFoldLaw == \A n \in 0..SGMax : \A first, last \in 0..n, pin \in BOOLEAN :
    n + 1 = sgWritten /\ SGSummaryGood(n, first, last, pin, sgData) =>
      SGSummaryGood(n + 1, SGFoldFirst(first, n, sgData[n + 1]),
          SGFoldLast(last, n, sgData[n + 1]), SGFoldPin(pin, sgData[n + 1]), sgData)
SGLastGuard(l, f) == f \in SGFrames /\ SGAdvance(IF l = 0 THEN SGPrevious ELSE sgPosition[l], f)
SGAckStep == /\ sgAck' >= sgAck
             /\ \A k \in 1..sgAck : sgData'[k] = sgData[k] /\ sgPosition'[k] = sgPosition[k]
SGAckStableAlways == [][SGAckStep]_sgVars
SGAcknowledged == sgAck <= sgDurable
SGBinding == \A k \in 1..sgWritten : sgData[k] = sgPosition[k]
SGClosedStep == sgClosed => UNCHANGED <<sgWritten, sgDurable, sgData, sgPosition, sgTail, sgDurableTail>>
SGClosedAlways == [][SGClosedStep]_sgVars
SGFencedStep == sgPhase = "fenced" => sgPhase' \in {"fenced", "down"} /\ sgAck' = sgAck
SGFencedAlways == [][SGFencedStep]_sgVars

\* A finite in-flight operation completes under fair, failure-free local I/O
\* and scheduling. New appends, new seals, crashes and errors are absent here;
\* neither unavailable storage nor endlessly supplied new work has a deadline.
SGStableNext == SGWrite \/ SGSync \/ SGPublish \/ SGSealSync \/ SGValidate \/ SGReject
                \/ SGRepair \/ SGRecoverySync \/ SGRecoveryPublish \/ SGClosedReplay \/ SGQuiesce
SGFairSpec == SGInvariant /\ [][SGStableNext]_sgVars
              /\ WF_sgVars(SGWrite) /\ WF_sgVars(SGSync) /\ WF_sgVars(SGPublish)
              /\ WF_sgVars(SGSealSync) /\ WF_sgVars(SGValidate) /\ WF_sgVars(SGReject)
              /\ WF_sgVars(SGRepair) /\ WF_sgVars(SGRecoverySync)
              /\ WF_sgVars(SGRecoveryPublish) /\ WF_sgVars(SGClosedReplay)
SGSettled == sgPhase \in {"ready", "closed", "rejected"}
SGInFlight == sgPhase \in {"write", "sync", "publish", "seal", "scan", "validated", "recover_sync", "recover_publish"}
SGProgress == SGInFlight ~> SGSettled
SGNoAcknowledgement == sgAck = 0
SGNoRecoveredUnknown == ~(sgPhase = "ready" /\ sgSummary > sgAck)
SGNoUnpositioned == ~sgPin
=============================================================================
