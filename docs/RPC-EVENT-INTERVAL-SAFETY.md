# RPC event interval: conditional safety correspondence

This argument covers only the change from `629bee4fd9dcca02529703a06eefe9250fd5d1ac`
to `917243fd1b501843d75899cc167f7eff32b5bbb2`. It does not close the outstanding
machine-checked composition and executable-refinement obligations of the base.

The exact diff contains one executable change in `crates/server/src/runtime.rs`:
construct the same multi-threaded Tokio runtime with all drivers enabled, but
set `event_interval(8)`. Pinned Tokio 1.53.1 implements `Runtime::new()` as
`Builder::new_multi_thread().enable_all().build()`. The other changed file is
the experiment contract. The Raft and engine source trees, dependency lockfile,
worker selection, transport, handlers and protocol configurations are unchanged.

## Assumptions and argument

Let `S` be the application's protocol state, including pending requests, sealed
read groups, Raft state, applied indexes, admission reservations and responses.
Let `Next` include every application transition enabled by protocol guards,
including timer expiry, cancellation, refusal and failure transitions. These
guards must be safe for arbitrary task interleavings and admissible fault/input
histories; safety cannot depend on a particular polling interval.

Assume that the base's initial states satisfy invariant `I`, every `Next` step
preserves `I`, and the unchanged Rust/Tokio synchronization and I/O operations
implement those transitions. Runtime maintenance that changes no protocol state
projects to a stuttering step. A maintenance action delivering I/O or a timer
allows the corresponding guarded application transition; it does not itself
authorize a successful read or write. These are explicit source-refinement
premises, not a verified Tokio implementation or a whole-database theorem.

The candidate changes the choice and timing of enabled execution, while
preserving the application transition definitions and guards. For every finite
candidate trace, induction establishes `I`: the initial state satisfies `I`;
a stuttering step preserves it by identity; any application step preserves it
by the `Next` premise. Thus this scheduling delta preserves safety conditional
on the base's safety and source-refinement premises. It does not assert that
the base and candidate produce identical timed traces or failure outcomes.

In particular, successful asynchronous read preparations on the sealed-group
path still require sealed membership before their fresh ReadIndex, the exact
group confirmation, a successful whole pump, sufficient applied coverage and
each request's existing view/epoch checks. The separate synchronous barrier
path retains its own exact-context confirmation and applied-index conditions.
Writes retain their committed/applied acknowledgement conditions. Earlier I/O
service supplies no new quorum, persistence, ownership or response authority.

The historical [Raft scheduling proof](RAFT-SCHEDULING-PROOF.md) and
[sealed read-group contract](READ-GROUPS.md) retain their original revision
boundaries and assumptions. This argument does not transfer all of their claims
across intervening changes or label their open composition gates complete.

## Progress and validation limits

No wall-clock or unconditional progress bound follows. Busy-worker maintenance
includes I/O/timers, statistics, deferred wakeups and shutdown checks. Contention
for the shared driver can prevent a poll; idle workers also poll independently.
The interval is neither a network RTT guarantee nor an election deadline bound.
Existing liveness arguments still require their fairness, live-quorum, finite
callback and eventual-apply premises.

Focused tests and actual frozen-binary process/fault histories exercise the
changed production constructor. They provide execution evidence, not a proof
of all interleavings. Performance and exact fault-acceptance results are recorded
separately in [the measured results](RPC-EVENT-INTERVAL-PERFORMANCE.md).

Source correspondence record: `/tmp/kv9-rpc-event-source-correspondence-first/result.json`.
The unchanged Raft tree object is `efed0cc604d5c360342d769d6b59d454f7ef8c14`;
the unchanged engine tree object is `dbd76e67b5e8ef34107145c80472b8ebdcfc7059`.
The candidate runtime file SHA-256 is
`69d2ad5470eeee5062819700c4c5a3b45d922961febab93735015ddccd92d8ee`.
