# Resuming the first catalog after an original-store restart

Issue [#44](https://github.com/c4pt0r/kv9/issues/44) exposed a permanent startup
stall: persisting the initial Raft ConfState made a store non-pristine, and
startup treated that fact as proof that catalog creation had finished. If all
original voters restarted before creation, none could finish discovery even
with every original disk and a healthy quorum.

Startup now distinguishes durable Raft history from an applied catalog. A
matching original store may resume formation; an Active store must recover its
existing log and pass the [store lifecycle](STORE-LIFECYCLE.md) checks first.
An initialized marker or recovered catalog still prevents a fresh bootstrap.
A marker lost after commit is reconstructed from the certified catalog.

## Formation protocol

1. The root descriptor fixes the cluster, generation, original voter addresses,
   and independently prepared store incarnations before any Raft owner starts.
2. Discovery and Raft elect an available original voter. No designated voter or
   provisioning process is required to remain available.
3. Before planning the initial catalog, the leader waits until its applied
   watermark reaches its current term. Applying the election no-op drains all
   earlier retained committed entries, including a prior initialization.
4. Under the catalog planner mutex, the leader confirms its role and term and
   reads the catalog again. An existing certified catalog is adopted.
5. An empty catalog produces a seed command using the existing root identity.
   `propose_in_term` rejects the command if the planning term no longer owns
   leadership. A pending proposal prevents repeated planning in that term.
6. The initialization receipt or the recovered certified catalog authorizes
   publishing the initialized marker and entering Serving.

Serving is a catalog lifecycle state. Following recovery, a new election and
read/application barriers may still be needed before client operations succeed.

## Parameterized proof and bounded exploration

[RootFormation.tla](../proofs/tla/formation/RootFormation.tla) abstracts one
current leader's protocol over a retained Raft prefix. Its history distinguishes
an uncommitted initialization, a committed but unapplied initialization, an
applied catalog, a durable initialized marker, and later committed user data.
Crash removes volatile planning state. Election changes the term and may drop
an uncommitted suffix; committed history remains. Recovery requires the exact
root and original store authority.

[RootFormationProof.tla](../proofs/tlaps/formation/RootFormationProof.tla) has
14 declarations and 153 obligations. Induction proves the history, authority,
applied barrier, and fresh-plan invariants. The action safety theorem requires
fresh initialization appends to use an empty retained catalog and the current
planning term, and preserves later committed user data. Conditional progress
starts from any invariant crashed original store: with a stable leader/quorum,
successful I/O, and weak fairness of restore, drain, plan, append, and marker
publication, the store eventually reaches Serving.

This is a proof of the formation protocol at those interfaces, not a proof of
Raft election/log retention, the Rust implementation, or OS durability. The
model collapses log payloads to the distinguished root; identical abstract
stuttering transitions are ignored by the action property. The pending-state
fault therefore checks the stronger `RFCuts` state invariant directly. Runtime
regressions separately check real proposal indexes and retained catalog rows.
Store authority and persistence are obligations of the lifecycle/recovery
contracts; stable quorum and fair execution are liveness assumptions. Unlimited
failures and a minority without quorum do not have a progress guarantee.

```sh
python3 scripts/check-formation-protocol.py --tlapm /path/to/tlapm/bin/tlapm \
  --jar /path/to/tla2tools.jar --output /tmp/kv9-formation-protocol
python3 scripts/check-store-controls.py --output /tmp/kv9-store-controls
cargo test --locked -p kv9-server --lib \
  runtime::tests::original_stores_resume_formation_after_each_pre_catalog_crash_cut -- --exact
```

The runner checks two finite root/term bounds with two fingerprints, a fair
continuation, witnesses for later user data and committed-unapplied catalog,
and eight isolated protocol faults. Each fault requires passing original and
restored runs around its intended TLC violation and failed TLAPS obligation.
The faults cover the non-pristine fence, planning before apply, term changes,
forgotten pending initialization, ignoring an applied catalog, foreign roots,
lost-store authority, and unfair application. SANY audits assumptions and the
proof dependency tree; omitted proofs, injected axioms, and truncated or empty
outputs are rejected separately.

## Runtime and Chaos Mesh mapping

The real three-voter runtime regression restarts all original stores after
initial ConfState persistence, after an election-only prefix, and after an
initial catalog has committed while application is paused. Each cut must
recover the original certified root. A subsequent catalog row must survive
another full restart with deleted initialized markers, retaining its exact ID
and an advancing allocator. Compiled source controls restore the old startup
fence and remove the current-term application barrier to check those assertions.

The Chaos Mesh fixture starts all real processes behind an injected all-peer
network partition, confirms live owners and zero catalog commit, then uses
PodChaos to replace every owner while retaining its PVC. Fixture shell gates
hold replacement processes until all old Pod UIDs have gone; stale status
files cannot satisfy this check. After removing faults, the normal quorum and
late-voter handshake scene resumes from those exact disks, followed by the
independent KV history acceptance matrix. The evidence auditor replays its
linearization witness and rejects stale Pod IDs, missing victims, an already
committed initial catalog, and missing partition effects. Raw resources, victim UIDs, TCP
probes, and pre/post status files are retained for audit. The single Kind host
establishes process/Pod failure behavior, not host or zone failure isolation.
