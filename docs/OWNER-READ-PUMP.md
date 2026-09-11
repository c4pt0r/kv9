# Owner-local ReadIndex submission

Status: experimental; correctness and matched runtime evidence are pending.
Parent runtime: selected CRC baseline. RPC, read group size and admission
window are unchanged. No performance improvement is claimed before measurement.

## Change and cost hypothesis

The driver used the public `RaftPeer::read_index` wrapper to submit its sealed
async read group, then immediately pumped the same peer. The public wrapper
sets the owner's pending bit. In an otherwise isolated read turn that bit
forces another empty owner turn before parking. The private admission primitive
now lives behind a drain-token operation that submits and pumps in sequence;
external synchronous submissions retain the public notifying wrapper.
Incoming `step_message` did not notify and is not changed by this experiment.

## Conditional correctness argument

The proof boundary is one `DrainToken::pump_with_read_submission` invocation
under the existing driver's exclusive pump gate, with the existing peer lock,
bounded callback and single owner. It does not prove the Rust compiler,
scheduler, transport or the whole Raft implementation.

1. The private admission primitive preserves the old fatal-state, leadership
   and committed-current-term guards and `raw.read_index` transition under the
   same peer lock. Submission still means neither confirmation nor completion.
   The public wrapper notifies only after successful admission and after the
   peer guard is released, as before.
2. The non-cloneable, same-peer drain token passes a borrowed admission callback
   directly to the sealed-group registry and then invokes that peer's `pump`.
   Every ordinary return, including a typed pump error, follows this invocation.
   No return, wait, timer or owner-loop boundary lies between them. An unwind
   or process abort is outside this ordinary-return progress guarantee.
3. Neither the primitive nor the token operation clears pending work. Any
   external notification after the owner's earlier `begin_turn` survives until
   the next owner-loop check. A notification racing with park is still protected
   by the existing shared mutex/condition-variable protocol. Multiple publishers
   collapse to the same boolean hint; no publisher's queue ownership changes.
4. Omitting this internal hint cannot hide unprocessed submitted Ready work:
   the same peer is pumped immediately, and the existing successor-Ready check
   remains in the owner loop. Retained async prefixes, pre-current-term election
   deferral, transport publication, cancellation and stop retain their existing
   explicit scheduling paths. This conclusion assumes the existing bounded
   service and owner scheduling premises; it is not a wall-clock latency bound.
5. The exact-context quorum confirmation, complete driver-turn success,
   persistence failure handling, applied-index fence and resident read-view
   authorization are unchanged. The token helper returning does not authorize
   a read response. No lease, stale read, early write acknowledgement or weaker
   quorum rule is introduced.

`OwnerReadPump.tla` models the local sequencing and retained-notification
obligations in points 2–3, including typed failure and abort. Its induction
proof covers arbitrarily many interleaved boolean notifications within that
transaction. It intentionally does not model a quorum certificate or successful
read. This is a source-mapped local proof plus executable composition tests,
not an automatically extracted proof of the implementation. The unchanged
`RaftSchedule`, `ReadAdmission` and read-completion models retain their original
scope; their historical source maps do not automatically prove this revision.

## Source mapping and executable checks

| Obligation | Implementation | Focused evidence |
| --- | --- | --- |
| Admission keeps the same Raft guards | `rawnode.rs`: private `admit_read_index`, public `read_index` | Existing leadership/election admission tests |
| Owned admission invokes the same peer's pump | `DrainToken::pump_with_read_submission`, `NodeDriver::step_inner` | Real three-voter same-turn quorum messages; omit-pump control |
| No internal extra turn | Token callback uses the private primitive | Actual owner park observer and pump counter; self-notify control |
| External work remains signaled | Public wrapper and unchanged `WorkSignal` | Parked synchronous read; real follower ACK delivered during callback; clear-pending control |
| Retained suffix progresses | Unchanged async registry suffix notification | 65 real waiters, two sealed groups and two owner turns |
| No premature read completion | Existing complete-turn and apply fences | Real quorum ACK remains insufficient while committed apply is paused; existing failed-apply test |

All new test observations use bounded synchronization and real Raft messages.
No forged context, watermark or timer wake is used to rescue the asserted path.
Mutation controls must first pass the baseline, fail at the intended assertion,
and pass after exact restoration, using freshly compiled first-party artifacts.

## Acceptance route

Run local source checks and strict model/proof controls, then the original
stream/unary recovery workload. Screen the candidate against CRC and Redis
with the unchanged c1/c64 point read50/read100 protocol. Evaluate per-repeat
throughput, single GET mean and histogram tails, and mixed GET tails separately.
Only a useful candidate proceeds to the larger matrix and exact-source Chaos
Mesh promotion gate. Hosted CI remains manual. A rejected screening experiment
is retained with its measurements and does not replace the selected runtime.
