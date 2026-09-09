# Raft receive authority for dynamic members

Issue [#42](https://github.com/c4pt0r/kv9/issues/42) exposed a receiving-side
incarnation boundary. A fresh disk reused the numeric ID and address of a
promoted dynamic member. Before registration rejected its different
`StoreIncarnation`, a same-root heartbeat entered its empty Raft log and
raft-rs panicked with an out-of-range commit position. Registration rejection
alone did not prevent participation.

## Implementation contract

A new `RuntimeDiscovery` starts with Raft receive authority closed. Every
`BatchRaft` batch checks that local authority after the root digest check and
before decoding or enqueuing messages. Sender authentication remains a separate
requirement. A fresh dynamic member also leaves the Raft driver owner stopped:
preventing inbound messages while allowing elections or outbound votes would
leave another participation path open. Discovery and registration remain usable.

There are two dynamic-member authorization paths:

1. The live incarnation receives its own successful registration response. The
   runtime first installs the return route, membership catch-up capability and
   exact receipt, then publishes receive authority, then starts the owner.
   An unsuccessful or lost response does not grant authority. A successful
   retry can complete an already consumed admission for the same store.
2. Recovery finds the certified local cluster identity, an Active local NODES
   row with exactly the durable `StoreIncarnation`, and a durable ConfState
   containing this member. That store starts without a new registration RPC,
   including a crash cut where the initialization marker is missing. A
   mismatched local row is an explicit recovery error.

The leader serializes admission planning with the catalog mutex and existing
term/application barriers. Before consuming a Pending admission or changing a
transport route, it checks any existing NODES incarnation binding. Revocation
and a renewed ticket do not allow a different store to reuse that numeric ID.
The original store can use the renewed ticket and its new canonical address.
The route is installed after successful validation and durable admission
consumption, before AddLearner needs it. A rejected caller cannot overwrite it.

`InvalidIncarnation` is a typed permanent registration refusal. Its wire reason
is `invalid-store-incarnation`; status publishes
`rejected_invalid_incarnation`. Status also records
`raft_receive_authorized` and `raft_owner_started`. These are diagnostics, not
additional recovery certificates.

## Model, proof and implementation mapping

[ReceiveAuthority.tla](../proofs/tla/receive/ReceiveAuthority.tla) models one
dynamic numeric replica ID, an arbitrary nonempty set of positive incarnation
identifiers, a retained binding, volatile authorization, durable local recovery
certification, routing, receipt delivery, crashes and restarts. Zero denotes
absence. Revocation/new tickets may permit a route change for the same binding;
they cannot erase that binding.

| Model transition | Implementation boundary |
|---|---|
| `GRBind` | Valid pending admission consumption and NODES incarnation committed through `RuntimeBackend::register` |
| `GRRoute` | `register_catalog_peer` with the committed endpoint generation after binding/admission validation |
| `GRReply`, `GRGrant`, `GRStart` | Successful `WalkOutcome::Registered`, receipt/capability publication, `authorize_raft`, `driver.spawn` |
| `GRReceive` | Per-batch `RaftGrpcService::batch_raft` receive check before inbox publication |
| `GRCertify` | Active row and ConfState durably applied to this store |
| `GRCrash`, `GRRestart` | Volatile authority discarded; `local_member_is_active` checks the exact durable store before recovery authorization |

[ReceiveAuthorityProof.tla](../proofs/tlaps/receive/ReceiveAuthorityProof.tla)
contains 13 declarations and 117 checked obligations. Induction proves the
receive, owner, route and immutable-binding properties for all specified crash
and restart executions. Three progress lemmas establish receipt-to-gate,
gate-to-owner and owner-to-message delivery. `GRFairSpec` starts at any state
satisfying the invariant and specifies a crash-free continuation with weak
fairness of grant, owner start and receive. Thus a valid response in such a
continuation eventually leads to a running owner and received message.

The proof does not assume quorum availability during unlimited failures. Its
registration and durable-certification transitions abstract trusted committed
consensus results; the model does not prove Raft, ticket cryptography, Rust
compilation, filesystem durability or a complete source-level refinement. The
standard-module/backend trust boundary is documented in the
[TLAPS inventory](../proofs/tlaps/README.md). Initial-root formation and initial
voter lost-disk reuse are excluded from this dynamic-member model.

## Reproduction and adversarial controls

```sh
python3 scripts/check-receive-protocol.py --tlapm /path/to/tlapm/bin/tlapm \
  --jar /path/to/tla2tools.jar --output /tmp/kv9-receive-protocol
python3 scripts/check-receive-controls.py --output /tmp/kv9-receive-controls
python3 -m unittest discover -s scripts -p test_local_admin_client.py -v
bash scripts/root-trust-e2e.sh
```

The protocol runner uses the pinned tools and strict semantic proof audit. It
checks both bounded crash/restart configurations with two fingerprint choices,
a fair continuation, two reachable witnesses, and seven isolated model/proof
faults: receive/start before grant, grant without a receipt, recovery with
another store's certificate, routing before validation, rebinding, and removal
of owner fairness. Every fault has passing original and restored sources. A
finite TLC counterexample and a failed deductive obligation are retained as
separate evidence. Omitted proofs, extra axioms, empty/incomplete proof output
and incomplete temporal-check output are rejected. Parser failures and timeouts
do not count as expected protocol rejections.

Five compiled source controls select real runtime tests with original, mutated
and restored code. They cover stale-heartbeat ingress, premature owner start,
wrong local incarnation, early endpoint mutation and loss of the typed
incarnation refusal for a renewed ticket. The last control leaves the independent
endpoint-writer binding check intact; it demonstrates a refusal-classification
regression, not an actual rebind. The test checks unchanged routing and pending
admission before checking the public refusal type. The existing real non-seed-leader registration test also verifies
same-store recovery without an initialization marker or join ticket. The root
E2E requires a live current child with an explicit rejection, empty fatal field,
closed authority, no owner and commit position zero; a dead process cannot pass.

## Accepted scope and remaining boundaries

The original #42 receive/replacement scope is reconciled in
[REPLACEMENT-ACCEPTANCE.md](REPLACEMENT-ACCEPTANCE.md). Initial voters now require
independently prepared [store lifecycle authority](STORE-LIFECYCLE.md), and
[formation recovery](ROOT-FORMATION.md) resumes on the original durable stores.
A copied root descriptor cannot mint the old incarnation on a newly prepared
disk. [Production endpoint migration](ENDPOINT-RECOVERY.md) also replaces live
transport workers and has accepted retained-PVC Chaos evidence.

These component proofs keep their stated trust and durability assumptions.
Whole-disk clones, future numeric-ID reuse, general replica replacement and
multi-group migration need their own ownership protocols under #24. No plain
numeric ID or copied descriptor proves possession of an original log.

The earlier Chaos admission-pressure failure at `6779112` remains retained in
[#43](https://github.com/c4pt0r/kv9/issues/43). Subsequent accepted complete
matrices are separate evidence, not a relabeling of that failed command. Daily
verification is local; hosted runs are reserved for releases and key milestones.

## Historical receive-authority increment

Local validation for this increment passed 505 workspace/unit/integration/doc
tests (22 opt-in tests remained ignored), Clippy with warnings denied, seven
local CLI/status tests, the five source controls (15 compiled executions),
the protocol suite (28 TLC executions and 28 proof/audit executions), root E2E,
and the real three-replica MinIO E2E covering failover, remote checkpoint,
reclaimed WAL, live-tail recovery and deletes. Independent audits compared
source hashes, selected tests, proof inventories, exact obligation counts and
expected failures. The source controls were rerun after the test-only Clippy
correction. This local result does not replace exact-revision hosted acceptance.

The original endpoint assertions inspected the configured route and did not
prove replacement of an already-created transport worker. That distinct gap
was reproduced and resolved under [#47](RAFT-ROUTING.md), followed by the public
API, durable recovery and actual changed-address Chaos acceptance in
[ENDPOINT-RECOVERY.md](ENDPOINT-RECOVERY.md).
