# Durable group control: proof and refinement

`Control.lean` proves 12 statements over arbitrary modeled transitions. The
model treats the complete root/operation/task/group/replica encoding as one
opaque identity. It separates uncommitted planning, committed creation,
committed activation desire, local execution, failure and restart.

The inductive invariant establishes that automatic execution requires the
exact committed creation/desire pair for the local store. Preparation alone
cannot start execution, staging does not publish desire, and an existing
creation cannot be rebound. Committed desire survives all modeled transitions,
including restart. A constructive trace shows that a successful request can
be acknowledged before any local group is running. Reconciliation selects at
most one eligible request per turn and skips foreign, running and failed slots.

The 2026-09-16 source-pin review includes the new kind-102 namespace binding
path. It may stage the same immutable kind-101 activation desire before adding
its namespace row, under the same atomic planner/commit boundary. Subsequent
range publication is separate from activation and covered by the data-range
model. Existing creation-only requests still consume two TASKS rows; binding a
namespace consumes a third. No control-model transition or theorem changes.

Run with Lean 4.33.1:

```sh
python3 scripts/prove-group-control.py --lean /path/to/lean --output /fresh/output
```

The checker pins implementation/model sources, treats warnings as errors,
audits theorem axioms, rejects nine semantic mutations, and rejects custom
axioms and proof holes. Standard Lean axioms are allowed. This is a checked
abstract model with the following source correspondence, **not verified Rust
extraction**.

| Model boundary | Implementation and premise |
| --- | --- |
| `plan`, `discard` | `MetaTxn` overlay; failed planning drops the complete transaction, including both sequence allocations. The shared catalog planner lock spans the ordered barrier, planning and same-term exact commit. Existing Raft/catalog ordering is assumed. |
| `prepare`, immutable creation | Existing kind-100 `CreationIntent`, exact certified root and registered store incarnations; prior preparation and activation proof contracts still apply. No cancellation or reuse exists. |
| `commit`, desire/creation pair | Kind-101 task encodes the exact creation; creation and desire stage in one batch. Existing atomic positioned apply is a premise. `committed_activations` reads both rows from one fresh engine snapshot and alone mints `CommittedActivation`. A supplied RPC/decoded intent cannot mint it. |
| Receipt | `RuntimeBackend::create_data_group` returns only after exact applied commit; duplicate desire commits a new term-fenced Noop and labels the position a confirmation. A timeout/error preserves uncertainty. The client never automatically retries. |
| Automatic `activate` | Runtime gates on endpoint/membership readiness. Manager matches root, node and incarnation, then invokes the existing durable-Active-before-voting path. The model's local identity abstracts this entire exact tuple. Existing manually requested activation and recovered Active owners retain their separately proved authority. |
| `select` | Bounded 255-row scan, at most one new attempt per reconciliation turn, at most one turn per 100 ms. The source loop stops after failure as well as success. Running and failed slots are skipped. A new process must revalidate durable files before resuming. |

The model assumes immutable committed metadata, correct row decoding, exclusive
store ownership, and the existing durable filesystem/Raft contracts. It does
not prove authentication, cryptography, Rust execution, storage hardware, time
bounds under a blocked filesystem, eventual scheduling, placement, retirement,
public KV epochs or horizontal throughput gains. Local process failures and
negative tests are separate evidence; actual Chaos Mesh remains an open gate.
