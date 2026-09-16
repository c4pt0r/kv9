# Durable data-group activation and bounded Raft workers

Updated 2026-09-16. Parent: [#9](https://github.com/c4pt0r/kv9/issues/9),
[#22](https://github.com/c4pt0r/kv9/issues/22).

## Result

Prepared data groups can now become independent fixed-voter Raft groups while
the node's existing listener remains online. The manager durably publishes
**Active before constructing a peer or registering its inbox/worker**. The
embedded activation call requires locally applied creation authority and local
endpoint/membership readiness. It does not publish a range or public KV route.

Active is one-way. On restart, the manager discovers the group's exact immutable
creation/store binding and opens its Raft log with **recover-only** semantics.
Missing/empty voting logs, missing engine topology, foreign configurations,
unexpected checkpoints, unpositioned user data, or engine positions without
matching durable committed Raft history refuse that group. Every positioned
replay frame is checked, including an invalid earlier frame behind a valid final
watermark. Valid voting and committed data history is preserved, not passed
through the empty/unstarted validator. Current groups retain their initial
voters; creation alone cannot authorize membership changes or lease policies.

Startup validates stores before acquiring owners. Recovered Active groups
resume after runtime membership/endpoint readiness; a mere unactivated intent
does not start a voter. Ambiguous activation failure requires recovery.
One failed group is isolated from metadata and other groups. Earlier
preparation-only builds reject the unknown Active phase rather than resetting
it; this fail-closed property is not complete rolling-upgrade acceptance.

## Resource and ownership contract

Each runtime lazily starts **two shared data workers**, independent of the
dedicated metadata owner. The pool accepts at most **255 append-only slots**.
Groups do not allocate a thread each. Each driver has one coalesced pending bit
and a weak subscription to the shared wake signal. A producer publishes work
before notification; shared wake consumption cannot clear the child's pending
bit. Registration wakes for preexisting work, and completion wakes after
releasing the slot to preserve notifications arriving during execution.

Selection rotates through eligible groups. The registry marks a slot in flight
before releasing its mutex; no network, persistence or state-machine work runs
under that mutex. Each group has its own monotonic tick deadline, independent
of traffic; a delayed turn emits one tick rather than a catch-up burst.
Driver ownership CAS prevents a second pool or dedicated pump claiming it.
Shutdown stops and joins workers before dropping group handles/locks or the
parent store guard. A terminal driver is not revived by an activation retry.

This is an initial O(number-of-local-groups) scan with fixed worker and slot
bounds. It does not establish a node-wide byte budget, storage/CPU isolation,
fairness under permanently blocked I/O, or measured scheduling overhead. Those
remain D01 acceptance work. The [dedicated data RPC](GROUP-WIRE-FENCING.md)
continues to prevent legacy metadata receivers from consuming group messages.

## Local evidence

Six new focused tests cover:

- Six before/after Active-publication cuts, proving no peer inbox is acquired
  on interrupted publication; retries fail closed until recovered.
- Ten invalid recovery cases, including lost/empty logs, engine/topology loss,
  foreign configuration/data, phase rollback over voting history, applied state
  beyond commit, wrong terms and an invalid intermediate frame.
- Notifications before subscription and while parked without waiting for a
  60-second tick; another worker progresses beside a deliberately blocked
  group; duplicate ownership and slot-cap refusal.
- Three real NodeRuntime instances over HTTP/2 and independent durable stores:
  online creation, distinct data leaders, isolated values at the same key,
  exact write receipts, metadata/other-group progress during an apply pause,
  leader loss, successor writes, replica catchup, and reopening every store.
  Deleting one active group's log then leaves that group refused while the
  other group and metadata recover and continue operating.

The runtime test initially exposed two harness assumptions: a one-shot leader
transfer can target a superseded local leader observation, and a locally ready
endpoint does not imply an elected metadata leader. It now waits under bounded
deadlines for the required roles and retries only the best-effort transfer
control, not unknown user writes. Both failed runs are retained.

Final local checks pass **878 workspace tests/doctests**, with 28 existing
ignored, strict all-target Clippy and formatting. Eight single-defect Rust
controls are rejected at their intended assertions. The
[activation/scheduling model](../proofs/lean/group-activation/README.md) checks
15 theorems with eight semantic controls and two proof-policy controls. The
existing nine preparation theorems and their controls are also rechecked with
an explicit Active-to-ready projection and current source pins; historical
validation packets are unchanged. This is not verified Rust extraction.

The [validation receipt and 108-member evidence packet](group-activation-v1/README.md)
retain source bindings, raw logs, original failures and all defect controls.

## Remaining scope and benchmark gate

These are local component and multi-runtime integration tests, **not actual
Chaos Mesh or independent-host failure evidence**. No new QPS or measured
horizontal-scaling result is claimed. Public range/epoch routing, network admin
creation/reconciliation, group checkpoint/manifest wiring, retirement, replica
movement and splits remain unfinished. Public user KV still uses metadata's
existing group; the new data drivers are private to the runtime.

Continue with bounded online control/reconciliation and public routing, then
safe movement/split. Preserve actual Chaos Mesh, snapshot/retention proof and
no-singleton gates. The [3/6/9-node benchmark contract](HORIZONTAL-SCALING-PLAN.md)
remains mandatory: three voters per group, fixed durability and p99 budgets,
equal-load latency, explicit resource costs and online-expansion measurements.
The 1.6x/2.4x targets remain targets. Daily checks stay local; no hosted CI ran.
