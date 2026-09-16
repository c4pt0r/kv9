# Scoped Raw client and surviving metadata discovery

Checkpoint: 2026-09-16. D02 / [#23](https://github.com/c4pt0r/kv9/issues/23).
This increment builds on [initial data-range routing](DATA-RANGE-ROUTING.md).
It does not implement automatic split, replica movement or measured scaling.

## Public behavior

`kv9_server::client::routed::RoutedRawClient` discovers a newly bound Raw
keyspace from multiple metadata endpoints and sends scoped data requests over
the selected persistent tonic stream. Unary tonic is a reference option. A
logical call supports Get, Put, Delete, BatchGet and atomic BatchPut. Every key
in a batch must fit one range. Cross-range batches are refused before dispatch;
there is no fan-out that changes atomicity. The scoped client does not expose
Scan or DeleteRange yet. The legacy client remains available separately.

The client is configured with a cluster root digest, tenant, keyspace and at
least two distinct seed endpoints. A seed is a discovery candidate. Follower
responses can teach other active metadata endpoints; only a metadata leader's
post-ReadIndex snapshot supplies a binding and data replicas. Stale leader hints
cannot prevent trying the other known metadata candidates within a lookup round.
An unavailable seed has a bounded probe timeout rather than consuming the whole
call deadline. A deployment still needs enough reachable endpoints and a Raft
quorum; a bounded call can legitimately end before an election completes.

Each data request carries the complete canonical range descriptor: cluster
root, creation digest, region, tenant, keyspace, configuration/version epochs,
half-open bounds and seal state. The server selects the group by region and
requires an exact match. The owning group retains its Safe ReadIndex snapshot
checks and ordered write fence. Endpoint and leader hints never grant ownership.
Cached observations can race or become stale; server checks remain authoritative.

The new `LookupRawRoute` and `RoutedRaw` RPCs and stream opcode 5 are distinct
from the unscoped protocol. An older receiver cannot silently execute a request
after ignoring unknown scope fields. There is no automatic legacy fallback.
The previous offline V3 writer-upgrade requirement remains in force.

## Retry and resource contract

- One logical call owns one immutable payload, absolute deadline and shared
  attempt budget. Lookup, redirect and data RPCs all consume that budget.
- Typed no-effect scope/NotLeader refusals can refresh or redirect a write.
  Admission and cross-range refusals terminate the call.
- Any uncertain data-write result is `UnknownWrite` and terminates that call.
  A timeout or dropped reply is not evidence that the write failed. Another
  logical call may proceed, but the client never replays the unknown write.
- Read failures can try another replica. If a client alone loses contact with
  the data leader while voters remain connected, that leader need not change;
  discovery failover does not promise writes during that isolation.
- At most 32 discovery candidates, 32 cached endpoint connections, 128 cached
  routes, 256 concurrent calls and 16 attempts per call are configurable. The
  overall deadline is at most 30 seconds. Active connection references can
  outlive cache eviction but remain bounded by admitted calls. Existing stream,
  admission, message, key, value and 256-item batch limits remain in force.

`RoutedReport` records lookup/data attempts, node IDs, per-attempt times and
failures, stop reason, and exact region/epochs/binding digest for data attempts.
No successful outcome is inferred from lookup alone. Secrets are omitted.
The retry interval must allow enough time for the deployment's election budget;
an attempt limit with very short backoff can expire before a healthy election.

## Persistent correctness workload

Build `kv9-routed-workload` from the `kv9-server` package. Pass a JSON
`RoutedConfig` using `--config FILE` and the token through `KV9_CLIENT_TOKEN`.
It maintains one client for its entire stdin/stdout lifetime. Each input line
has a positive, strictly increasing `id` and an `operation`; byte strings use
JSON byte arrays, for example:

```json
{"id":1,"operation":{"kind":"put","key":[107],"value":[118]}}
{"id":2,"operation":{"kind":"get","key":[107]}}
```

It flushes a start record and one complete outcome per invocation. EOF exits
cleanly; malformed, oversized or reused input IDs fail. It records unknown
writes without replay. This serial fault-history runner is not a throughput
benchmark, and its correctness results do not qualify new QPS.

## Checked evidence and remaining scope

Local checks: **895 workspace tests/doctests pass, 28 existing tests ignored**;
strict all-target Clippy and formatting pass. Seven routing tests cover silent
seeds, learned candidates behind cyclic stale hints, applied-but-lost replies,
scope refresh, cross-range batches, malformed identities/refusal metadata and
one persistent streaming client surviving each real runtime endpoint's loss
and restart. The real runtime test also exercises batch ordering/duplicates and
rejects foreign root, creation and tenant scopes.

The [checked model](../proofs/lean/routed-client/README.md) adds **15 theorems,
11 semantic defect controls and two proof-policy controls**. It proves shared
budgets, original deadlines, no replay after Unknown, late completion, exact
scope and whole-batch boundaries over an explicit abstract transition system.
Preparation, activation, control and range proofs were rechecked after source
correspondence review. The proof is not verified Rust extraction; its Raft,
storage, transport and trusted-refusal premises are documented.

The local Chaos harness uses three retained-storage voters, two distinct groups
and two persistent clients. It targets first-seed isolation, a data-leader
container kill, a two-way leader partition and client-only data-leader isolation.
The positive seed-discovery case keeps the current metadata and data leaders
reachable. A follower currently returns metadata hints rather than proxying a
linearizable lookup; a client isolated from the metadata leader can exhaust its
lookup budget even while the voter quorum is healthy. That limitation is not
covered up by selecting an endpoint for the client in the harness.

The independent reader is deliberately limited to the fixture's serial,
unique-key Put/Get histories. It retains Unknown alternatives and requires
post-fault readback of every submitted mutation. It does not certify general
concurrent linearizability or uncertain atomic batches. Storage archives and
process identities are checked separately. Garbage-collected container records
are reported as absent with unknown exit codes, not invented successful exits.

The accepted actual run is `chaos-fourth`: the complete harness and a separate
final reader both exited **0**. It recorded **48 logical calls: 44 successes,
one UnknownWrite and three lookup failures**, with no uncertain-write replay.
All four fault windows passed; a direct minority control returned typed
NotLeader and its key remained absent after healing. Post-fault reads checked
every submitted mutation, and each group's applied prefix covered its successful
write receipts on all three voters. Both client processes ended cleanly. Three
stopped-store archives (337,920 tar bytes, 102 entries) passed full-member
readback before exact namespace cleanup; historical resources were unchanged.
The four observed server process identities were absent after shutdown; their
CRI records had been garbage-collected, so normal shutdown exit codes remain
unknown. The killed leader's actual exit 137 is retained in its fault record.

The [portable evidence packet](routed-client-v1/README.md) retains all attempts.
The first harness run failed on an uncreated tenant; the second failed by treating
a stale persisted status sample during restart as a fatal test error. The third
completed the runtime campaign but failed an outdated tenant constant in the
reader. Its original exit 1 is preserved alongside a corrected runtime-only
audit. The fourth is the sole complete accepted campaign. Thirty-eight offline
controls check the final history reader, CLI parser, archives, lifecycle evidence
and stale-status handling. No failed wrapper is relabeled as a successful run.

This is a single-host default-feature debug correctness campaign. It is not
the complete C02 matrix: all-voter Chaos permutations, concurrent histories,
atomic-batch/I/O fault cases, moving ownership and separate physical failure
domains remain open. No general availability or full D02 checkbox is implied.

The metadata mapping remains one initial full-keyspace range per new group.
Split publication, moving ranges, changing replica sets, routed scans and
delete-range, group retirement and aggregate scheduling/storage bounds remain
open. The fake stale-scope test is a protocol test, not an implemented split.
The [benchmark contract](HORIZONTAL-SCALING-PLAN.md) separately requires 3/6/9
independent hosts, unchanged three-voter durability, equal p99 budgets,
resource costs and online expansion. No new QPS or scaling gain is claimed.

Reproduce the local implementation checks:

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo build -p kv9 -p kv9-server --bin kv9 --bin kv9-routed-workload
python3 scripts/prove-routed-client.py --lean /path/to/lean --output /fresh/proof
```

`scripts/routed-client-chaos.py --help` lists the explicit binary/image hashes,
local qualification manifest, existing Kind/kubeconfig and fresh output required
for the reviewed local environment. Build/load the image first. Then run
`scripts/check-routed-client-chaos.py --run RUN --output FRESH_JSON` independently;
the final reader requires both runtime evidence and completed cleanup. Runtime
failure preserves the owned namespace for diagnosis; archive and verify stopped
stores before removing that exact namespace. The packet retains the exact
accepted commands, binary/source hashes, image identity and prior cleanup runs.

Daily CI remains local. No hosted GitHub workflow is dispatched for this checkpoint.
