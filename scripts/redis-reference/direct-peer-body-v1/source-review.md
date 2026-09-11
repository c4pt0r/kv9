# Direct peer body: independent source and test review

Verdict: no concrete safety or liveness blocker found in the reviewed implementation under its single-peer-owner and normal cooperative-executor premises. This is source reasoning plus review of the 16 component tests and existing gRPC test coverage. It is not a model-checked refinement, process/Chaos acceptance, or performance endorsement. Root reports the focused grpc:: suite passed 41 tests, including all 16 new component tests and the frozen-reader/blackhole tests; this reviewer ran no Cargo, test, fixture or benchmark command.

## Ownership and route boundary

Enqueue remains serialized with route registration by GrpcTransport.peers. Each envelope retains its immutable PeerDestination allocation, so A→B→A does not reinterpret old traffic. Each Receiver.open additionally creates a fresh Arc token even for same-route reconnect. Token validation, dequeue, receive-waker replacement and invalidation share one queue mutex. A stale body does not mutate the queue or its stored waker; cooperative exhaustion may first return Pending without touching either, followed by terminal readiness on a later poll. Old Session/Body Drop clears only its own still-active token. Receiver Drop separately closes admission and discards the queue, even if tonic retains a body.

The worker creates Session before handing Body to tonic. Normal RPC completion explicitly drops the guard; route-select cancellation and worker abort also drop it. The next session is opened only after that owner future is dropped. A batch already selected before invalidation may still have escaped into tonic/HTTP2. Likewise, an enqueue racing a route update may be inspected by the old body before the owner handles the watch change and be discarded by its destination filter. These are best-effort transport behaviors, not claims that all admitted traffic is delivered or immediately recalled. Inbound root/token/sender/destination/authority checks are unchanged.

No queue operation takes the outer peers lock, so the observed lock order is peers→queue without a reverse acquisition. Explicit wake calls and outage/receiver discarded-queue destruction occur outside the queue mutex. Body polling still clones/replaces a Waker and allocates/drops bounded envelope storage under that mutex; the argument assumes the pinned Tokio/tonic waker and ordinary allocator behavior, not arbitrary reentrant RawWaker vtables or bounded allocator wall time.

## Watchdog reasoning

Session.stalled registers Notify interest with enable() before reading the state. A send before registration leaves a permit; a send after registration wakes the registered owner. At most one stalled future is live in the actual peer-session call path. This is a necessary usage premise: the private API does not promise simultaneous old/new session watchdogs are independent consumers of notify_one. Body and Session cancellation are fenced independently from the body-waker slot.

The timer runs in the peer owner, independently of HTTP2 body polling. A deadline wake rechecks token and the current pending_since under the mutex, so a previously armed timer cannot expire a backlog whose valid dequeue reset its timestamp. Empty state does not expire. Not notifying on a valid dequeue or empty transition can cause a harmless wake at the old deadline; the recheck prevents a false expiry. New work on an empty queue notifies the owner. Repeated sends and stale inspection do not reset the age of a continuously nonempty backlog. A truly empty queue followed by new work intentionally starts a new budget.

Progress here means emitting a nonempty current-route batch from the body. It does not attest network write, remote application receipt, or quorum confirmation. Once no envelope remains queued, the backlog watchdog is idle even if previously emitted bytes remain in HTTP2; keepalive and subsequent traffic retain their distinct responsibilities. A deadline decision may win immediately before an otherwise-ready dequeue; reconnect/loss in that ordering is allowed. Scheduling starvation can delay the watchdog beyond three seconds, and sustained producer/lock contention has no proved wall-clock bound.

## Bounds, closure and tests

The shared queue retains at most 4096 envelopes, with at most 128 inspected in one body poll and the same soft 1-MiB batch target. This is not a strict queue-byte or whole-tonic-memory bound. Outage discard is a single bounded queue take. Stale-only prefixes self-wake when work remains. poll_proceed is outside the mutex; inspected or terminal work charges budget, while an empty Pending poll restores it. The spawned current-thread test verifies repeated Ready batches eventually yield without consuming the next envelope and resume after a scheduler yield. It does not prove a scheduler fairness or real-time bound.

The tests meaningfully cover same-route and A→B→A retained-body/waker/Drop fencing, abort-before-first-poll, receiver closure with an idle waker and with queued work, sender EOF, queue overflow/outage reuse, FIFO/count/soft-byte rules, stale-only continuation, and independent watchdog expiry while the body is unpolled. The controlled invalidation test proves the token has been cleared and the mutex released while wake cleanup is blocked; replacement dequeue succeeds during that interval. It is not exhaustive exploration of all dequeue/revocation races. Watchdog tests preserve the three-second production budget, using controlled timestamps for expired/non-progress cases and one real idle wait. The armed-reset test uses a 100-ms remaining deadline and a 150-ms outer observation; severe scheduler delay can fail its arming assumption, so retain such a failure rather than treating it automatically as a runtime bug.

