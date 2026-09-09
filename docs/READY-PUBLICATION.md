# Durable Ready publication

Tracking: [C04](https://github.com/c4pt0r/kv9/issues/14) and
[LightReady commit recovery](https://github.com/c4pt0r/kv9/issues/35).

## Defect and repair

`RaftPeer::process_ready` persisted entries and the original `Ready` HardState,
then called raft-rs 0.7.0 `RawNode::advance_append`. Reporting local persistence
can advance the commit watermark and produce a `LightReady::commit_index`.
The old integration published its committed entries without persisting that
watermark. `advance_append` also updates `prev_hs.commit`, so another `Ready`
need not report the missing commit again.

raft-rs permits an application to leave this commit watermark volatile. kv9 has
a stricter recovery contract: `DiskRaftStorage::committed_term` rejects an engine
applied position above the durable Raft commit. The old ordering could therefore
apply valid committed work and reject the resulting engine position on restart.
A single voter campaigning and proposing, followed immediately by disk reopen,
reproduces the missing watermark without sleeps, heartbeats or fabricated messages.

The repair snapshots the complete current HardState while holding the peer lock
and persists it whenever `LightReady::commit_index` is present. Only then may
the cycle extend outbound messages, committed-entry delivery or read-state
queues. This preserves term and vote along with commit. A storage error takes
the existing fatal path, clears pending queues and stops the driver; no work from
the failed cycle becomes eligible for application or acknowledgement. A later
request cannot repair an unreported durability error by continuing the same peer.

The extra synchronous write occurs only when `advance_append` reports a later
commit. This is a necessary recovery ordering, with a latency cost at that
boundary. Batching and asynchronous persistence must preserve this ordering and
receive their own refinement and throughput validation before adoption.

## State and implementation mapping

| Model state or action | Implementation meaning |
|---|---|
| `rpDiskEnd`, `rpDiskCommit` | Recoverable log end and HardState commit after storage synchronization/replay |
| `rpDelivered` | Highest committed index made available to the driver; an index projection of the queue |
| `rpApplied`, `rpAcknowledged` | Atomically positioned durable engine application and logical receipt eligibility |
| `RPCollect` | Collect one Ready cycle under the `RaftPeer` lock |
| `RPPersistEntries`, `RPPersistHardState` | `PersistentRaftStorage::append` and the original Ready HardState write |
| `RPAdvance` | `RawNode::advance_append`, including a possible late commit |
| `RPPersistLight` | The new complete HardState write before any cycle publication |
| `RPPublish` | Extend messages, committed entries and read states after all persistence succeeds |
| `RPApply`, `RPAcknowledge` | `NodeDriver::step`, positioned state-machine application and exact applied receipt checks |
| `RPFailBefore`, `RPFailAfter` | Storage error with no durable result or with an uncertain durable result |
| `RPCrash` | Discard volatile queues and reopen from the durable Raft/engine positions |

The error actions include entry-batch partial prefixes, not just all-or-nothing
append. A failing HardState write may recover either the old or the complete new
record. An in-operation crash maps to an allowed persistence outcome followed by
`RPCrash`; the mapping is not one Rust statement per TLA+ step. The model retains
acknowledgement history across restart only as an observer variable.

`rpPublished` records publication by the current/latest collected cycle. Failure
exclusion does not claim that earlier durable replies disappear from the network.
A concurrent `wait_applied` call that passed its fatal check can still return an
already valid receipt after another thread records a fatal error. Acknowledgement
in the model denotes logical eligibility, not the physical response delivery time.

## Assumptions and proven properties

The only module-level input assumption is `RPMaxIndex` being a positive natural.
Protocol/storage premises are expressed by the action relation:

1. Upstream Raft supplies valid commit cuts bounded by the persisted log and
   never replaces a committed prefix. Message identity, quorum intersection,
   elections and log matching are outside this index projection.
2. Successful storage synchronization survives the modeled crash. CRC-framed
   replay accepts complete valid records and rejects/truncates incomplete tails.
   Arbitrary corruption, Byzantine storage and rollback after successful sync
   are outside this contract.
3. Engine data and its applied position become durable atomically. Application
   consumes only delivered committed entries in order. The existing WAL and
   storage fault tests check this separately; it is not derived by this proof.
4. The peer serializes Ready processing. Fatal persistence errors prevent future
   driver application until reopen. Transport delivery and client scheduling can
   stutter without affecting the safety result.

The strengthened invariant includes parameter typing, all index bounds, phase
constraints, publication state and the chain
`acknowledged <= applied <= delivered <= durable commit <= durable log end`.
Initialization and every normal, failure, crash and stuttering transition preserve
it. Temporal induction then proves `RPSpec => []RPCoverage`, exclusion of failed
cycle publication, and the canonical action property
`RPSpec => [][RPFailedStop]_rpVars`. While failed, no applied/acknowledged position
can advance; crash may discard queued delivery and begins a new peer lifetime.

`ReadyPublicationProof.tla` has six theorem declarations and 75 checked proof
obligations for arbitrary legal bounds. It imports the same transitions as TLC.
The combined gate checks 47 declarations/534 obligations with 15 isolated controls,
including three Ready mutations that TLC also rejects with concrete traces.
See [the proof inventory](../proofs/tlaps/README.md) for exact toolchain pins,
assumption auditing, backend trust and fail-closed output requirements.

## Topology and progress limits

The deterministic late-commit reproduction uses one voter to make local
persistence complete the quorum immediately. This is not the production topology.
kv9 synchronously persists a Ready before sending its Append messages; in the
ordinary stable multi-voter path, acknowledgements for newly appended entries
therefore cannot overtake that local persistence. A three-voter recovery test
checks all replicas' exact applied positions after immediate modeled power loss;
it does not claim to exercise a multi-voter late-commit schedule. Joint membership,
future asynchronous persistence and upstream behavior changes need separate
reachability analysis. The model conservatively permits every valid late commit.

Safety allows infinite crashes, storage failures and stuttering. No availability
or liveness theorem is claimed. Conditional on eventual failure-free storage,
finite successful calls and fair driver scheduling, a collected cycle has a
finite sequence of persistence/advance/publication stages. A delivered committed
index then becomes applied and receipt-eligible if engine application and client
observation complete. This is a written conditional progress argument, not a
mechanized liveness proof or a service-level bound. Raft also needs a connected
quorum and an eventual leader; the object store remains an external dependency.

## Validation and remaining refinement

The memory-watermark and immediate disk-reopen regressions fail on the old
ordering and pass with the repair. The disk test checks recovered term, vote and
commit coverage. The modeled storage matrix reaches append, original HardState,
configuration state and LightReady HardState failures using EIO/ENOSPC before and
after write/sync boundaries. The three-voter test stops immediately after all
replicas apply the proposal and checks recovery after loss of unsynced bytes.

CI runs both protocol models, the full deductive inventory, Rust regression tests
and actual Chaos Mesh histories across voter failure, partition, delay and six
voter/errno I/O cells. The Chaos matrix demonstrates the integrated fatal/recovery
path; it does not claim every injection strikes the LightReady-only write. The
deterministic storage matrix exercises that precise boundary.

These tests and the source mapping do not constitute a mechanically verified Rust
refinement. Composition with metadata planning, term/vote/log identity, snapshot
installation, full dynamic-membership schedules, engine/object-store recovery and
cross-host availability remain open C04 and later roadmap obligations.
