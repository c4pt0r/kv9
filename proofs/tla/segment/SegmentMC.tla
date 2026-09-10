------------------------------ MODULE SegmentMC ------------------------------
EXTENDS SegmentDurability
SGMCIndex == [f \in SGFrames |-> CASE f = 1 -> 3 [] f = 2 -> 9 [] OTHER -> 10]
SGMCTerm == [f \in SGFrames |-> CASE f = 1 -> 1 [] f = 2 -> 2 [] OTHER -> 0]
\* Each live-check initial state is a concrete reachable operation/recovery cut.
SGMCEmpty == [phase |-> "ready", written |-> 0, durable |-> 0, summary |-> 0,
    ack |-> 0, data |-> [k \in SGSlots |-> 0], position |-> [k \in SGSlots |-> 0],
    pending |-> 0, pin |-> FALSE, tail |-> FALSE, durableTail |-> FALSE,
    identity |-> SGExpected, bad |-> FALSE, checked |-> TRUE, synced |-> TRUE,
    closed |-> FALSE, closedEnd |-> 0]
SGMCOne == [SGMCEmpty EXCEPT !.written = 1, !.durable = 1, !.summary = 1, !.ack = 1,
                            !.data[1] = 1, !.position[1] = 1]
SGMCCuts == {
    [SGMCEmpty EXCEPT !.phase = "write", !.pending = 1],
    [SGMCOne EXCEPT !.phase = "sync", !.pending = 1, !.durable = 0, !.summary = 0, !.ack = 0, !.synced = FALSE],
    [SGMCOne EXCEPT !.phase = "publish", !.pending = 1, !.summary = 0, !.ack = 0],
    [SGMCOne EXCEPT !.phase = "seal"],
    [SGMCOne EXCEPT !.phase = "scan", !.summary = 0, !.tail = TRUE, !.durableTail = TRUE,
                       !.checked = FALSE, !.synced = FALSE],
    [SGMCOne EXCEPT !.phase = "scan", !.summary = 0, !.ack = 0, !.checked = FALSE, !.synced = FALSE],
    [SGMCOne EXCEPT !.phase = "scan", !.summary = 0, !.identity = 2, !.checked = FALSE, !.synced = FALSE],
    [SGMCOne EXCEPT !.phase = "scan", !.summary = 0, !.bad = TRUE, !.checked = FALSE, !.synced = FALSE],
    [SGMCOne EXCEPT !.phase = "scan", !.summary = 0, !.closed = TRUE, !.closedEnd = 1,
                       !.checked = FALSE, !.synced = FALSE]}
SGMCFairInit == \E cut \in SGMCCuts :
    /\ sgPhase = cut.phase
    /\ sgWritten = cut.written
    /\ sgDurable = cut.durable
    /\ sgSummary = cut.summary
    /\ sgAck = cut.ack
    /\ sgData = cut.data
    /\ sgPosition = cut.position
    /\ sgPending = cut.pending
    /\ sgPin = cut.pin
    /\ sgTail = cut.tail
    /\ sgDurableTail = cut.durableTail
    /\ sgIdentity = cut.identity
    /\ sgBad = cut.bad
    /\ sgChecked = cut.checked
    /\ sgSynced = cut.synced
    /\ sgClosed = cut.closed
    /\ sgClosedEnd = cut.closedEnd
SGMCLiveSpec == SGMCFairInit /\ SGFairSpec
=============================================================================