The deterministic tests cover representative empty-to-nonempty Notify ordering and stale wakes, not every Notify interleaving. Existing real gRPC route migration, stalled-reader, established blackhole and connect-budget cases remain relevant but do not replace exact committed-release process identity/restart validation or actual fault acceptance. No new process E2E, Chaos, all-workspace or performance acceptance is established by this review. The change does not intentionally alter core Raft state transitions; any formal safety inheritance remains conditional on unchanged guards and the permitted best-effort transport model.

## Reviewed hashes

- /tmp/kv9-direct-peer-body/crates/raft/src/grpc.rs: `94678473f675a3e2f352f280511986bec4b6c370551718f7ae1523d3b498cb37`

- /tmp/kv9-direct-peer-body/crates/raft/src/grpc/direct_body.rs: `b4713f80b05e01067ef99a2f540301bfe73e9c6d6cdfc02b43ca221a0ca6edc6`

- /tmp/kv9-direct-peer-body/crates/raft/src/grpc/direct_body/tests.rs: `81520e0b68129345c48c24a59fd48af17af878ef06f60af7dc83631fabb8d25e`

- /home/dongxu/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/tokio-1.53.1/src/sync/notify.rs: `58995a30236252e2825f0311a08959635e8044ffe2796c0a59562dffbe008638`

- /home/dongxu/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/tokio-1.53.1/src/task/coop/mod.rs: `e6efc19435559c8621d30aab8e37e0ad652857f8eefc170f73bf8dac653729de`

## Addendum: deterministic expiry and non-poisoning test failure

The original 7189 bytes above are preserved verbatim; their SHA-256 is `f27ff92e3ed02eb22814540d7707ab9fd9c24d1e86f53c8a5c9cb379f5a0a111`. The retained correction/before copies of direct_body.rs and its tests match the original reviewed hashes. This follow-up inspected only those two source deltas, the retained first mutant failure, and the new source-mapped argument. No build, test, fixture or source-control execution was performed by this reviewer.

No concrete blocker was found in these deltas. Session::stalled now checks the current locked backlog deadline immediately after its ownership/closure predicate, before selecting between Notify and the timer. An already-overdue backlog therefore terminates without relying on timer-versus-notification selection. The timestamp remains read under the same mutex as dequeue/reset, and the existing timer-arm recheck remains intact. Valid progress ordered before the check uses the refreshed timestamp; expiry ordered before progress may close that session, as already permitted. The change does not mutate queue/token/waker ownership or redefine progress. It still requires the owner to be scheduled and acquire the mutex; it does not provide a hard wall-clock recovery bound.

The watchdog test now copies Option<Instant> into observed_pending in a separate statement before asserting the same equality. The temporary MutexGuard is released before the assertion can panic. The explicit marker is `stale traffic reset the backlog age`. Thus the intended mutant can fail the same semantic comparison without poisoning the queue during test unwinding; no acceptance predicate is relaxed. The original mutant log independently shows the failed equality, subsequent PoisonError in owner cleanup, and SIGABRT. Its SHA-256 is `bbe4f30da3c54ba78a121fea5907ccb883a4e5c2166137f5bdb20e901ebe52a4`, at `/tmp/kv9-direct-peer-body-route-controls-first/enqueue-postpones-stall-deadline/mutant.log`. That first control remains rejected. Correction rationale and pre-edit sources remain in `/tmp/kv9-direct-peer-body-review-correction`.

The new docs/DIRECT-PEER-BODY.md is consistent with the reviewed source boundary: separate route and RPC identities, one mutex for token/dequeue/waker transitions, conditional progress, a soft batch-byte target, and no claim that body dequeue establishes remote receipt. Its stated invariants are a deductive source argument, explicitly distinguished from machine-checked whole-system refinement. The expiry wording accurately reflects the new direct predicate. Its test/control discussion preserves the rejected first run and keeps process, Chaos and matched measurement as separate gates. No material wording correction was identified.

Root reports 224 Raft tests and Clippy passed, and nine corrected source-control triples passed before adding explicit split-out test-source binding. The final stronger-bound control run was still pending at this review handoff. This addendum does not independently certify that execution or broaden the original review into process/fault/performance acceptance.

Current reviewed SHA-256 values:

- crates/raft/src/grpc.rs: `94678473f675a3e2f352f280511986bec4b6c370551718f7ae1523d3b498cb37`
- crates/raft/src/grpc/direct_body.rs: `8de391879f192afbeca9321b147bf1058301cd45b2b8d07e58791dc4bf229ef6`
- crates/raft/src/grpc/direct_body/tests.rs: `a23cf0eae30b4415273527d2fbd9b84c0fb606fb72196be1d6f63562176df47b`
- docs/DIRECT-PEER-BODY.md: `b2a0c15de277a34fb15134e68d6962a2a891ecd226d55b18b7f7250722950a05`
