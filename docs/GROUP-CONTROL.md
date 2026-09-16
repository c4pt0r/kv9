# Online durable data-group control

Updated 2026-09-16. Tracks [#9](https://github.com/c4pt0r/kv9/issues/9) and
[#22](https://github.com/c4pt0r/kv9/issues/22).

Follow-up: [initial public data routing](DATA-RANGE-ROUTING.md) adds namespace
bindings to these groups and raises the writer/wire floor to V3. The validation
counts below describe this earlier control checkpoint.

## Behavior and operational boundary

Authenticated `CreateDataGroup` and `kv9 client create-data-group` now request
creation and activation through the existing metadata Raft leader. The request
binds an exact certified root, a nonzero 16-byte operation ID and 3, 5 or 7
active registered voter stores. Creation and activation desire commit in one
atomic catalog batch under the existing planner lock, ordered barrier and
same-term apply receipt. No external coordinator or fixed management endpoint
is required. A follower returns the existing explicit leader refusal.

An existing operation with the same exact replica incarnations retains the
same task, region and intent digest. A changed replica binding is refused.
Retrying an existing desire returns a **new confirmation receipt**, never the
original mutation's position. Timeouts and transport errors leave the outcome
unconfirmed; the client has no implicit retry. Retain the operation ID and exact
request when explicitly confirming an uncertain outcome.

```sh
KV9_CLIENT_TOKEN="$TOKEN" kv9 client create-data-group \
  --addr "$METADATA_LEADER" --root-digest "$ROOT_DIGEST" \
  --operation-id 00000000000000000000000000000001 --voters 1,2,3
```

The response means **durable desire accepted**, not elected, available or
routable. It explicitly prints `readiness=not_asserted`. This creates an empty,
fixed-voter data group; it does not move existing KV data or assign a key range.
Existing `create_data_group_intent` remains preparation-only.

Every eligible node reconciles locally applied desires, including after a
leader failure or full process restart. Exact root/node/store-incarnation
matching precedes disk preparation. A node attempts at most one new activation
each 100 ms reconciliation turn. Running and terminally failed slots are
skipped. Failure is isolated and recovery must revalidate the disk. Previously
durable Active groups retain their existing restart path.

The existing global **255 TASKS-row bound** is unchanged. An online-created
group consumes two rows, so an otherwise empty task table admits 127 such
groups. Other tasks and preparation-only groups share this budget. Exhaustion
rejects the complete request without partial allocation; exact retries still
work at capacity. This suffices for the planned fixed 18-group benchmark, but
is not a final cluster-size limit or node-wide memory budget.

The per-process atomic status file now includes `data_groups` JSON and
`data_group_control_error`. Per-group observations distinguish prepared,
active and failed state and report local role/term/leader, committed index,
engine applied index and the separate Raft driver applied position. A newly
elected empty group can have an engine position of zero after applying its
Raft election Noop. Status is evidence only and grants no routing authority.

## Local validation

The full local workspace run passes **882 tests/doctests**, with 28 existing
ignored tests. Strict all-target Clippy and formatting checks pass.

The [checked model and refinement](../proofs/lean/group-control/README.md) add
12 theorems, nine semantic defect controls and two proof-policy controls. The
existing preparation and activation proofs are rerun with refreshed source
pins. Four new Rust tests cover committed readback, immutable preparation,
idempotency, full-capacity retries, transaction rollback, seven malformed-row
cases, bounded local attempts, failed-slot isolation and foreign incarnations.

`scripts/data-group-control-e2e.py` runs three independent production-binary
processes over real TCP and local disk. It checks credential/root/follower
refusals; two autonomously activated groups; killing the metadata leader;
confirming the same operation on a successor with a new receipt; creating a
third group while one voter is down; returning-voter reconciliation; restarting
all processes; and refusing one missing Active log while other groups and
metadata recover. It records binaries, commands, status observations and
verdicts, and terminates every owned child process on success or failure.

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo build -p kv9 --bin kv9
python3 scripts/data-group-control-e2e.py --bin target/debug/kv9 --output /fresh/output
```

The retained [validation packet](group-control-v1/README.md) includes unsuccessful
development attempts. Harness fixes distinguish whitespace-separated command
fields from status lines, use the driver watermark for applied election Noops,
and target the actual `raft/raft.log` path. An invalid primary-key mutation is
rejected by the catalog before commit; its test now checks that refusal and
the decoder separately. These corrections do not count as database failures
or as extra accepted benchmark runs.

## Next and remaining acceptance

Next is D02: range-aware public routing with authoritative epoch validation at
the target group's ordered apply boundary, explicit cross-range semantics and
bounded safe rerouting. Existing data must stay with its current owner until
the migration protocol is ready. D03 transfer, D04 manual/automatic split and
placement follow the [scaling plan](HORIZONTAL-SCALING-PLAN.md).

This increment does not close D01: retirement/tombstones, group checkpoint
ownership, aggregate byte budgets and full fault acceptance remain open.
There is **no new QPS result or measured horizontal scaling**. Independent
3/6/9-host benchmarks with three replicas, fixed durability, p99 budgets,
resource costs and online split/migration remain required. The local
three-process SIGKILL/recovery gate is not an actual Chaos Mesh run. No hosted
GitHub CI is triggered for this incremental checkpoint.
