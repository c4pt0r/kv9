# Raw write execution path and measurement decisions

Tracking: #9, #13 and #20. This map describes the unchanged general server
`5ee897a2f58c57bdf17ea1757c96224adf0f0dbb`; the read-credit candidate `57ff6851`
inherits this write path. New v3 clients at `0be806d9` expose actual point PUT/SET
and mixed workloads for comparing these servers. No server optimization is
introduced by this document.

## Required ordering and current execution

1. Public admission reserves the request count and encoded bytes. The same
   reservation follows the actual backend job through cancellation, submission
   and completion. Cancelling the RPC must not release capacity around a live
   proposal.
2. The public write wrapper starts an owned asynchronous task, dispatches the
   synchronous preparation closure to a blocking worker and then awaits its
   asynchronous completion. `Reservation::start` runs on the blocking worker.
   Consequently preparation-queue timing includes dispatch waiting.
3. `RuntimeBackend::prepare_raw_write` validates keyspace, region and epoch on
   a metadata view, plans mutations and consumes one non-cloneable
   `ValidatedFence` into a fenced command. Write planning does not run a fresh
   ReadIndex before every proposal. Ordered apply adjudicates the fence again
   so an intervening ownership change cannot authorize a stale write.
4. `NodeDriver::propose_with_async_wait` reserves bounded apply-wait capacity
   before proposing. Encoding and submission run synchronously. The peer lock
   jointly protects the leadership check, proposal and exact position capture;
   it is also held by Ready persistence. Removing a blocking boundary without
   addressing these possible waits can block the RPC executor.
5. The Raft owner persists Ready entries and HardState before publishing its
   messages, committed entries or read states. A local append position alone
   does not acknowledge a write. Normal quorum replication remains required.
6. Ordered apply checks the command fence and persists the engine batch and
   applied position before publishing visible state and exact receipts. The
   owner publishes completion; the waiting task validates the receipt and
   finishes the public reservation. Only an explicitly proven replacement is
   eligible for the existing internal reproposal policy; its absolute deadline
   is unchanged.

