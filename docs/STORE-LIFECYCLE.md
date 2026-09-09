# Durable store lifecycle

Issue [#42](https://github.com/c4pt0r/kv9/issues/42) requires initial root voters
to distinguish an original disk from a replacement disk carrying an old root
descriptor. Production provisioning now prepares each actual data directory
independently, and the runtime enforces the lifecycle before starting its Raft
owner or opening receive authority. The CLI requires `root-create
--store-incarnations` to name those prepared stores. Initial formation recovery
and its separate proof are documented in [ROOT-FORMATION.md](ROOT-FORMATION.md).
All three initial voters passed actual missing-log and independently prepared
replacement-PVC Chaos cells at `371163b`; complete histories and retained
provenance are recorded in [PERSISTENT-CHAOS.md](PERSISTENT-CHAOS.md). The later
21-window matrix includes accepted production endpoint migration under #47.
[REPLACEMENT-ACCEPTANCE.md](REPLACEMENT-ACCEPTANCE.md) reconciles all five original
#42 acceptance criteria and the limits retained by the component proofs.

## Contract

`StoreGuard::lock` obtains an exclusive operating-system file lock on the data
directory. The caller must retain the guard until every user of that store has
stopped. A second cooperating process cannot own the same directory. The lock
is local; it introduces no runtime coordinator or external availability dependency.
Unrelated processes replacing or deleting locked files are outside the ownership
contract. Filesystems must support the requested locking and sync operations.

`prepare` independently mints a nonzero random incarnation on the actual disk.
The required rollout creates a root descriptor from the returned public
incarnations, rather than allowing that descriptor to mint identities on empty
disks. The lifecycle moves from Prepared to Bound(root digest) to Active(root
digest). Node ID, incarnation, and an existing root binding cannot change.
Checksums reject malformed records; they are not cryptographic authentication.

Publication writes a new temporary file, synchronizes the file, renames it over
the lifecycle record, then synchronizes the directory and all canonical
ancestors. Only successful completion updates the guard's cached authority. Any
publication error poisons the guard: verification and further publications fail
until it is closed and reopened. Reopening decodes and synchronizes the visible
record and namespace before returning usable authority.

A complete unpublished Prepared temporary record may survive a process crash.
Preparation preserves that file and mints a different incarnation; it never
imports the orphan's identity. Arbitrary data, malformed temporary records,
records for another node, and orphan Bound/Active records do not authorize a
new preparation. Unrecoverable preparation debris therefore fails explicitly.

The runtime makes the initial Raft log and namespace durable
before calling `activate`, and makes activation durable before any Raft owner starts
or receive gate opens. Every Active restart calls `DiskRaftStorage::recover`,
which opens an existing file without creating directories or files. Missing,
empty, or wholly torn logs are rejected before tail repair or fresh ConfState
creation. A valid recoverable prefix still receives the existing durability
and consistency checks; recovery never reconstructs an acknowledged vote from
an identity descriptor.

Legacy migration is a separate caller obligation. `adopt_certified_recovery`
requires the caller to recover the existing log, validate the applied Raft
position and local catalog identity, and check the exact committed root
certificate first. A copied root/identity bundle alone is insufficient. The
primitive cannot itself establish that catalog certificate. Runtime tests exercise these ordering and migration obligations directly.
`NodeRuntime` retains the guard until its Raft owner has joined and its gRPC
runtime and store-owning fields have been dropped.

## Model and proof

[StoreLifecycle.tla](../proofs/tla/store/StoreLifecycle.tla) models one initial
root voter, its independently prepared directory, immutable root binding,
visible and durable lifecycle/log states, publication errors, process crashes,
whole-directory loss, log loss, and certificate-based legacy recovery.
`slUsed`, `slEver`, and `slRetired` are history variables for specification only;
startup does not consult a global identity registry.

| Model action | Required implementation boundary |
|---|---|
| `SLPrepare`, `SLRootBind` | Successful local preparation; root descriptor constructed from that exact returned incarnation |
| `SLWriteBinding`, `SLPublish` | Binding record write/rename followed by namespace publication |
| `SLCreateLog`, `SLSyncLog` | First Raft log creation only before activation; file and namespace durable before authorization |
| `SLWriteActivation`, `SLPublish`, `SLStart` | Durable Active record before runtime owner or receive authority starts |
| `SLFail`, `SLCrash`, `SLRestart` | Poison on publication error; discard volatile authority; stabilize retained visible state on reopen |
| `SLLoseLog`, `SLWholeLoss` | Recovery-only refusal or a newly prepared incarnation that cannot match the old root |
| `SLLoseLifecycle`, `SLLegacyWrite` | Migration from recovered matching log and committed catalog certificate |

[StoreLifecycleProof.tla](../proofs/tlaps/store/StoreLifecycleProof.tla) contains
14 declarations and 138 obligations. Induction establishes immutable root
binding, owner authorization, no recreation of a retired incarnation's log,
and activation only after durable log recovery and the required binding or
certificate. These safety results permit arbitrary modeled crashes and errors.

Conditional progress starts from any invariant state with a healthy matching
store and durable log. A continuation without further failures, with weak
fairness of activation write, publication, and owner start, eventually starts
the owner. This proves local authorization progress; it does not prove catalog
formation, quorum availability, replication progress, or progress under
unlimited failures.

The start-progress proof separates an executable `SLStart` witness from its
effect on the state frame. With `slOwner = FALSE`, starting sets it to `TRUE`,
so `SLStart` is equivalent to its nonstuttering action. TLAPS's built-in
`ENABLEDaxioms` equivalence rule lifts that proved action equivalence to
enabledness. This decomposition avoids asking SMT to discover both a
13-variable successor and a changed tuple component in one obligation. The
model, fairness assumptions, pinned tools, and default solver timeouts are
unchanged; the strict inventory includes every added proof obligation.

Incarnation freshness is an explicit abstraction of successful random ID
allocation, not a proof that finite random identifiers cannot collide. The
model assumes correct recovered log identity/position, committed certificate
validation, and honest successful filesystem synchronization. It does not
prove Raft, Rust compilation, operating-system locking, hardware durability,
or a complete source refinement. The mapping above identifies caller
obligations that runtime tests and Chaos Mesh must cover before integration
acceptance. The pinned backend and standard-library trust boundary is described
in the [TLAPS inventory documentation](../proofs/tlaps/README.md).

## Reproduction and fault controls

The [persistent Chaos gate](PERSISTENT-CHAOS.md) includes an initial-voter
missing-log matrix: actual PodChaos owner death, two refused starts per voter,
majority service, and recovery of the original retained log. The fixture and
independent evidence checker keep process death, file loss and restored
identity observable as separate boundaries. A separate three-voter PVC matrix
prepares new disks, copies only the old root/store identity bundle, verifies
typed initialization and actual startup refusal, then recovers each original
PVC. It checks the independently prepared incarnation rather than relying on
a missing listener or stale status alone.

```sh
python3 scripts/check-store-protocol.py --tlapm /path/to/tlapm/bin/tlapm \
  --jar /path/to/tla2tools.jar --output /tmp/kv9-store-protocol
python3 scripts/check-store-controls.py --output /tmp/kv9-store-controls
cargo test --locked -p kv9-common --lib store_lifecycle::tests
cargo test --locked -p kv9-raft --lib recovery_only
cargo test --locked -p kv9-raft --lib activated_store_recovery
```

The protocol runner uses strict fresh-cache TLAPS proofs, SANY dependency and
assumption auditing, two bounded incarnation sets with two TLC fingerprints,
a fair continuation, and two reachable witnesses. Eight isolated protocol
faults cover reused identities, Active log recreation, owner start before
publication, activation before log sync, legacy recovery without a certificate
or log, owner start after a publication error, and missing owner fairness.
Every fault must produce its intended TLC violation and failed proof obligation
between passing original/restored checks. Missing proofs, extra axioms, and
incomplete model/proof output are independently rejected.

Nine compiled source controls cover identity verification, exclusive ownership,
missing/empty log recreation, continued authorization after a publication
error, owner start before activation, the non-pristine formation fence, and
planning before the current-term application barrier, and detached owners
after a failed listener bind. Owner creation follows all fallible startup
work so an error cannot release the guard while a detached thread uses the store. Common tests inject both EIO and ENOSPC before and after each publication
operation, and exercise a process interruption after temporary-file sync.
These host-filesystem tests check ordering and failure propagation; they are
not simulated power-loss evidence. The separate deterministic filesystem model
runs real Raft vote/recovery code through recovery I/O failures and repeated
power loss, preserving the one-vote-per-term property. Its model limitations
remain those documented in `crates/common/src/fs/testing.rs`.
