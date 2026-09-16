# Data-group RPC fencing before activation

Updated 2026-09-16. Parent issues: [#9](https://github.com/c4pt0r/kv9/issues/9),
[#22](https://github.com/c4pt0r/kv9/issues/22).

## Problem and resulting behavior

Older V2 `BatchRaft` receivers ignored `region_id`. Sending a data-group message
to one could put it into the metadata Raft inbox. A successful capability probe
would not solve this: the address can serve an older process after a restart,
downgrade or endpoint replacement.

Data-group workers now call **`BatchDataRaft` on every session**. Old V2
dispatchers do not recognize that method and return `UNIMPLEMENTED`. There is
no fallback. Metadata retains `BatchRaft` and wire ID 0. Data uses catalog IDs
greater than 1; ID 1 remains reserved. Modern receivers reject any wrong-domain
batch before delivering any of that batch, and preserve root identity, local
receive authority, authenticated sender and destination checks on both methods.

Each peer has at most two lazily allocated workers/connections: one for
metadata and one shared by every data group. Their outbound queues each hold
at most 4096 messages. Saturating data leaves metadata queue capacity available.
Both workers receive endpoint changes; allocation identities prevent queued
A→B→A messages from reusing the first A generation. Dropping the owner aborts
both workers. An immediately rejected RPC now waits at least the existing
100-ms retry minimum before reconnecting, avoiding an `UNIMPLEMENTED` spin.

This is a transport prerequisite for durable group activation. It does not
start prepared groups, publish public routes, change Raft quorum semantics,
introduce a coordinator service, or establish a complete rolling-upgrade gate.
Older intermediate group fixtures that used `BatchRaft` for data must use the
new method. No production data-group activation was present in those commits.

## Verification and retained evidence

Seven new tests exercise real loopback HTTP/2:

- Both methods reject mixed/wrong domains and reserved IDs before delivery.
- Data RPCs retain root, local authority, envelope and payload identity gates.
- A legacy dispatcher refuses the new method, while a positive hazard control
  demonstrates that its old method consumes a nonmetadata envelope.
- A production sender against that dispatcher never falls back and respects
  retry spacing while metadata continues to arrive.
- After successful data delivery, same-address downgrade and distinct-address
  replacement refuse data traffic; a subsequent upgrade restores delivery.
- Queue saturation reserves metadata capacity, many groups share the same
  workers, A→B→A updates both workers, and owner destruction stops them.

The legacy dispatcher is a test emulator of the old method surface and unsafe
receiver behavior, not an execution of a released old binary. Same-address
downgrade closes the active RPC and removes the new method from subsequent
dispatch. It is an application-layer fault test, not an actual process crash.
The existing five multi-group tests now use the split method paths, including
three independent three-voter Raft groups with different leaders and isolated
state, plus progress while one group's driver is paused.

The [Lean model and refinement record](../proofs/lean/group-wire/README.md)
contain **15 checked theorems**, six rejecting semantic controls and two
proof-policy controls. Six single-defect Rust controls test missing method
preflight, unsafe legacy fallback, shared queues, a stale data worker route,
missing retry delay and stale queued-envelope delivery. Source pins and raw
compiler/test logs are retained with the validation packet.

Final [local validation](group-wire-fencing-v1/README.md) passes **872 workspace
tests/doctests**, with 28 existing ignored, plus strict all-target Clippy and
formatting. The 51-member evidence archive has been read back and hash checked.

Actual multi-group Chaos Mesh, durable activation/recovery, bounded Ready/tick
scheduling, fair data-group admission and node-wide byte budgets remain open.
Existing single-group Chaos results are not transferred to these new paths.

## Horizontal-scaling acceptance remains benchmark-driven

There is **no new QPS or scaling measurement** in this checkpoint. The
[benchmark contract](HORIZONTAL-SCALING-PLAN.md) remains the acceptance rule:
independent 3/6/9-node topologies with three voters per data group, fixed
semantics and durability, sustainable throughput under a fixed p99 budget,
latency at equal offered load, and online split/movement impact. The 1.6x and
2.4x throughput thresholds are predeclared targets, not observed results.

The next mainline step is a durable active phase before voting, recover-only
opening of active stores, and bounded scheduling of independent groups.
Public routing, movement and split follow. Daily validation stays local;
GitHub workflows remain manual for releases and key milestones.