Relevant source: [public wrapper](https://github.com/c4pt0r/kv9/blob/5ee897a2f58c57bdf17ea1757c96224adf0f0dbb/crates/server/src/grpc.rs),
[runtime preparation and fence](https://github.com/c4pt0r/kv9/blob/5ee897a2f58c57bdf17ea1757c96224adf0f0dbb/crates/server/src/runtime.rs),
[driver and apply wait](https://github.com/c4pt0r/kv9/blob/5ee897a2f58c57bdf17ea1757c96224adf0f0dbb/crates/raft/src/driver.rs),
[Ready processing](https://github.com/c4pt0r/kv9/blob/5ee897a2f58c57bdf17ea1757c96224adf0f0dbb/crates/raft/src/rawnode.rs).

## Existing batching and memory behavior

The server already amortizes some required work. One Raft Ready sync covers
its appended entry slice and HardState. The driver groups an already queued,
bounded contiguous Raw prefix for engine apply; catalog, manifest, no-op and
configuration entries remain barriers. Commands retain their own outcomes and
positions. A future change must improve this actual grouping rather than
assume every command currently has an independent sync.

The in-memory index uses structurally shared persistent maps. Taking a snapshot
clones map roots in O(1); it does not copy the complete dataset. Updating keys
still performs map work and owns key/value data. Batch request validation also
checks every key's region membership. The amount of CPU spent in these paths
must be measured for larger batches rather than inferred from pure GET profiles.

Relevant source: [Raw apply grouping](https://github.com/c4pt0r/kv9/blob/5ee897a2f58c57bdf17ea1757c96224adf0f0dbb/crates/raft/src/state_machine/raw_group.rs),
[Raft storage](https://github.com/c4pt0r/kv9/blob/5ee897a2f58c57bdf17ea1757c96224adf0f0dbb/crates/raft/src/storage.rs),
[memory index](https://github.com/c4pt0r/kv9/blob/5ee897a2f58c57bdf17ea1757c96224adf0f0dbb/crates/engine/src/mem.rs).

## Interpreting the existing metrics

| Metric | What it observes | Interpretation limit |
| --- | --- | --- |
| `public_raw_write_prepare_queue` | Public reservation until blocking preparation starts | Includes asynchronous dispatch and blocking-pool waiting; not a kernel-only switch cost |
| `public_raw_write_backend` | Preparation start through logical completion | Includes validation, submission and completion; point and batch writes share this class |
| `raft_proposal_submission` | Command encoding and synchronous peer proposal | Can include lock waiting; local proposal completion is not quorum acknowledgment |
| `runtime_logical_proposal_wait` | Initial submission through final logical result | Excludes earlier public dispatch/planning; can include proven replacement attempts |
| `raft_application_wait` | Registered exact-apply wait through observed outcome | Can include peer work, persistence, apply and receiver scheduling |
| `raft_command_apply` | A command's apply interval, including shared group persistence | Group members can share an interval; summing samples overcounts elapsed wall time |
| `raft_wal_record_sync`, `engine_wal_record_sync` | Their respective synchronization calls | Independent populations, replica roles and grouping; not additive per-request stages |

Before/after exporter snapshots encompass initialization, warmup, measured
traffic, drain and final readback. Public Raw write metrics include setup writes;
Raw read metrics also include final scans. Validate process/exporter identity,
freshness, complete histogram arithmetic and population counts before deriving
deltas. Do not subtract independently populated means to manufacture a latency
partition. CPU samples measure on-CPU work and omit blocked time.

## Next decision

The [completed v3 comparison](V3-WORKLOAD-PERFORMANCE.md) now establishes the
broader point/batch read/write gap: candidate c64 PUT is 117,729 calls/s versus
Redis 493,396/s, and BatchPut(64) is 628,102 keys/s versus Redis 6,066,081/s.
The independent source-bound endpoint derivation passes all 48 native cohorts.
For candidate batch PUT c64, command apply has a 2,173-us mean group interval;
engine WAL sync is 0.136 us in its independent population on tmpfs. These are
complete-envelope metrics, not a measured-only additive client latency budget.

Prioritize point/batch write CPU attribution, especially command conversion,
fence checks, log handling and ordered-index mutation. The fixed CPU readout
and larger-batch gap justify this before another isolated GET wakeup tweak.
Existing group commit is already present; avoid assuming a larger group or
removing a task automatically improves throughput and tails. Implement the
measured avoidable work with the same mutation ordering, snapshots, atomic
applied position, deadlines and quorum/recovery contract. No executor rewrite
or durability shortcut is justified by the endpoint means alone.

## What Redis changes about the comparison

The reference Redis 7.0.15 uses a dictionary for ordinary string operations,
with command execution owned by its main thread. Its GET/SET path does not
traverse a replicated ordered state machine. That removes both required quorum
work and implementation-specific ownership transfers from this reference.
Redis's asynchronous replication is not equivalent to KV9's write contract.

KV9's resident index is `rpds::RedBlackTreeMapSync<Vec<u8>, Vec<u8>>`, a
structurally shared ordered tree. This supports ordered scans and independently
pinned views, but does not make writes free. Capturing a root is O(1); updating
a shared path may copy nodes, with pointer traversal and reference-count work.
Local RPDS 1.2.1 insertion uses `SharedPointer::make_mut` along shared paths.
The comments saying writes never pay for snapshots are incorrect: snapshot
capture is constant-time, while later writes can copy shared nodes. Node
copies share entry pointers, so this is neither a full-tree copy nor universal
deep-value copying. Its contribution to current latency is still unmeasured.

Both `MemEngine::write` and `write_applied` receive an owned `WriteBatch`, then
iterate borrowed mutations and clone PUT keys/values before insertion. The
Raw command-to-batch conversion introduces an earlier ownership conversion.
These are concrete candidates for eliminating redundant buffers while keeping
mutation order, snapshot isolation, batch atomicity and applied-position
publication intact. A hash-only replacement would also change ordered-scan
behavior; any index experiment must keep the existing engine contract.

The [completed write CPU diagnostic](WRITE-APPLY-CPU-PROFILE.md) now attributes
41.903% of BatchPut(64) selected CPU samples to the bitwise engine CRC loop.
An equivalent byte-table computation is therefore the next isolated candidate;
buffer ownership and index changes remain separate follow-ons. Preserve existing group commit and the
commit/apply acknowledgement contract. An apparent shorter call graph is not
proof of better latency: recent transport and scheduling screens have already
shown that distinction.

An independent source review confirmed the relevant Raw/runtime/engine files
are identical between the two measured server roles. The smallest ownership
experiment consumes the final batch only after WAL append/sync, validates the
applied position before mutation, and computes the data-revision flag during
ordered consumption. Separate follow-ons can consume command preparation and
apply lowering. An overwrite-specific RPDS `get_mut` path is less certain:
shared snapshots may force cloning the old entry/value, and a missed lookup
can copy paths. Keep that as a separately measured hypothesis rather than an
assumed faster replacement.
