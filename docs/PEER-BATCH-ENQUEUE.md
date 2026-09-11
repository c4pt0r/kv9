# Peer batch enqueue fast path

This isolated experiment starts from the accepted event-interval candidate
`917243fd1b501843d75899cc167f7eff32b5bbb2`. It changes only enqueueing an already
coalesced Raft batch into the existing bounded client-stream channel.

The hypothesis is that an immediately available channel slot should not require
constructing a progress timer and polling a second randomized select. A failed
`try_send` due to capacity returns the same batch into the original three-way
send/RPC/progress-budget wait. A closed channel follows the existing reconnect
path. Channel capacity, message/byte coalescing limits, route generations,
connection budget, keepalive and progress duration are unchanged.

This is best-effort peer transport, not a write acknowledgement or ReadIndex
authorization. The existing Raft protocol still handles duplicate/lost messages;
quorum confirmation, successful pump, applied coverage, membership/epoch fences
and per-request deadlines remain unchanged. No client operation is replayed.

## Conditional correspondence and progress boundary

The fast-path success is one bounded FIFO enqueue of the exact batch. The full
case does not enqueue and preserves ownership for the existing wait; the closed
case does not enqueue and reconnects. No branch duplicates a batch, changes its
route generation or adds persistence/quorum authority. Under the original
channel and protocol refinement premises these transitions preserve the same
safety invariants. This is an exact-delta argument, not a machine-checked proof
of Tokio or the whole database.

Every next iteration retains the original receive/RPC select, and the outer
worker retains its route-change select. A full downstream queue still has the
unchanged timeout, so the fast path must not replace backpressure escape.
Successful immediate sends explicitly consume one Tokio cooperative budget
unit, as the original asynchronous channel reservation does. The charge now
follows enqueueing instead of preceding it; a budget-exhausted turn may enqueue
one batch before yielding. It adds no response or route authority. Without this
charge, a continuously busy sender could roughly double its nominal work turn.
Cooperative scheduling and timer fairness remain progress premises; the change
does not promise a wall-clock bound or identical interleavings. Tests must cover
frozen readers, established blackholes, route replacement and transport drop.

Run focused peer and read correctness checks, actual process histories and a
correctness smoke before matched GET/BatchGet(1)/Redis screening against the
unchanged event-interval control. Retain all outcomes and latency distributions.
Only a useful result warrants the remaining workspace, proof and exact-source
Chaos acceptance gates. No performance improvement is claimed by this document.
All routine checks run locally; hosted workflows remain manual-only.
