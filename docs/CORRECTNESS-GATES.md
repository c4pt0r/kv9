# Mandatory correctness and availability gates

Updated: 2026-09-08. These requirements supplement every stage of roadmap issue
[#9](https://github.com/c4pt0r/kv9/issues/9). They do not replace any existing deliverable.

## Proof obligations

Every core protocol change must define its state, transitions, failure assumptions,
invariants, safety theorem and conditional liveness argument before acceptance.
Use TLA+ as the primary protocol specification and model-check finite instances
with TLC, including counterexample controls and conditional temporal properties.
Provide a rigorous proof with explicit lemmas and induction over transitions;
machine-check deductive protocol proofs with TLAPS or the pinned Lean project
under `proofs/lean`. Retain existing Lean lemmas with their explicit scope.
Finite model checking, property tests and E2E histories are complementary evidence,
not substitutes for an unbounded proof. TLA+ supports deductive proofs through
TLAPS; it is not limited to finite model checking.

Each proof must identify the implementation operations it represents and the
refinement assumptions not yet verified. A proof about an abstract model is not a
claim that the Rust implementation has been mechanically verified. A missing
refinement argument remains an open obligation. No `sorry`, `admit`, custom axiom
or disabled checker may discharge an obligation.

| Protocol | Required obligations | Implementation boundary |
|---|---|---|
| Raft integration | Quorum intersection; durable single vote per term; log matching and leader completeness; persist before send; applied prefix and exact receipt correlation | `raft/rawnode`, `storage`, `driver`; separate upstream Raft assumptions from kv9 integration |
| Metadata | Statement atomicity, PK/unique/FK preservation, allocation uniqueness, bootstrap uniqueness, stale-planning rejection | `meta/store`, `bootstrap`, `server/runtime` |
| WAL/checkpoint | Atomic data/position recovery, successful-sync durability, directory publication, cut coverage and safe reclamation | `engine/wal`, `persist`, `flush_journal` |
| Manifest/GC | Canonical identity, generation/epoch CAS, sound positive and negative settlement, retained evidence, no deletion of reachable state | `region/manifest`, `raft/state_machine`, future GC |
| Group ownership | Single writable owner, complete range coverage, safe membership, recoverable split/merge/migration | RegionManager, catalog routing, Raft configuration |
| Transactions | TSO uniqueness and fencing, commit authority, SI visibility, lock recovery and safe MVCC retention | `meta/tso`, `txn`, MVCC engine |
| Admission/scheduling | Bounded accounting and conditional progress; no loss of consensus/control progress under overload | Queue, cache and scheduler implementations |

Track theorem names, source revisions, model scope, counterexamples and remaining
refinement obligations with each issue. A green theorem checker does not close a
broader issue whose implementation or failure coverage is incomplete.

The current TLA+ inventory and reproducible runner are under `proofs/tla`.
`docs/METADATA-PLANNING.md` records the first metadata protocol model, its written
induction argument, implementation mapping and the checked TLAPS prefix, receipt,
planning freshness and catalog uniqueness proofs under `proofs/tlaps`. Full type
bounds, conditional draining and implementation refinement remain open. A green
TLC run does not discharge those obligations.

`docs/READY-PUBLICATION.md` records the separate per-peer Ready model and
deductive proof of durable commit coverage, failed-cycle publication exclusion
and fail-stop action safety. `scripts/check-ready-tla.py` checks finite crash
schedules, three isolated faults and two reachability witnesses. The combined
TLAPS inventory has 47 theorem declarations and 534 obligations. These results
do not establish upstream consensus, term/vote identity or composition with Rust.

The ReadIndex admission protocol is recorded in [READ-ADMISSION.md](READ-ADMISSION.md).
It checks the leader/current-term admission handshake, preserved request context
and budget, exact receipt/application prerequisites and conditional progress. Its
upstream Raft and refinement assumptions remain explicit; the separate gRPC apply
timeout under #48 is not closed by this protocol proof.

[ENDPOINT-WRITERS.md](ENDPOINT-WRITERS.md) records the shared endpoint generation,
atomic admission revocation and serialized route-installation extension. Its
parameterized safety and conditional-convergence proof covers existing writers;
public migration, changed-endpoint restart and rollout fencing remain open in #47.

## Chaos Mesh E2E is mandatory

Run actual Chaos Mesh resources against dedicated kv9 workloads in an explicitly
selected isolated Kubernetes cluster. Scope every selector to the run namespace
and exact workload labels. Never use an ambient kubectl context for injection.

Required families include PodChaos kill/failure, NetworkChaos partition, delay,
loss, duplication/reordering where supported, and IOChaos errors/delay. Combine
them with bootstrap, membership, checkpoint/pending recovery and later
split/merge/transactions. Document backend-specific injection limitations.

An `AllInjected` condition is insufficient on its own. Retain positive target and
effect observations, operation histories, selected victim identities, timestamps,
Kubernetes/Chaos state, process logs and recovery results. Keep success artifacts
as well as failures. Require application-level history correctness and progress
after healing; transport failure alone is not an acceptable read-refusal verdict.

Process termination does not model power loss. The deterministic persistence
model must separately distinguish visible writes, file durability and directory
entry durability, including partial unsynced persistence and EIO/ENOSPC.

## No single point of failure except the object-store dependency

No database node, metadata leader, TSO provider, scheduler, transaction coordinator,
router, discovery seed or client endpoint may be indispensable. Service-critical
state must be replicated or reconstructible from a surviving quorum and retained
objects. Singleton active roles need an explicit takeover protocol and durable
authority; multiple processes alone do not prove availability.

For a group of `2f+1` voters, the availability claim is conditional on at most `f`
unavailable voters, a communicating majority, eventual message delivery and
sufficient storage capacity. This is not a promise of writes on both sides of a
partition. Safety must hold when those liveness assumptions fail.

Test the failure of every voter in turn, including whichever node currently owns
each singleton role. Clients must continue using remaining endpoints without
requiring the failed seed or a single gateway. Later cross-machine acceptance must
place replicas in separate failure domains and test actual host loss.

The existing local Kind cluster has one Kubernetes node. Pod-level tests there
prove application-replica behavior only; they do not establish host-failure
isolation or production Kubernetes-control-plane availability. Production
orchestration, DNS, credentials and client routing must not introduce hidden
service-critical singletons. Object storage is the sole allowed external storage
availability dependency; its assumed guarantees remain explicit.

## Acceptance evidence

Every relevant issue links its proof/refinement record, actual Chaos Mesh run and
single-failure-domain analysis. These are additional hard gates, alongside the
existing unit, mutation, history, recovery and performance checks. Missing or
inconclusive evidence keeps the corresponding obligation open.

The registration routing increment and its checked cyclic-coverage lemmas are
specified in [REGISTRATION-ROUTING.md](REGISTRATION-ROUTING.md). Its actual Chaos
Mesh window requires a joining learner to complete exact catch-up while its first
seed remains partitioned and the surviving quorum serves an independent history.

[PUBLIC-ADMISSION.md](PUBLIC-ADMISSION.md) defines the public backend reservation
ledger, its local inductive count/byte proof, source controls and overload window
under an actual leader partition. Its budget excludes transport/decode, response
materialization and whole-process memory; broader resource obligations stay open.
