# Fixed-voter activation and shared-owner proof

`Activation.lean` checks 15 statements about durable activation, recovery gates,
coalesced work notification, exclusive ownership and tick coalescing. Run:

```sh
python3 scripts/prove-group-activation.py --lean /path/to/lean --output /fresh/output
```

The checker pins the implementation/model sources, compiles with warnings as
errors, audits each theorem's axioms, rejects eight semantic defects and rejects
two proof-policy violations. Accepted compiler: Lean 4.33.1. Standard Lean
axioms are allowed; custom axioms and proof holes are forbidden.

The 2026-09-16 source-pin review adds immutable group identity to the state
machine before driver registration. Private Raw dispatch handles refer to that
existing owner; they create no additional pump or slot. Shutdown clears the
directory before stopping the worker pool and releasing group/store guards.
Range installation follows activation and is covered by the data-range model.
The existing activation model and its durable-before-voting premises remain.

## Refinement record

| Model | Rust implementation and interpretation |
| --- | --- |
| Initial prepared replica | `RegionManager::prepare`, committed metadata readback and the preparation model establish exact immutable creation/store binding and both durable stores. Preparation grants no voter. |
| `publish` | `start_group` synchronizes Active record, rename and ancestor directories before constructing a peer/inbox/driver. Interrupted publication poisons the slot; recovery resynchronizes any surviving record before validation. |
| `authorize` / `start` | `activate_data_group` and `resume_active` check local endpoint/membership readiness before `DriverPool::register`. These are activation gates, not public range authority. No driver handle escapes the public runtime API. |
| `restart`, `mayInitialize` | Active uses `DiskRaftStorage::recover`, existing engine topology and renewed validation. IntentDurable is the sole initializing phase; StorageReady also uses recover-only open. |
| `mayRecover` | Exact record/intent/root/node/incarnation comparison, fixed configuration, WAL topology, every replay position and final applied position checked against durable committed Raft term. Boolean inputs abstract these concrete check results. The theorem establishes conjunction of all gates, not correctness of the WAL decoder or Raft library. |
| Slot claim / begin / release | Registry mutex, `running` flag, stable slot index, driver `pump_started` CAS and `pump_gate`. Append-only slots cannot move or be reused while a worker runs. Each worker completes its selected slot before selecting again. Registration refuses another background owner. |
| `notify`, `clearWake`, `release` | Per-driver `WorkSignal`, weak shared-wake subscription, `begin_turn` before draining and completion wake after clearing `running`. Shared wake never clears child pending work. Subscription and registry insertion both wake the pool. |
| `tick` | Independent per-slot `TickDeadline::due`. One due check emits at most one tick and advances the monotonic deadline, without a catch-up loop proportional to missed intervals. |

The lifecycle invariant is inductive over arbitrary modeled traces: an owner
has durable Active, current-process validation/authorization and no terminal
failure; any sent vote/message implies durable Active. Active survives failure
and restart. Recovery requires all store/history predicates.

For scheduling, consuming pending work requires an owner; a step cannot replace
an occupied slot's owner; notifications coalesce; releasing ownership or
consuming the shared wake preserves child work. These are safety properties.
Eventual execution additionally requires eventual scheduling and finite service
times. The actual runtime uses two data workers, at most 255 slots and rotating
selection. Its registry lock never covers persistence/application. These finite
construction bounds are source-reviewed and directly tested, not cardinality
theorems about an extracted Rust heap.

## Explicit limits

This is an abstract model with source pins and a refinement record, **not
verified Rust extraction**. It relies on the existing durable WAL/fsync/rename
contract, immutable committed metadata creation, Raft safety, honest local
storage under exclusive ownership, Tokio/OS behavior and Rust control flow.
Core Raft quorum/persistence ordering is unchanged. One permanently blocked
worker leaves another data worker and the dedicated metadata owner; exhausting
all workers or shared physical resources is not a liveness guarantee.

This does not prove range publication/epochs, snapshot installation, replica
movement, retirement, total memory budgets, public API linearizability, physical
power-loss behavior or throughput scaling. Actual Chaos Mesh is separate;
local fault cuts and runtime-lifetime tests do not meet that acceptance gate.
