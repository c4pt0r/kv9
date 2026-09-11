# Bounded parallel stream request experiment

The ordinary point stream currently polls every handler in one
`FuturesUnordered` owned by its receive task. Async waits overlap, but synchronous
work inside those polls (authorization, metadata lookup, result copying and
protobuf encoding) runs serially within that task. The experiment replaces this
set with a `JoinSet`, allowing the runtime to schedule individual handlers on
different workers. The unchanged request client and Raft backend make this a
server-side scheduling comparison. Extra scheduling, allocation and contention
can outweigh the parallelism; throughput and whole-call latency must decide
whether to retain it.

## Ownership and bounds

Each handler receives the same owned response-channel reservation that the
receive task obtains before reading its frame. Let `P` count a pending receive
reservation, `A` active handlers holding reservations, and `B` queued replies.
Acquiring, transferring, sending and dropping that reservation preserve
`P + A + B <= CHANNEL_LIMIT`. A completed task may remain in the join set until
collected, but the set's length is independently limited to `CHANNEL_LIMIT`.
No batch waits for future arrivals and no new unbounded queue is introduced.

The stream's semaphore grant is shared by the response stream, its owner and
every handler task. Dropping a `JoinSet` requests cancellation; Tokio performs
that cancellation cooperatively. The last grant reference therefore cannot
disappear while a handler remains live. A response-stream drop cannot recycle
that stream slot ahead of handler cleanup. Fully buffered replies retain the
response stream's grant even after input closes and its owner exits.

The owner treats a handler panic/join failure as generation termination and
aborts the remaining handlers. It does not detach their observation futures.
Existing write-completion tasks still own their original public admission
reservations through exact apply settlement, even after observation is canceled.
Reads retain their public reservation until the preparation or blocking job
actually drops/completes. Response reservations and stream grants do not replace
that public admission accounting.

## Consistency correspondence

Per-frame authentication, monotonic request IDs, operation and deadline bounds,
the dispatch implementation, encoded-size checks, reply IDs and client
generation/retry rules are unchanged. A frame keeps the deadline computed when
it was read; scheduling does not renew it. The existing stream already permits
overlapping requests and out-of-order replies. Request IDs validate framing and
correlation; they do not promise FIFO execution of concurrent operations.

Each task still calls the same public handler. Reads require the same Raft
quorum barrier followed by one authorized immutable view. Writes require the
same committed/applied receipt. This experiment changes which worker polls a
request, not any consensus, metadata, storage or receipt transition. Real-time
ordering of nonoverlapping calls still follows the existing response/receipt
and read-barrier contract. This is a source-level scheduling/ownership argument,
not a new whole-runtime machine-checked refinement claim; the broader adapter
proof obligations remain open.

## Required evidence

Keep the existing real HTTP/2 framing, authentication, generation/correlation,
deadline, unknown-write, held-admission and bounded-reply tests. Additional
controls must exercise response-drop cancellation and worker panic, including
cleanup and preservation of independently settling writes. Run local workspace
and Clippy gates, retained process histories, actual fault acceptance and matched
clean-release measurements before accepting a performance improvement. Candidate
results and any rejected experiment remain separately identified; no default
promotion or Redis-parity claim follows from this design.
