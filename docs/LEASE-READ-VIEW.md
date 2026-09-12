# Experimental lease reads over exact applied views

Tracking: [#9](https://github.com/c4pt0r/kv9/issues/9), #20.

The experimental server can now serve prepared Raw GET and bounded BatchGet
from an exact retained view under a valid leader lease. Ordinary startup remains
Safe ReadIndex. No lease performance result, production clock qualification,
lease-enabled Chaos Mesh acceptance or Redis parity is claimed.

Installation requires the `experimental-leader-lease` server feature and an
explicit call to `NodeRuntime::start_with_experimental_lease`, supplying an
immutable `LeasePolicy` and a trusted, nonblocking `LeaseClock`. There is no CLI,
environment switch or presumed-safe default clock. The Raft adapter's
`ReadOnlyOption::Safe` is unchanged. Existing fixed-membership, snapshot and
restart restrictions in [the voting installation](LEASE-VOTE-BINDING.md) apply.
Writes retain their Raft majority, persistence and application requirements.

## Authority and ownership

`read_preparation_async` first reserves one slot in the existing 128-request
asynchronous read registry. The reservation owns the original context, start
and absolute establishment deadline. A local attempt does not allocate a
oneshot or submit a quorum read. If the attempt refuses or a required lock is
contended, the same reservation moves into the existing sealed Safe ReadIndex
queue with that context and deadline. It does not acquire a second slot or
restart the budget. Cancellation and stop retain the registry's bounded owner
cleanup. The outer public RPC admission and transport deadlines still apply.

The driver tries its `pump_gate`, then verifies stop/fatal state. Under the
peer's authority mutex it captures a fresh commit frontier and a private lease
ticket bound to the installation allocation, actual term, configuration,
incarnation and revocation generation. It then captures the successful unified
applied position and an owned positioned snapshot from its own state machine's
`WalEngine`. No caller supplies a snapshot, engine or substitute applied number.
The index's data and command position are copied under one index-state lock;
this operation neither takes the WAL mutex nor performs storage I/O.

After releasing the state-machine and applied-position locks, the driver
re-enters the peer gate, samples the clock and validates the same ticket against
the retained view's prefix. It finally checks the original absolute deadline
and stop flag. `pump_gate` remains held throughout this local sequence. Any
refusal discards that view. Expired authority plus unavailable quorum produces
a typed refusal or timeout, never successful local data.

Success returns a non-Clone `LeaseReadView`, not a `ReadBarrier`. The owned view
is consumed by the server; its diagnostic index cannot construct another
authorized view. Metadata, keyspace, range and both region-epoch halves are
checked on this same view before values are returned. If lifecycle contention
or the existing batch-copy budget defers execution, the job retains that view.
A later epoch change or lease expiration does not authorize taking a new one.
Fresh RPCs always establish their own authority; no authorized view is cached
across requests. A response may finish after lease expiry if its original view
was authorized within that request, as proved in the existing history argument.

## Exact-prefix refinement argument

The [lease proof](LEADER-LEASE-PROOF.md) and
[transition model](LEASE-AUTHORITY-MODEL.md) require an exact committed view
through `j`, covering the per-invocation frontier `c`. The engine's last command
position `q` is not by itself `j`: Raft no-ops and configuration records advance
the driver's unified applied watermark without changing the data engine.

Let `F(k)` denote the logical metadata and RawKV state after the ordered committed
log prefix through `k`. Between successful owner turns maintain:

1. Every log item through the published driver index `j` completed application;
   the driver publishes no successful tail beyond an error.
2. `q` is the last command position through `j`; the state machine's applied
   command index and the engine's atomically stored command position agree.
3. The engine's logical contents equal `F(j)`. Entries after `q` through `j`
   are non-data entries, so they do not require a fabricated engine command.

The serving base case follows committed-log recovery and the current-term
bootstrap/application fence. In particular, startup verifies the recovered
engine position's term against committed Raft history; an unpublished/replaying
driver is not a serving authority. For the inductive step, a command applies its
data and position together and advances the state-machine command index. A
no-op preserves both. Configuration application does not modify RawKV data;
the experimental voter installation separately refuses membership changes.
After all items in a turn succeed, publication advances `j` to that turn's tail.
An error takes the fatal path instead. Thus all three invariants are preserved
at successful owner boundaries.

The read holds the same owner gate used by application, so no partial next turn
can interleave with capture. Its positioned engine view satisfies
`q == state_machine.applied_index` and `q <= j`, and retains the immutable data
version captured with `q`. By the invariants this view is exactly `F(j)`, even
when `q < j` because of no-ops. Final controller validation requires `j >= c`
and rejects an index beyond current commitment. The pre-existing current-term,
quorum-lease containment and final authority lemmas therefore apply to this
exact view. Reads of metadata and values cannot select different `F` versions.

This is a concrete refinement argument under the ordered serving-engine writer
invariant, not a machine-checked proof of all Rust, raft-rs, clocks and storage.
The driver owns the state machine privately; serving RawKV and catalog mutations
propose commands through it. The alternate `MetaRaft::propose_apply` drain is
test-only and refuses an externally driven peer. Recovery/layout migration
occurs before serving. Generic engine APIs still permit embedded callers to
write directly: such callers cannot claim this server's refinement merely by
constructing a driver. New direct writers, snapshot installation, membership
changes or storage read tiers must re-establish the invariants above.

## Constructor and call-site inventory

Re-run and classify every hit, including tests and declarations:

```sh
rg -n -F 'ReadBarrier {' crates/raft/src
rg -n -F 'LeaseReadView {' crates
rg -n 'try_positioned_resident_snapshot|finish_lease_(get|batch_get)|ReadOnlyOption::' crates/raft/src crates/server/src crates/engine/src
```

There are three production `ReadBarrier` constructions: the synchronous Safe
path, the original asynchronous Safe path, and the new reservation's successful
Safe fallback. None derives an index from a lease. There is one production
`LeaseReadView` construction, after the driver's final validation. The driver
has one positioned-snapshot acquisition from its own state machine; the engine
has the concrete WalEngine-to-MemEngine forwarding implementation. `Engine`,
`ReadView` and `ApplyStore` traits were not widened. The server has one GET and
one BatchGet dispatch into their lease finishers. The original server snapshot
constructors remain for Safe reads; the lease finishers do not call them.

## Local validation and remaining gates

[Retained source evidence](lease-read-view-v1/README.md) records 136 Engine,
243 Raft and 207 server tests passing: **586 passed, zero failed**, with one
pre-existing server workload test ignored because it requires an isolated
output path and independent history verification. Nineteen tests are new:
two engine, twelve Raft/admission and five server tests. Default recovery,
default/experimental server compilation, formatting and warnings-denied
all-target Clippy for the three crates pass.

Three actual runtimes with disk logs and gRPC peer transport exercise both
unary and streaming public GET/BatchGet. The tests observe a real lease hit
without an additional quorum read, preserve ordered duplicates/empty/missing
items, retain views across both epoch changes and deferred large copies, and
check typed RPC refusal after lease expiry when follower owners stop. Their
clock is deliberately controlled logical time; this is neither host-clock
qualification nor Chaos Mesh injection. An in-process driver test separately
counts unchanged transport sends during a successful local read.

Nine separately compiled single-defect source controls fail at their declared
assertions: missing final authority validation, stale commit capture, missing
apply coverage, foreign installation acceptance, missing admission bound,
reset fallback deadline, replacement of deferred GET or BatchGet views, and
replacement of a large batch's view after authorization. The eight distinct
targets pass on baseline and restored source. Forty command outcomes and
source/binary identities are retained. These are controls for the specified
protocol and ownership obligations; they are not nine independently checked
linearizability counterexamples. Test populations overlap the full suites.

Original failures remain recorded: an optional `.cargo` directory assumption
before compilation, two missing test imports, a stream-client deadline type
mismatch, and a fixture using a principal as the bearer token. The authentication
failures did not exercise the lease read path and are not safety evidence.
No production assertion, deadline, storage floor or command cap was relaxed.

Next qualify a concrete clock contract through supported process/host pauses,
then run actual lease-enabled Chaos Mesh partition, delay, pause and restart
histories, including asymmetric and application-message ordering cases. Obtain
matched c1/c64 GET, mixed and batch throughput, mean and p99 only after those
gates. The selected `11113f6` runtime and its historical performance remain the
baseline. Renewals still require a quorum; longer voting promises can delay
failure recovery. No original industrial checklist item closes here.
