# Durable lease installation and actual Raft voting gates

Tracking: [#9](https://github.com/c4pt0r/kv9/issues/9), #20.

This report records checkpoint `4c340cb`. The subsequent
[renewal/publication adapter](LEASE-RENEWAL-PUBLICATION.md) implements the next
leader-lifetime and grant/pump stage; service read views remain open.

The experimental adapter now installs the [lease voter component](LEASE-CONTROLLER.md)
inside `RaftPeer` and gates actual raft-rs election entry points. The ordinary
constructor still selects Safe ReadIndex. There is no renewal-message codec,
leader certificate publication, server lease configuration or lease read path
at this checkpoint. This stage does not provide a new performance or actual Chaos Mesh result.

## Durable installation and restart

`RaftPeer::with_lease_storage` consumes the storage and installs one private
`InstalledLease` during peer construction. There is no replacement or removal
method. The constructor validates the actual local node, group and fixed Raft
voter set, constructs the already checked timing bounds, synchronizes a fresh
epoch, then samples the clock and starts the complete recovery quarantine.
A failed clock sample after synchronization leaves the durable opt-in intact.

Raft-log record kind 5 carries an exact version, monotonically increasing
incarnation and immutable policy. The policy includes node, group, configuration
identity, canonical voters, promise duration, drift allowance and sampling margin.
The first incarnation is 1; each subsequent record must be its predecessor plus
one with the identical policy. Overflow, changed policies, malformed records and
non-sequential replay are typed refusals. The maximum body is 575 bytes including
the record-kind byte (64 voters); existing framing, checksum and synchronization
rules apply. No volatile storage implementation opts in by default.

The writer publishes the in-memory epoch only after record synchronization. A
failed writer is terminal, including a failure reported after synchronization.
Recovery can retain an unacknowledged complete record, but must synchronize its
replayed prefix before returning. The next installation advances from that
recovered epoch. A torn, uninstalled epoch can be discarded without reusing an
acknowledged installation. This argument assumes the existing exclusive store
ownership and crash/recovery model; cloning or rolling back an active voter log
is not a supported way to create another voting process.

The marker is decoded even without `experimental-leader-lease`. The ordinary
`with_storage` constructor refuses a marked store. Binaries predating kind 5
already reject unknown checksum-valid record kinds. Thus disabling the feature
or downgrading cannot silently restore immediate voting on this directory.
There is no policy migration or opt-out protocol yet; these are explicit
compatibility limits of the experimental constructor.

The persisted epoch is diagnostic installation identity, not read authority.
Binding it to a single leader controller for each term, renewal sequence and
non-transferable read ticket remains part of the next adapter stage.

## Election-entry coverage

All decisions below share the mutex that owns `RawNode`. The clock is sampled
after taking that mutex. `VoterLease::may_vote` receives the actual local term,
and the adapter verifies the actual fixed configuration. Higher-term replication
is still processed; it does not clear a held promise. Clock sampling errors,
regression and persistence failures permanently fence this voter incarnation.

| Actual entry point | Bound behavior |
| --- | --- |
| Explicit `campaign` | Check the voter before `RawNode::campaign`. |
| Follower/candidate tick | Check before raft-rs can call `tick_election -> hup -> campaign`. |
| Leader tick | Continue heartbeat/check-quorum ticks; these can step down but cannot cast a self-vote in that tick. |
| RequestVote, including forced-transfer context | Drop before `RawNode::step` while quarantined, held or fenced. |
| PreVote response | Check before `poll(Won)` can internally call `campaign(ELECTION)` and self-vote. |
| Vote response | Check before completing an election after a clock/configuration fence. |
| TimeoutNow / TransferLeader / local Hup | Gate the forced-election/transfer route; the testing transfer helper uses the same gate. RawNode also rejects externally injected local messages. |
| PreVote request | Ordinary Raft processing is retained; a pre-vote is not an actual vote. The later actual-vote and campaign transitions are gated. |

The source audit is against pinned raft-rs 0.7.0. It accounts for direct campaign,
tick-driven `hup`, follower `MsgTimeoutNow`, and pre-candidate `poll(Won)` as
campaign callers. Ordinary vote eligibility, term/vote durability, log matching,
majority commitment and Safe ReadIndex processing remain raft-rs responsibilities.
Outgoing votes still pass through the original Ready and LightReady persistence
sequence; the adapter does not fabricate a vote response.

This binding implements the voting part of the [proved authority protocol](LEASE-AUTHORITY-MODEL.md):
while a voter is quarantined or held, no covered transition can cast a competing
vote. Otherwise, it delegates to ordinary Raft eligibility. A prior higher-term
vote cannot be undone by a lower-term grant because the forthcoming grant adapter
must read the local monotonic term under this same lock. Quorum intersection and
the already proved conservative timing containment then exclude a replacement
leader during a usable certificate. That final conclusion still depends on the
grant/publication and read bindings, which this stage does not yet implement.

## Configuration and fault boundaries

The proof remains fixed-configuration. Experimental peers refuse new membership
proposals, fence on a new membership apply, and drop snapshots before raft-rs can
restore a different configuration. Already recovered configuration entries still
use the existing replay guard. A joint configuration cannot be installed. These
restrictions are confined to explicitly installed experimental peers; ordinary
peers retain their existing membership behavior. Recoverable membership changes
and snapshot installation require a separate lease migration/revocation protocol.

The private test grant seam calls the actual component while holding the peer
mutex; it is not a network grant path. Tests exercise real raft-rs elections and
real disk storage, plus deterministic filesystem crash schedules. They do not
constitute a full Rust refinement proof, independent-host failure test or actual
Chaos Mesh acceptance. A user-supplied `LeaseClock` implementation remains an
explicit trusted premise; detecting a backwards sample cannot detect every
clock-rate violation or stopped clock.

## Local validation

[Retained evidence](lease-vote-binding-v1/README.md) records 219 passing Raft
library tests (14 new, including 10 peer-binding tests), zero failed or ignored,
formatting, experimental-feature compilation and warnings-denied all-target
Clippy. A separate default-feature integration test passes against the normal
library build, proving that compiling without leases still refuses a marked
voter's ordinary reopen. The 180 filesystem fault combinations use before/after
and short-write faults, I/O/full-disk errors and 18 crash schedules per cut.

Eight compiled faulty-source variants fail their exact intended tests: implicit
PreVote self-vote, tick election, explicit campaign, forced vote, disabling the
mode on restart, epoch publication without synchronization, changed policy and
snapshot membership restoration. The six distinct target tests pass on both
baseline and restored source; this population overlaps the 219 library tests.
The runner copies actual workspace sources into an isolated capsule and uses
the shared retained-build lock. No mutation touches the working checkout.

The first capsule omitted `proto/` and the shared-target environment, so its
build failed before running any semantic control. The original script and
failure remain retained; the repaired capsule includes the protobuf source and
explicit offline/shared-target settings. Its eight controls and feature-disabled
probe all complete within the unchanged resource limits. Inactive retained test
executables were cold-converted with exact readback, and reproducible compiler
caches were explicitly invalidated, to satisfy the original 80 GiB free floor
plus 16 GiB source reservation. No storage cap or timeout was increased.

## Next integration

Bind each leader lifetime to the persisted installation identity; add exact,
versioned renewal and grant envelopes, actual local term/leader validation,
durable ACK publication and certificate activation only after the whole owning
pump succeeds. Then bind each read's fresh commit frontier and exact applied
immutable view, final authority/clock check and original request budget. Qualify
the actual clock platform and run actual partition/delay/pause/restart Chaos Mesh
histories for that candidate before comparing throughput and latency.
