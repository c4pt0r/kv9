# Asynchronous exact-apply waiting

Tracking: #20 and #9. This candidate starts from resident reads `11cae97` and
changes point PUT, batch PUT and point DELETE scheduling. It does not include
the separate write-serialization experiment. The client-visible goal remains
memory RawKV performance approaching Redis under recorded comparable workloads.
This document describes an implementation candidate, not performance selection
or completion of the industrial correctness gates.

## Execution and receipt contract

The public handler transfers its existing count/encoded-byte reservation to a
private Tokio task. A blocking worker owns that same reservation while queued,
validating the context, planning the write, constructing its fenced command and
submitting the first proposal. It returns the reservation and an asynchronous
completion future to the private task. The worker is then available for other
requests while this task awaits the exact apply outcome.

The public RPC only awaits the private task's JoinHandle. Dropping that handle
on cancellation detaches the task; no abort handle is exposed. The private task
retains public admission through the logical outcome, including replacement
retries. The pinned Tokio version is 1.53.1; its
[JoinHandle ownership contract](https://docs.rs/tokio/1.53.1/tokio/task/struct.JoinHandle.html)
and [blocking task contract](https://docs.rs/tokio/1.53.1/tokio/task/fn.spawn_blocking.html)
are assumptions about the executor boundary. Runtime destruction can abort async
work; it is outside the claim that an ordinary canceled RPC retains ownership.
Preparation or completion panic drops the owning reservation and is reported
through the task join error. The generic backend fallback still performs its
entire synchronous operation on the blocking worker.

Each NodeDriver has a 128-reservation apply registry, using the existing Raft
owner and work signal. Reservation occurs before proposal submission. The count
covers unsubmitted reservations, queued requests, and requests temporarily
extracted by the owner for inspection. A full/stopped/expired reservation refusal
cannot submit a proposal. A stop after submission instead returns a failure with
an unknown application outcome; it does not promise that the proposal was absent.

After the entire pump and its completion publication succeed, the owner services
registered waits using the same `inspect_applied` discriminator as synchronous
waiters. It returns the recorded term, index and exclusive outcome from the
existing applied receipt ring. A different term at the exact index is Replaced.
A missing receipt below a full ring's oldest retained index is Unconfirmed,
before any passed-watermark replacement decision. A missing, non-evicted index
passed by the established application watermarks is Replaced under the existing
ring/publication contract. Commitment alone cannot authorize success.

Fence rejection remains StaleEpoch. A manifest receipt on this non-manifest path
is an error. Unconfirmed and driver failure are never retried. Only Replaced
permits a new submission, using the same Arc-owned fenced command and the same
absolute deadline. Synchronous and asynchronous paths share `settle_proposal`.
The production asynchronous loop owns the command/deadline and passes them into
its submission effect; tests can inspect those actual arguments.

Registration notifies the owner after queue insertion and mutex release. This
covers application before registration even if no later Raft traffic arrives.
Dropping or timing out a receiver closes it before notifying the owner. Stop
marks the registry stopped and fails its queued requests; extracted requests
remain owned until inspection finishes and are not restored into a stopped
queue. A result selected from valid application evidence may survive a racing
stop. Stop does not retroactively invalidate an already eligible receipt.

The original logical deadline starts immediately before first submission. It is
not a hard I/O interruption deadline: submission can block on persistence, as it
could previously. At wait timeout the result is unknown and the detached task
can finish; the proposal may still apply later. Admission bounds waiting work,
not the lifetime of every proposal ever written to the Raft log. The existing
no-retry rule for unknown outcomes remains necessary.

## Conditional refinement proof

Fix an arbitrary finite execution. Assume the existing Raft safety, durable
Ready publication, applied-ring retention/order, fence evaluation and
known-not-applied replacement contracts. Also assume Rust's unique ownership and
the executor behavior stated above. These premises are not proved by this change.

For each attempt, let its registry state be Unreserved, Reserved, Queued,
Extracted or Terminal. For each logical public write, let its reservation owner
be Handler, OwnedTask, BlockingJob or Terminal. The complete transition classes
are:

| Transition | Registry effect | Public ownership/evidence effect |
|---|---|---|
| Accept public request | None | Handler owns one reservation |
| Start private task / blocking job | None | Move the same reservation without cloning |
| Reserve before submit | Increment only when count < 128 and not stopped/expired | BlockingJob retains ownership; no receipt |
| Submission refusal/unwind | Destroy Reserved | No successful receipt; finish/drop public owner |
| Register | Reserved -> Queued without changing count | Store the exact ProposedAt, original deadline and one sender |
| Extract | Queued -> Extracted without changing count | No callback or notification under the registry mutex |
| Still pending | Extracted -> Queued if not stopped | Preserve position, deadline, sender and reservation |
| Select exact outcome | Extracted -> Terminal; release before sending | Send only the common discriminator's result |
| Receiver timeout/cancellation | Close receiver, wake owner | No success constructed; unknown outcome cannot authorize retry |
| Stop / owner destruction | Reject registration, drain/drop retained work | Send failure or preserve an already selected valid result |
| Replaced with time remaining | Reserve a fresh attempt | Same command and original deadline; same public reservation |
| Terminal logical outcome | No live wait owned by this task | Finish/drop the sole public reservation |

Induct on these transitions with the following invariants.

1. **Capacity conservation.** While a registry exists, its count equals the
   number of live Reservation values, including values in a callback's extracted
   vector. Reserve is the only increment and tests the bound under the same
   mutex. Moves preserve this number; each unique Reservation destructor performs
   one decrement. No callback/queue transfer releases it early. Therefore the
   count is always in [0, 128]. Destruction of the registry makes its count
   unobservable and retained weak owners cannot form a cycle.
2. **Single public owner.** A live logical write has exactly one public
   reservation owner. Each task/worker handoff moves that value. RPC cancellation
   drops only the waiting JoinHandle after the private task is started. The task
   finishes its reservation after the logical result, or unwinding destroys it.
   A replacement changes neither this owner nor the original input charge.
3. **Exact success evidence.** A successful outcome is produced only by the
   common applied-ring discriminator after successful owner processing. It uses
   the ring's recorded position and verdict for the requested term/index.
   Submission, queueing, elapsed time, commitment, receiver loss, and registry
   destruction cannot create an Applied outcome. One sender is consumed at most
   once, and the sender is removed before Request destruction can send failure.
4. **Retry safety and finite budget.** The common settlement function permits a
   retry only for Replaced while the original deadline has not expired. The loop
   retains one command and one deadline and passes both to each retry submission.
   Reserve checks that same deadline again before the proposal effect. Other
   outcomes terminate; there is no path from Unconfirmed to a new proposal.
5. **No lost ownership at stop.** Stop and queue restoration serialize on the
   registry mutex. If stop wins, extracted pending requests are destroyed outside
   the mutex. If restoration wins, close extracts and destroys them. A Reserved
   value concurrent with stop cannot register, and its failure path drops it.
   Extracted requests and unsubmitted reservations remain counted until the
   respective actual owning work ends.

The empty state establishes all five. Acceptance and reserve establish 1–2
without evidence. Registration, extraction and restoration only transfer owned
values and preserve 1–5. Selection checks the inherited discriminator, which
establishes 3; its sender removal and reservation release preserve 1 and 5.
Terminal errors and closed receivers create no successful evidence or retries.
The only retry transition checks Replaced and preserves the command/deadline,
establishing 4 for the next attempt. Stop/restoration have the two exhaustive
mutex orders in 5. Destruction consumes ownership without creating a successful
result. These cases exhaust the table and prove preservation for every finite
execution under the stated premises.

Erase queue moves and executor scheduling from a successful asynchronous
execution. Its remaining sequence is validation, proposal, exact application
verdict, zero or more known-not-applied replacement retries of the same command,
and a response with the recorded receipt. This is an allowed sequence of the
existing synchronous write protocol. The change therefore preserves that
protocol's success/retry safety conditionally; it does not establish equal
wall-clock error timing or independently prove the underlying consensus system.

Progress requires fair executor/owner scheduling and finite submission callbacks.
With retained evidence, registration wakeup enables a new inspection even when
application preceded registration. Otherwise a live quorum and eventual apply
are required for success; partitions can yield an unknown outcome. A timeout
closes the receiver and wakes cleanup, but neither forces blocked storage to
return nor cancels a logged proposal. No new cluster service or service-critical
singleton is introduced.

Machine-checked composition, exact-candidate Chaos Mesh fault/history acceptance,
and performance selection remain promotion gates. A written conditional proof
and compiled semantic controls are not substitutes for those gates.

## Cost and local validation

The design removes blocking-worker residence during apply waiting, but adds an
owned async task, oneshot handoff, bounded registry mutex traffic and owner-side
receipt inspection. Each service turn inspects at most 128 requests; the existing
ring has at most 1,024 entries, so an unsuccessful full scan can perform up to
131,072 ring-entry comparisons per turn. This is a bounded cost, not evidence
that the candidate improves throughput. No busy requeue notification is emitted
for a still-pending request; apply/registration/timeout/stop supply progress wakes.

Status exposes `raft_async_apply_{limit,queued,in_flight,peak,stopped}`. Application
wait latency is recorded per attempt; logical proposal latency is recorded once
across retries. Public count and encoded-byte limits are unchanged. Their byte
charge is the existing encoded request charge, not a total heap-use measurement.

Focused Rust tests cover committed-but-unapplied writes, exact fence/manifest/term
outcomes, fatal closure, late registration on an idle owner, evicted receipts,
reservation before submission, extraction/cancellation/stop ownership, unchanged
retry command/deadline, forbidden retries, public cancellation across queued and
async waiting stages, panic cleanup, and actual fenced PUT/batch PUT/DELETE effects.
`scripts/check-async-write-controls.py` requires seven independently compiled
production mutations to fail their named semantic assertions, with baseline and
restored passes, unchanged tests, private build outputs and retained identities.
Compile failures and timeouts cannot count as expected semantic failures.

The acceptance record must state the exact revision, default features, complete
local test results and process identities. Prior resident-read Chaos evidence
retains its original source scope. The comparison against resident `11cae97`
must use the unchanged benchmark client and workload validation, before/candidate/
after cohorts, actual executable attestations and drained process lifetimes.

### Local implementation record, 2026-09-10

- Workspace tests and doctests: 643 passed, zero failed, 23 ignored.
- All-target workspace Clippy with warnings denied and formatting checks passed.
- Seven compiled semantic controls passed all 21 baseline/mutant/restored phases.
- The unchanged default three-process RawKV fixture passed writes, leader loss,
  reads/writes after election, deletion and original-directory restart. All four
  observed process lifetimes executed the default binary with SHA-256
  `72f72c49bf16307f1b9c91a99db7ef41555e847f752e9da78e51623a168c8628`
  and exited. Source and executable identity were unchanged during the fixture.
- Retained paths: `/tmp/kv9-async-write-wait-workspace-first.log`,
  `/tmp/kv9-async-write-wait-clippy-first.log`,
  `/tmp/kv9-async-write-controls-first`, and
  `/tmp/kv9-async-write-wait-process-first`.

These are local implementation results. They contain no new throughput result,
checked protocol composition, actual Chaos Mesh acceptance for this candidate,
cross-host failure claim or power-loss claim. Hosted CI was not dispatched.
