# kv9 client SDKs, from an operator's side

Four SDKs speak the public gRPC service (`crates/server/proto/kv9.proto`, `kv9.v1.Kv9`) with one
shared contract (`sdk/README.md`; each directory has its own README). Use this page when a user
asks how an application talks to the cluster, when an application's errors need explaining, or
when an operation should be scripted rather than typed into `kv9-admin`.

Contents: 1 which SDK · 2 connecting · 3 data-plane calls · 4 typed errors and retries ·
5 the operator surface (Rust) · 6 after a split, merge, move or restore · 7 testing the SDKs

## 1. Which SDK

| SDK | Directory | Transport | Surface |
| --- | --- | --- | --- |
| Rust | `sdk/rust/kv9-client` | tonic (async) | data plane **and** the operator `admin` module `kv9-admin` is built on |
| Python 3.10+ | `sdk/python` (`pip install -e sdk/python`) | grpcio (sync) | data plane + `create_keyspace`, `list_keyspaces`, `cluster_info`, `get_region` |
| TypeScript / Node 18+ | `sdk/typescript` (`npm install && npm run build`) | @grpc/grpc-js (async) | same as Python |
| C++17 | `sdk/cpp` (CMake target `kv9_client`) | grpc++ (sync) | same as Python |

`kv9-repl` (`sdk/rust/kv9-repl`) is an interactive data-plane shell on the Rust SDK; `kv9-admin`
(`sdk/rust/kv9-admin`) is the operator CLI on the Rust SDK's `admin` module. Python stubs are
generated (`sdk/python/gen.sh`); regenerate them after any proto change.

## 2. Connecting

Any node's address works. Give a `peers` map (`node id → address`) so the client can follow
`NotLeader` hints to the leader; without it a request to a follower fails with `NotLeader`.
The token is one of the server's `KV9_CLIENT_TOKENS` (the bearer header `authorization: Bearer
<token>`). Address keyspaces by name (`keyspace_named` / `keyspaceNamed` / `KeyspaceNamed`);
the by-id constructor stays for callers that hold the id.

```python
client = kv9.Client("127.0.0.1:20161", token="…", peers={1: "127.0.0.1:20161", 2: "…", 3: "…"}, timeout=5.0)
ks = client.keyspace_named("orders")
```
```rust
let client = Client::new("127.0.0.1:20161", "…")?.with_peers([(1, "127.0.0.1:20161"), (2, "…"), (3, "…")]);
let ks = client.keyspace_named("orders").await?;
```

TLS is not implemented yet; the transport is plaintext HTTP/2.

## 3. Data-plane calls

`get`, `put`, `delete`, `batch_get` (one quorum-confirmed view), `batch_put`, `batch` (the
atomic range batch: ordered puts and deletes applied at ONE `(term, index)` or not at all, never
across a range boundary), `scan` (one page; follow `resume_from` across range boundaries; Rust
`scan_all` does it for you), `delete_range` (chunked per range). Every write returns its exact
applied `(term, index)` receipt; compare the pair, never the index alone. Local bounds are
checked before any RPC: key ≤ 4096 B, value ≤ 64 KiB, 1..=256 batch items, message ≤ 1 MiB,
deadline ≤ 30 s.

## 4. Typed errors and retries

Outcomes come from trailing metadata, never from message text:

| SDK error | Meaning for the operator |
| --- | --- |
| `NotLeader { leader }` | the node is not the leader, nothing executed; the SDK follows the hint through `peers` (the only automatic retry) |
| `ReadUnconfirmed { phase }` | the read barrier did not establish (often an election); retry the read |
| `AdmissionRefused { reason }` | back-pressure (`KV9_MAX_RAFT_LOG_ENTRIES`, request limits); back off |
| `PartialWrite { … }` | a `delete_range` committed some chunks then failed; the receipt says how far |
| `Rpc { FAILED_PRECONDITION }` without a marker | stale epoch (a split, merge or move changed the range): the SDK refreshes the epoch through `GetRegion` once and retries once; a batch crossing a range boundary is refused by design |
| `Rpc { UNAVAILABLE / DEADLINE }` on a write | the outcome is **unknown**: the write may have applied; replaying is the application's decision |

During a split's or merge's **seal window** the range answers neither reads nor writes for a few
seconds: applications see unavailability or stale-epoch errors and retry through it. During a
drain or a leader transfer they see `NotLeader` hints, which the SDK follows.

## 5. The operator surface (Rust `kv9_client::admin`)

Every `kv9-admin` verb is one call here, for scripts that should not shell out. Calls go to the
metadata leader (`Client` follows hints); writes commit one row and return its task and receipt.

| Need | Calls |
| --- | --- |
| Observe | `node_status`, `list_nodes`, `list_data_groups` / `list_data_groups_detailed` (with readback errors), `list_tasks`, `node_endpoint`; plus `cluster_info`, `list_keyspaces`, `get_region` on `Client` |
| Membership | `admit_node` (full mode; returns the join ticket), `admit_node_mode(…, "auto"\|"full"\|"scoped")`, `revoke_admission`, `promote_node`, `remove_node`, `transfer_leader` (metadata region or a data region); `NodeRow.meta_mode` |
| Groups and keyspaces | `create_data_group`, `create_data_keyspace`, `add_replica` (mode add or move) |
| Ranges | `split_range`, `merge_ranges` |
| Retire a node | `drain_node` |
| Backup and restore | `backup_keyspace`, `list_backups`, `verify_backup`, `retire_backup`, `restore_keyspace`, `list_restores` |
| Stop an operation | `abort_task` |

Membership calls return `changed: false` when the state already holds (the CLI's `outcome:
confirmed`). Waiting is the caller's job: poll `list_nodes` (`drained`), `list_data_groups`
(plan/split/merge stages), `list_backups` (`complete`), `list_restores` (`published`), as
`kv9-admin --wait` does. The admin principal's token is required when the servers set
`KV9_ADMIN_PRINCIPALS`; an application token gets `PermissionDenied`.

The Python, TypeScript and C++ SDKs have no operator surface; script operations from them by
calling `kv9-admin --output json` and reading `result`.

## 6. After a split, merge, move or restore

- **Split / merge / move**: nothing to change in the application. The SDK refreshes the epoch
  once on a stale-epoch answer and follows `NotLeader` hints; scans follow `resume_from`.
- **Restore**: a restore publishes a NEW keyspace (`restore create --as <name>`); the
  application switches by opening the new name (`keyspace_named("orders-restored")`). Nothing
  is renamed in place.
- **Drain / node removal**: keep `peers` current; a client whose only endpoint was the drained
  node loses its way in once the process stops.

## 7. Testing the SDKs

```bash
scripts/sdk-e2e.sh      # builds kv9, boots three nodes, runs every SDK's smoke test
```

Each smoke test reads `KV9_ADDR`, `KV9_TOKEN`, `KV9_KEYSPACE` and optionally `KV9_PEERS`. Python
unit tests: `pytest sdk/python`; C++: `ctest` in `sdk/cpp/build`; TypeScript: `npm test`.
