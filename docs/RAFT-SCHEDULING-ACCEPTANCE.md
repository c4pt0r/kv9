# Event-driven Raft runtime acceptance

Tracking: #41, #13 and #9. The reviewed runtime is
`cc8bc87b6b34d07c76eb2019651aff7574f5fea1`, consisting of scheduling `4201d04`,
queued outbound coalescing `032f683`, completion notification `b4a74b2` and
accepted-socket configuration `cc8bc87`. This boundary excludes subsequent
Raft append batching, Raw engine group commit, Ready joint synchronization and
the indexed read-receipt experiment.

The implementation is integrated on main through `a9510e2`. All 127 tracked
runtime/build inputs match the accepted candidate byte-for-byte; the integration
build is checked locally. Candidate test and benchmark executions retain their
original identities. The original work packages remain open.

## Composition review

The [scheduling proof](RAFT-SCHEDULING-PROOF.md) checks 386 obligations at
`4201d04`; the [completion/outbound proof](RAFT-COMPLETION-PROOF.md) checks 532
obligations at `b4a74b2`. Each retains its exact source boundary, counterexamples,
semantic controls, fairness premises and limitations. Their composition is a
reviewed refinement argument, not a mechanically extracted proof of all Rust.

The source audit at `/tmp/kv9-cc8-composition-source-audit.json` establishes:

- All five source-mapped Raft files at `cc8bc87` are byte-identical to `b4a74b2`.
- `rawnode.rs` and `transport.rs` also remain byte-identical to `4201d04`.
  The production WorkSignal, TickDeadline and RaftInbox definitions are unchanged.
- `RaftPeer::process_ready` remains byte-identical to accepted runtime `f2c1f6f`.
  The dependency lock and storage implementation are unchanged. The full changed
  path inventory is retained; no engine or metadata algorithm changes are present.

The owner still consumes its signal before draining admitted work. Completion
publication runs after the observed turn, after state guards are released. Its
generation mutex is a leaf: neither the publisher nor a waiter takes application
or peer locks while holding it. Observing, checking an exact predicate and then
atomically registering against the observed generation therefore composes with
the existing publication order. Additional notifications are stutters with
respect to the Raft state. They cannot manufacture quorum confirmation, a
write receipt or apply coverage. Exhaustion permanently fails the signal and
fences an otherwise successful owner turn; fatal paths publish their state before
notifying. A concurrent operation still needs its independent exact authority.

The owner claim and pump gate remain per driver. A completion waiter cannot
consume Ready or inject a tick. The outbound coalescing loop consumes only a
bounded available prefix and retains the captured destination allocation and
session. Its intermediate queue inspections do not change the owner signal,
Raft log or receipt state; finishing the batch offers the same FIFO sequence to
the existing transport. The count bound includes stale work. Arrivals between
successive nonblocking receives can join the current batch, without any wait for
future arrivals. Network cancellation/loss remains allowed by the transport
contract, and no new eventual-network-delivery premise is introduced.

The socket change wraps the already reserved listener in pinned tonic 0.14.6
`TcpIncoming`. Its inspected implementation retains the listener, applies
`set_nodelay` to each accepted socket before yielding it, and logs option failure
without granting any protocol authority. The production-helper regression reads
the option from two actual accepted sockets and verifies the address remains
reserved. Changing TCP buffering refines the existing asynchronous network
schedule: request identity, message order, authorization, durability and quorum
conditions are unchanged. There is no new timing premise for safety.

The prior Ready/apply protocol supplies durability and exact-publication
authority. Bounded inbox and committed-prefix service can change turn boundaries;
they do not permit publication beyond the successfully processed prefix. Tick
progress remains conditional on eventual owner/lock/storage service. Synchronous
I/O can still delay a tick; this increment is not a real-time deadline proof.
The driver and queue are per peer lifetime and add no service-critical database
singleton, proxy, collector or clock service.

## Local checks

The exact clean candidate passes 589 workspace tests/doctests with 23 explicitly
ignored tests, warning-denying workspace/all-target Clippy, and a default build.
The fresh three-process Raw fixture verifies typed context refusals, replication,
leader replacement, new writes and deletes after failover, and original-store
restart through term 2 / index 14.

The process executable SHA-256 is
`62dfd1ce29766b42b3281e0f676a379dcd8a72f682d53853393fcd87fd5b0bd9`, unchanged
before and after the fixture. Cargo records features `[]`. Local checks used
logical CPUs 6-31 on the shared host. The workspace run used the worktree-local
target because the shell lacked `CARGO_TARGET_DIR`; Clippy, the successful default
build and process run explicitly used the shared target. The first standalone
build selected the wrong package for the binary and failed before compilation;
its original log is retained beside the corrected successful invocation.

Earlier deterministic implementation controls remain bound to test-only commit
`8e738f2` over runtime `b4a74b2`: seven wakeup and six latency-observer triples.
They are separate from the protocol-model controls and from the fresh exact
candidate checks. Actual MinIO and Chaos Mesh use a separately identified default
executable; neither binary hash is relabeled as the other.

The local archive is
`target/correctness-evidence/2026-09-09-cc8-local-promotion.tar.gz`: 182 entries,
54,495,193 bytes, SHA-256
`c4fdeff9c922cf2ba3217faf01b57c6561f2457b32dca1ac1dbd5f554e51e0dd`.
Its manifest SHA-256 is
`de2a84ac5e72ce2306a9902f16a949b6a41aad5bf88ac01cfbf6820b54290fb0`.
Every entry was decompressed and compared with its original bytes. The archive
retains the executable, runtime/build inputs, fixture stores, logs and audits.

## MinIO and actual Chaos Mesh

The [exact-candidate fault report](CC8-MINIO-CHAOS-ACCEPTANCE.md) passes real
MinIO checkpoint/reclamation/restart and the unchanged 21-window Chaos Mesh
matrix. Independent checks repeat both full-history validators on a fresh copy
and verify all six effect families. The 5,001 CLI operations include 4,510
successes, 480 unknown outcomes and 11 refusals; 1,294 persistent-client operations
include 1,277 successes, 14 unknown outcomes and three refusals. Unknowns remain
unknown in the evidence and checker. Every fault window includes observed
majority service from both clients; removing the history collector also leaves
the database serving reads and writes.

All six per-voter EIO/ENOSPC cells observe the error reaching Raft, terminal exit
and recovery. Each voter also refuses repeated missing-log starts and a
replacement PVC carrying its old bundle, then recovers the original store.
The owned test namespace is removed and all eight pre-existing namespace UIDs
are preserved. These are process/Pod faults on one shared Kind host, not physical
host-loss or power-loss acceptance. Object-store availability remains an explicit
external dependency.

## Performance disposition

The [accepted-socket paired benchmark](../scripts/redis-reference/results/b4a74b2-cc8bc87.md)
retains its fixed protocol and actual
outcome populations. Its high-concurrency write population includes 88 unknown
writes and 213,735 admission refusals, with zero residual backend jobs. The
read-path improvement does not establish write capacity, zero-error overload
service or Redis-class performance. The independently evaluated later Ready
candidate and volatile diagnostic retain their separate results and guarantees.

The next implementation path remains [RawKV performance development](RAWKV-PERFORMANCE-PATH.md):
bounded proposal submission, explicitly ordered persistence overlap, and measured
read-path CPU reduction. Indexed receipt lookup remains an unproven experiment.
All daily validation is local; no hosted workflow is dispatched by this increment.
