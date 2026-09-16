# Initial public routing to durable data groups

Checkpoint: 2026-09-16. Implements the first part of D02 / [#23](https://github.com/c4pt0r/kv9/issues/23),
following [online group creation](GROUP-CONTROL.md). This is a development
checkpoint, not complete D02, automatic split or horizontal-scaling acceptance.

## Behavior

The new authenticated `CreateDataKeyspace` RPC and CLI bind a **new Raw
keyspace** to an existing durable data-group creation. The metadata leader
atomically commits the namespace, its initial region, an immutable binding task
and any missing activation desire. Duplicate exact requests retain the binding
and return a new confirmation position. Conflicting names/tenants or reusing
an existing namespace are refused. The receipt confirms metadata application;
it does not assert that the data group is already serving.

```sh
KV9_CLIENT_TOKEN="$TOKEN" kv9 client create-data-keyspace \
  --addr "$METADATA_LEADER" --root-digest "$ROOT_DIGEST" \
  --creation-task "$CREATION_TASK" --name orders
```

Each eligible replica reconciles the committed binding. Its data-group leader
proposes initial ownership through that group's own ordered Raft log. The
runtime publishes a private dispatch handle only after that descriptor is
applied. GET, BatchGet, Put, BatchPut, Delete, Scan and DeleteRange then use
the data group's engine and Raft driver. Metadata stores no copy of those Raw
values. A missing or pending handle cannot fall back to metadata: both the
legacy API gate and its ordered write fence reject the new namespace marker.

The initial mapping is **one full-keyspace range per group**. Existing
keyspaces retain their original owner. Neither this API nor a decoded
descriptor can move existing data. This increment has no public seal command,
automatic split, range reassignment, replica movement or cross-group transaction
API. Existing split requests still return NotImplemented.

## Ordered authority and read semantics

The group's durable descriptor contains root, exact creation digest, region,
keyspace, tenant, configuration/version epochs, half-open bounds and seal state.
Every public write carries a private proposal permit and an ordered `Fenced`
command. At apply, both ordinary and coalesced Raw paths check the exact current
epoch, unsealed state, namespace, key encoding, column family and **every key**
against that group's current descriptor. A crossing batch is rejected before
any item is applied. A corrupt or foreign descriptor is an apply error with
no watermark advancement, not a successful stale rejection.

The new range command permits initial installation or a compare-and-set from
an open descriptor to the same identity/namespace/bounds with version advanced
once and state sealed. It cannot reopen, expand or transfer ownership. A write
prepared before a seal but ordered after it is refused. Identical-state
confirmations do not mutate ownership.

Reads use the existing Safe ReadIndex barrier and authorize/read one subsequent
engine snapshot. They cannot exchange an authorized snapshot for a later view.
A read concurrent with sealing may linearize before it. DeleteRange preserves
the existing chunked, non-atomic progress contract and revalidates each chunk;
it does not promise a global snapshot or a cross-region transaction. A reversed
finite empty range retains the existing zero-work receipt behavior.

New data-group reads currently use the blocking snapshot path after asynchronous
ReadIndex. The earlier resident/lease experiments are not promoted to these
groups. There are **no new performance measurements** for this implementation.

## Upgrade and bounds

Pre-range metadata writers do not recognize the new namespace marker. To
exclude them, node-to-node RPC paths now use `kv9.raft.v3`, with no fallback,
and each store publishes `KV9LIFE3` durably before opening an owner. Previous
V1/V2 binaries reject that store. Existing lifecycle identity, phase, root and
Raft data remain intact. V1/V2 records can be read only to perform the forward
format fence.

**Use an offline all-node upgrade.** Mixed-version rolling availability is
unsupported. The actual-binary local gate checks V2-to-V3 protocol isolation
in both directions, preservation of an acknowledged legacy write, endpoint
migration under V3, and refusal by the old writer after the format upgrade.

The bounded TASKS catalog still has 255 shared rows. Creation, activation and
namespace binding consume three rows per fully bound group: at most **85**
such groups if the task table is otherwise empty. Other tasks share that budget.
The two-worker data scheduler remains in place. This is sufficient for the
planned fixed 18-group benchmark panel, not a final cluster resource budget.

## Local evidence

- Full workspace: **888 passing tests/doctests**, 28 existing ignored; strict
  all-target Clippy and formatting checks pass.
- New metadata test checks atomic binding, committed readback, idempotency and
  conflict rollback. Raft tests check canonical decoding, namespace/bounds/epoch
  rejection in ordinary and grouped apply, no accepted batch prefix, terminal
  sealing, and malformed/foreign authority without watermark advancement.
- A real public-TCP runtime test checks two namespaces with the same logical
  key, legacy metadata fallback refusal, values absent from metadata, follower
  refusal, BatchPut/BatchGet,
  scans, two-chunk DeleteRange, point deletion, restart and cached-handle refusal
  after sealing.
- Three default-feature production processes check public data-group routing,
  acknowledged reads/writes after data-leader loss, metadata-leader loss,
  creating a third group/namespace with one voter down, all-process restart,
  and isolating a missing Active data log without recreating it.
- The [checked model and correspondence](../proofs/lean/data-range/README.md)
  add **16 theorems**, eleven semantic defect controls and two policy controls.
  Preparation, activation, control and wire proofs are rechecked after explicit
  source-pin reviews. These are abstract models, not verified Rust extraction.

Reproduce the main local gates:

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo build -p kv9 --bin kv9
python3 scripts/data-group-control-e2e.py --bin target/debug/kv9 \
  --exercise-raw --output /fresh/output
```

The [portable packet](data-range-v1/README.md) retains accepted checks and
development failures. Early fixture compilation errors, proof syntax errors,
lint failures and a stale generated-protobuf build were corrected. A first
upgrade attempt used an unqualified stale binary and failed; the accepted
upgrade uses the independently built previous commit in a separate Cargo target
and requires different old/new binary hashes. No failed attempt is counted as
accepted evidence. Build old/new revisions with separate target directories.

## Next and remaining gates

The subsequent [scoped-client increment](ROUTED-CLIENT.md) adds a bound range
and replica lookup, surviving metadata endpoints, bounded stale-scope refresh,
exact root/namespace binding and write-uncertainty preservation. The metadata
mapping still represents initial full-keyspace groups; split/move publication
and routed scan/delete-range remain open. Next implement D03 safe replica
transfer and D04 durable manual/automatic split. D01 retirement, group checkpoint
ownership and aggregate byte budgets also remain open.

Local SIGKILL tests are **not actual Chaos Mesh** or full C02 history acceptance.
Every applicable fault family and client endpoint loss still need that evidence.
No single indispensable coordinator is added, but this checkpoint does not close
the complete no-single-point-of-failure acceptance gate.

Effective scaling must be demonstrated **in the benchmark**, as specified in
the [scaling plan](HORIZONTAL-SCALING-PLAN.md): independent 3/6/9 hosts, three
voters per group, fixed durability and p99 budgets, resource costs and online
split/migration. Additional groups or passing correctness tests do not establish
a throughput gain. No hosted GitHub CI is triggered for this checkpoint.
