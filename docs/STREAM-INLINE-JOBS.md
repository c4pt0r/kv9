# Poll bounded RPC handlers in their stream owner

The previous fixed global-queue polling experiment was slower in both c64 GET
run orders and is excluded from this candidate. This branch starts from
`40f014f`, with the selected CRC runtime configuration and the optional read-stage
observer disabled in default builds.

Each point-stream request previously created a Tokio task in a `JoinSet`. This
candidate stores the handler futures in `FuturesUnordered`, polled by the existing
stream owner. Read completions wake the stream owner and ready handler futures
are polled there. The hypothesis is that removing per-request task scheduling
and joining reduces RPC overhead. One stream also loses independent CPU
scheduling for its handlers; throughput and latency must be measured together.
No speedup is assumed from this source change.

## Ownership and bounds

The incoming frame still reserves a slot in the bounded outgoing channel before
dispatch. Each handler owns that reservation until its reply is queued or the
handler is dropped. Running handlers plus buffered replies cannot exceed
`CHANNEL_LIMIT`. A pending handler does not serialize another ready handler;
half-closing input drains all already admitted handlers before the stream ends.
Each handler retains the connection permit through its actual destruction.

The response stream still owns and aborts the one stream task on drop. Dropping
the future collection cancels read observation; prepared writes retain their
independently owned admission reservation until their original apply settles.
A handler panic unwinds the stream owner, closing the generation and dropping
the remaining observers. Existing panic and controlled-destructor tests check
that read cleanup and prepared-write ownership retain these distinctions.

Per-frame authentication, strictly increasing request IDs, frame limits,
response limits, deadlines, shutdown and backpressure remain in the same owner
loop. Each future retains the same `timeout_at` deadline established when its
frame is admitted. No extra queue, worker, service or unbounded result storage
is introduced.

## Correctness and measurement gates

No Raft, engine, metadata, WAL or core algorithm source changes. Fresh Safe
ReadIndex, sealed groups, exact context matching, successful whole-pump
completion, unified apply/view fences and durable write acknowledgements retain
their existing implementation and proof scope. This scheduling change is not
a new proof of Tokio, futures-util or the complete Rust implementation.

Root runs local Raft/server tests, formatting and all-target Clippy, including
pending-read concurrency/half-close, controlled cancellation cleanup, panic,
deadline, retained prepared-write admission and stalled-consumer bounds. A clean
source-bound retained build requires first-party cache invalidation. Ordinary
leader-loss/restart histories precede the complete same-client c1/c64 GET/mixed
opposite-order comparison against selected CRC and Redis. Default builds keep
the diagnostic observer off. A regression is retained and rejected; a favorable
screen remains experimental pending broader API and actual Chaos acceptance.
