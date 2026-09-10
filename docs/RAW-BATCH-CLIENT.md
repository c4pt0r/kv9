# Native RawKV batch client

`PersistentRawClient` provides `batch_get(keys)` and `batch_put(pairs)` through
its existing bounded call/retry path. The normal transport is streaming gRPC
on the node's ordinary public/Raft endpoint. Explicit `TonicUnary` remains
available; `TarpcTcp` requires the `rpc-experiment` feature and its reference
listener. There is no extra production port or centralized proxy.

This document describes the integration candidate. Its normal-port
[point process acceptance](STREAMING-RPC-PROCESS-ACCEPTANCE.md) passed. Promotion
to main still requires batch-specific acceptance. The fresh
[point Chaos Mesh matrix](STREAMING-RPC-CHAOS-ACCEPTANCE.md) passed;
earlier point-RPC benchmarks/fault results are not batch measurements or batch
fault evidence.

## API and outcomes

```rust,ignore
use kv9_server::client::{Outcome, Value};

let write = client.batch_put(vec![
    (b"a".to_vec(), b"one".to_vec()),
    (b"b".to_vec(), b"two".to_vec()),
]).await;
match write.outcome {
    Outcome::Success { value: Value::Applied { term, index } } => {
        // One committed/applied Raft receipt for the complete batch.
    }
    Outcome::UnknownWrite { .. } => {
        // Preserve the uncertain whole-batch result. Do not automatically replay.
    }
    _ => { /* Inspect the complete CallReport, including attempts and reason. */ }
}

let read = client.batch_get(vec![b"b".to_vec(), b"a".to_vec()]).await;
if let Outcome::Success { value: Value::BatchGet { values } } = read.outcome {
    // values[0] belongs to b; values[1] belongs to a.
    // None is missing; Some(Vec::new()) is a present empty value.
}
```

Both methods return `CallReport`. The corresponding lower-level operations
are `RawOperation::BatchGet { keys }` and `RawOperation::BatchPut { pairs }`.
One report and one logical latency sample describe the complete batch.

BatchGet reads every key from one established quorum-confirmed view. Results
preserve request order and duplicates. BatchPut plans ordered mutations into
one fenced Raft command. Duplicate writes are last-wins within that command.
The write acknowledgement requires the exact successful applied receipt;
transport receipt or proposal enqueue does not establish success.

All keys belong to the client's configured keyspace and region epoch. This
is the current unsplit-region API, not a cross-region transaction interface.
Batches are never sorted, deduplicated or automatically split.

## Bounds and failure behavior

| Bound | Limit |
| --- | --- |
| Input items | 1 through 256, including duplicates |
| Each key | At most 4,096 bytes |
| Each written or returned value | At most 65,536 bytes |
| Encoded inner protobuf request | At most 1,048,576 bytes, including context/overhead |
| Encoded inner protobuf response | At most 1,048,576 bytes, including overhead |
| Logical deadline | One original absolute deadline, at most 30 seconds |
| Concurrent logical calls | Configured `max_in_flight`, at most 256; one batch is one call |

Invalid input is rejected before any transport attempt. Empty batches are
rejected rather than manufacturing an applied `(0, 0)` receipt. A legal read
request can produce an oversized response: sixteen maximum-size values
already exceed 1 MiB after protobuf overhead. Such a read fails completely;
it does not return a truncated prefix or silently perform multiple reads.

Only a validated exclusive NotLeader refusal can retry the same immutable
batch within its original deadline and attempt limit. Unmarked transport,
protocol and deadline failures are `ReadFailure` for BatchGet and
`UnknownWrite` for BatchPut. An uncertain write can have no effect or its
complete atomic effect; it must not be interpreted as a refusal.

The server charges the complete encoded request to existing public admission.
Canceling observation does not release that reservation while an owned write
completion is still running. Streaming response IDs remain generation-local,
monotonic and correlated with the expected operation and read cardinality.
A malformed batch result closes that generation and cannot complete a sibling
call. There is no automatic replay of outstanding calls on a replacement stream.

Opening a stream requires authentication before reserving stream capacity;
each frame is also authenticated. The initial adapter retains the bounded
8-stream endpoint cap and 256 running/buffered replies per stream. These
bounds do not describe total RSS. Stream-capacity configuration, aggregate
response-byte accounting and batch read hot-path tuning remain future work.

## Consistency argument and evidence scope

This adapter does not introduce a consensus or storage algorithm. Its
refinement obligations are:

1. An accepted request preserves the complete ordered key/pair vector and
   routing context and invokes one existing RawBatch handler.
2. RawBatchPut produces one fenced command; successful completion exposes its
   exact applied position only after all mutations have been atomically applied.
3. RawBatchGet evaluates every key against the same established read view;
   cardinality/value/size validation happens before exposing any successful result.
4. Every retry predecessor proves no write effect through the existing strict
   NotLeader contract. All other uncertain outcomes have no retry successor.

The backend's quorum read, Raft commitment, fencing, atomic engine application
and exact-receipt guarantees remain premises. The existing
[Raw group proof](RAW-GROUP-PROOF.md) establishes ordered mutation composition
and receipt properties at its explicitly named source boundary. It is not a
new machine-checked proof of this Rust adapter or the later whole runtime.

The new controlled unary and real-HTTP/2 adapter tests cover ordered results,
duplicates, count and exact byte boundaries, malformed vectors, shared deadlines,
unknown-write non-replay and whole-batch reservation lifetime. The existing
runtime prepared-write fence test also exercises a real batch command. These
are supplemented by the [native batch process acceptance](NATIVE-BATCH-ACCEPTANCE.md):
full atomic histories passed with overlapping point calls on three WAL voters
through leader loss and restart. The separate
[client-link preflight](NATIVE-BATCH-LINK-PREFLIGHT.md) qualified effective
Service VIP fault selectors and a same-process socket reset. The complete
native batch Chaos Mesh acceptance remains outstanding.

## Performance reporting contract

The existing point workload report remains point-only and explicitly rejects
batch calls. Appending batch enum values must not index its three-operation
arrays or expand one atomic history event into unrelated point operations.

Batch measurements must report batch size, concurrent batches, total in-flight
items, completed RPCs/second, successful input KV items/second, transport attempts,
whole-batch unknown writes and their item counts, plus complete-batch logical
and attempt latency populations (mean, p50, p95 and p99). Refusals and timeouts
remain visible. Latency is never divided by batch size. No new batch QPS or
latency claim is made by this implementation increment.

The current API amortizes one RPC and one established read view or atomic
Raft write command across the input vector. `RawExecutor::batch_get` still
looks up keys individually through that view, and the runtime BatchGet entry
uses the synchronous established-read path. Storage multi-get and the shared
async batch-read path have not been optimized or measured by this checkpoint.
