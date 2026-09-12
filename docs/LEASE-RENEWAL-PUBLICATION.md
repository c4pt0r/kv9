# Experimental lease renewal and pump publication

Tracking: [#9](https://github.com/c4pt0r/kv9/issues/9), #20.

This adapter binds the [controller](LEASE-CONTROLLER.md) and
[durable voting installation](LEASE-VOTE-BINDING.md) to renewal messages and the
owning Raft driver. It does not enable server lease reads, mint a `ReadBarrier`,
qualify a production clock, or provide a lease performance measurement. The
selected runtime remains `11113f6` with Safe ReadIndex.

## Authority and messages

One private leader controller is created for each actual leader term and durable
installation incarnation. Its authority includes group, fixed configuration,
node, term and incarnation; each renewal additionally has a sequence and
revocation generation. All observations and decisions use the peer mutex that
owns `RawNode`. A clock error, domain change or regression fences the complete
installation, including future leader lifetimes. Transfer revokes local lease
authority without clearing the voter's promise. A revoked controller is not
rearmed in the same term.

Exact versioned Request and Grant envelopes travel on the existing Raft transport.
The namespace combines a reserved heartbeat `log_term` marker with distinct
payload kinds. Ordinary heartbeats use zero `log_term`; arbitrary Safe ReadIndex
contexts cannot collide with this namespace. An old peer echoing a request's
context cannot manufacture a grant, even if it preserves the marker.

Decoding checks the destination, configured sender, outer message type/term,
authority, immutable timing policy, canonical voter list, exact length and empty
Raft payload. The payload is 126 bytes for three voters and at most 614 bytes for
64 voters, excluding the existing transport framing. No wall-clock timestamp is
sent. This remains a crash/recovery protocol under the transport's existing peer
identity premise, not Byzantine consensus.

A follower first passes a sanitized heartbeat to actual Raft processing and
then checks the actual local term and leader before making the voting promise.
The sanitized heartbeat has no context, entries or commit advancement: an
unbounded leader commit index must not be applied to a lagging follower's log.
A lower-term renewal cannot undo a higher term already observed locally.
Quarantine and old promises still gate grants. The self-ACK requires the same
local voting promise; membership in the leader process is not a free vote.

## Publication boundary and model mapping

| Concrete transition | Authority effect |
| --- | --- |
| Prepare a round before Ready | Anchor the deadline before any request send; stage a bounded request and actual self-grant. |
| Receive a matching grant | Collect its voter once in that exact pending round; do not activate a certificate. |
| Persist Ready/LightReady and capture under the same peer lock | Move staged decisions into one private, non-Clone, non-Copy pump publication. |
| Finish application and publish completion successfully | Recheck the actual authority/clock, then send captured grants/requests or activate the captured quorum. |
| Application/persistence failure, stop or clock fence | Invalidate lease authority; no successful local read can derive authority from that installation. |
| Higher-term traffic or transfer | Revoke the old leader controller; preserve the voter's existing promise. |

Each publication is bound to an allocation identity and checked monotonically
increasing capture sequence. The `DrainToken` is already unique per peer. A
foreign or superseded publication is an error, and consuming a publication
prevents duplicate completion. Inbound grants arriving after capture remain
staged for the next pump; they cannot authorize the earlier pump.

Normal Raft messages retain the existing Ready durability ordering. Only lease
messages/certificate activation wait for the whole driver's successful turn.
Public `RaftPeer::pump` cannot create a lease publication. Application pause may
allow authority acquisition, but cannot create an applied read view or bypass the
future per-read application gate. A certificate alone is not a readable snapshot.

The round deadline remains anchored before the original send. Delayed pump
completion consumes usable time; it never rebases that deadline. Publishing a
follower grant rechecks its original promise, and replaying an identical grant
does not extend the voter hold. Requests or certificates at the exact deadline
are discarded. Renewal attempts are bounded to one pending round, scheduled at
half the conservative leader duration when the existing driver runs. A delayed
driver or network can prevent renewal; there is no availability guarantee under
an indefinitely delayed scheduler.

These transitions implement the acquisition, collection, publication and
revocation events in the [authority model](LEASE-AUTHORITY-MODEL.md). Together
with the durable voting gates, exact grant matching and the
[clock containment proof](LEADER-LEASE-PROOF.md), they preserve the conditional
claim that a usable quorum certificate excludes a replacement leader. This is
an implementation mapping with local source tests, not a machine-checked
refinement of all Rust, raft-rs, filesystem, transport and clock behavior.

## Local validation

[Retained evidence](lease-renewal-publication-v1/README.md) records **231 passing
Raft library tests**, zero failed or ignored, including **12 new tests**: eight
peer/driver tests, three envelope tests and one controller deadline test. The
default-feature restart probe, experimental compilation and warnings-denied
Clippy also pass. Actual drivers acquire and renew a certificate, refuse expired
isolated authority, elect a replacement after the promises expire, and revoke
authority on stop. A failed application emits no staged grant and fences any
previously active certificate.

Eight separately compiled source faults fail their exact target tests: early
ACK activation, late input leaking into an older pump, expired request send,
foreign pump publication, missing fatal fencing, transfer retaining authority,
policy mismatch acceptance and a heartbeat echo treated as a grant. The eight
targets pass on both baseline and restored source. The 36 command outcomes and
source identities are retained; these test populations overlap the 231 tests.

The first compile used the wrong test import for `NodeDriver`. The next run
passed 230 tests but failed the new election fixture because lease-clock advance
did not advance Raft's separate election timer. The corrected fixture drives
actual ticks and accepts either eligible surviving voter as leader. Both first
failures and their source are retained. No production assertion, storage cap or
command timeout was relaxed. Inactive retained executables were compressed with
exact readback, and reproducible compiler caches were explicitly invalidated
before source runs under the original 80 GiB free floor plus 16 GiB reservation.

## Remaining service and acceptance gates

The server still needs to bind each read's fresh commit frontier to the exact
retained immutable applied view, validate metadata on that view, and perform a
final authority/clock check while preserving the original admission, deadline
and cancellation. No adapter method currently bypasses Safe ReadIndex for a
service request. Expired authority plus unavailable quorum must refuse the read.
Writes continue to require Raft majority commitment and the existing durability.

Clock-rate bounds must hold through pauses and supported host suspension modes;
monotonic samples alone do not establish them. An undetected stopped clock is
outside the proof and cannot be repaired by an expiry check. Fixed membership,
restart quarantine and the snapshot restriction remain experimental limits.
Actual lease-enabled Chaos Mesh partition, delay, pause and restart histories,
including the outstanding asymmetric/message-order cases, are required before
matched c1/c64 GET, mixed and batch throughput/latency comparisons. Historical
ReadIndex round timings are diagnostic and cannot be subtracted from total GET
latency to manufacture a lease speedup. Longer promises also increase the time
failure recovery may wait before a replacement can be elected.
