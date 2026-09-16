# D01a: shared transport with isolated Raft group inboxes

This first [#22](https://github.com/c4pt0r/kv9/issues/22) increment removes the
single-group assumption at the gRPC transport boundary. It does not expose
public data-group creation, range routing, split or horizontal scaling yet.
See the [development and benchmark plan](HORIZONTAL-SCALING-PLAN.md).

This document records the initial `3dc6a13` transport checkpoint. The subsequent
[RPC fencing increment](GROUP-WIRE-FENCING.md) separates metadata from data
streams, reserves metadata queue capacity and makes the old-receiver boundary
safe on every connection. The one-stream description and compatibility gap
below describe the historical first increment.

## Implementation

`GrpcTransport` owns the node's existing peer routes and one outbound worker
per peer. Explicit `register_group` creates an independent bounded inbox and
`GroupTransport` handle; it creates no listener, connection, thread, engine or
catalog row. The handle stamps outgoing envelopes with its immutable region
ID. `NodeDriver::new` checks the scoped transport's group and exclusive driver
binding before returning a driver. Each group's Ready pump drains only its
own inbox and is notified by its own work signal.

The production server installs `inbound_router()` on its existing service.
Root identity, local receive authority, authenticated sender and destination
checks precede admission. Unknown or saturated groups drop individual messages
without closing the shared stream; later messages for healthy groups remain
eligible. Drops are observable through `RaftGrpcService::dropped`. Raft owns
retransmission and quorum acknowledgements; transport admission is never a
commit receipt. The payload destination must match the envelope destination.

V2 metadata retains wire ID **0**, despite catalog `META_REGION_0` being **1**.
Both identifiers are reserved from data-group registration. Metadata admission
uses its existing queue directly without the data-group registry lock. Data
groups use catalog IDs greater than 1. Legacy single-inbox service fixtures now
refuse nonzero group envelopes rather than misrouting them into metadata.

Registration is append-only and capped at 256 local groups including metadata.
Each inbox retains the existing 4,096-message / 64-MiB encoded-weight limit and
256-message / 1-MiB drain target, with the existing one-large-message exception.
This is a finite infrastructure bound, not acceptance of node-wide memory
budgets, worker fairness or scheduler overhead. Queue weight is not allocator
RSS. Outbound queues are still shared per peer; hot-group outbound fairness and
reserved metadata capacity remain D01b work.

## Safety argument and implementation boundary

This changes routing infrastructure, not Raft voting, commitment, persistence,
apply ordering or read authorization. Its local isolation invariant is:

1. Each admitted wire group has exactly one local queue. Metadata owns its
   immutable queue; registration inserts a fresh queue under an unused ID
   while holding the registry mutex. Reserved, duplicate and over-capacity
   registrations leave the registry unchanged. No wire event can insert a row.
2. Delivery either appends to the exact selected group's queue or drops. There
   is no default-to-metadata branch. The registry is append-only, so releasing
   its lock after cloning a queue cannot race with replacement or retirement.
3. A successful scoped driver binding requires equality with the immutable
   region ID and a successful `false → true` atomic compare/exchange. Thus two
   successful bindings cannot consume the same data-group handle. A rejected
   construction returns no driver; its peer's minted drain token is not reused.
4. Sending uses the handle's immutable group ID and the shared authenticated
   node transport. Draining uses the same handle's queue. By induction over
   registration, send, delivery and drain, this infrastructure cannot transfer
   a well-formed sender group's message to a different registered group.
5. Loss at unknown/full queues changes delivery, not Raft's safety rules.
   Progress still assumes eventual delivery and scheduling. The test that
   pauses one group is a finite progress observation, not an unbounded fairness
   or failure-domain proof.

These arguments assume trusted server construction and the existing transport
authentication boundary. They are not a proof of a complete RegionManager,
Byzantine behavior or arbitrary misuse of the low-level public transport trait.
Transport registration is not durable creation authority. Group retirement,
ID reuse/tombstones, replica-incarnation/epoch fencing, durable activation,
bounded worker scheduling and per-group storage recovery are not implemented
here. Envelope epoch fields remain zero and do not authorize public ownership.
Core lifecycle/split protocols still require machine-checked proofs and an
explicit refinement record before their acceptance.

Old released V2 servers ignored the group field. Therefore a future manager
must establish peer capability/version compatibility before enabling data
groups; this increment must not be interpreted as safe mixed-version data-group
activation. The production runtime still starts only metadata.

## Local validation

Five focused tests cover reserved/duplicate/capacity registration, legacy
nonmetadata refusal, wrong-group/duplicate driver binding, mixed-batch admission
behind a saturated/unknown group, and three distinct three-voter Raft groups
replicating over shared real loopback gRPC streams. Distinct leaders commit
different values under the same key; one paused group's proposal remains
unapplied while the other groups commit, then catches up when resumed.

These fixtures use independent in-memory stores. They do not exercise durable
restart, production scheduling, a public multi-range API, physical host loss,
actual Chaos Mesh or a scaling benchmark. No such acceptance is claimed.

Retained local logs and single-defect control sources use
`/mnt/data/kv9-work/scaleout-start-20260916`. The initial compile exposed a
missing trait qualification and was corrected; its failed log is retained.
The [published validation receipt](multi-raft-transport-v1/README.md) records
856 passing workspace tests/doctests (28 existing ignored), five passing
single-defect controls, formatting, strict Clippy and exact source hashes. GitHub
CI remains manual and is not triggered for this increment.
