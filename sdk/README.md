# kv9 client SDKs

Thin, dependency-light clients for the kv9 public gRPC surface (`crates/server/proto/kv9.proto`,
service `kv9.v1.Kv9`). Four languages share one contract; each directory has its own README.

| SDK | Directory | Transport | Status |
| --- | --- | --- | --- |
| Python 3.10+ | [`python/`](python/) | `grpcio` (sync) | v0 |
| Rust | [`rust/kv9-client/`](rust/kv9-client/) | `tonic` (async) | v0 |
| C++17 | [`cpp/`](cpp/) | `grpc++` (sync) | v0 |
| TypeScript / Node 18+ | [`typescript/`](typescript/) | `@grpc/grpc-js` (async) | v0 |

## The shared contract

**Endpoint and auth.** Connect to any node's public `--addr` over plaintext HTTP/2 (TLS is not implemented
yet; see README limits). Every request carries the metadata header `authorization: Bearer <client-token>`,
where the token is one of the server's `KV9_CLIENT_TOKENS`.

**Keyspaces by name.** People address a keyspace by its name; every SDK has a by-name constructor
(`keyspace_named` / `keyspaceNamed` / `KeyspaceNamed`) that does one `ListKeyspaces` lookup and hands
back the same handle as the by-id constructor, which stays for callers that already hold the id.

**Request context.** Every raw operation names its keyspace and the region epoch it believes in:
`RequestContext { keyspace_id, region_epoch { conf_ver, version } }`. A keyspace created with
`CreateKeyspace` is served by the metadata group and uses epoch `(1, 1)`. A keyspace bound to its own data
group gets its epoch from `GetRegion(keyspace_id, key)`. Every SDK's keyspace handle starts at `(1, 1)` and,
by default, refreshes the epoch once through `GetRegion` when the server answers `FAILED_PRECONDITION`
without a typed marker, then retries the call exactly once if the epoch changed.

**Operations.** `get`, `put`, `delete`, `batch_get` (one quorum-confirmed view, results in input order),
`batch_put` (put-only atomic batch), `batch` (the atomic range batch: ordered puts and deletes applied at
ONE `(term, index)` or not at all — see `DESIGN.md` §9.1), `scan` (one page; a nonempty `resume_from` means
"continue exactly here, even if the page is empty"), `delete_range` (chunked; reports how many chunks
committed). Admin: `create_keyspace`, `list_keyspaces`, `cluster_info`, `get_region`. The Rust SDK alone also
exposes the operator surface (`admin` module: `node_status`, `list_nodes`, `admit_node`, `promote_node`,
`remove_node`, …) that `kv9-admin` is built on; the other SDKs are data-plane clients.

**Bounds, checked locally before any RPC.** Key ≤ 4096 bytes, value ≤ 65 536 bytes, batch items 1..=256,
encoded message ≤ 1 MiB, one call deadline ≤ 30 s.

**Typed outcomes come from trailing metadata, never from message text.** Values are exact.

| Marker | Status code | Meaning | SDK error |
| --- | --- | --- | --- |
| `kv9-not-leader: true` (+ `kv9-leader-node-id: <id>`) | `FAILED_PRECONDITION` | this node is not the leader; nothing executed | `NotLeader { leader }` |
| `kv9-read-unconfirmed: quorum \| apply` | `UNAVAILABLE` | the read barrier did not establish; nothing answered | `ReadUnconfirmed { phase }` |
| `kv9-admission-refused: request_count \| encoded_bytes \| request_too_large` | `RESOURCE_EXHAUSTED` | refused before execution; back off | `AdmissionRefused { reason }` |
| `kv9-partial-write: true` + `kv9-committed-chunks`, `kv9-last-applied-term/index` | any | a `delete_range` committed some chunks, then failed | `PartialWrite { … }` |
| (none) | `FAILED_PRECONDITION` | stale epoch, batch crossing a region, API-type mismatch | `Rpc { code, message }` after the one epoch refresh |
| (none) | other | transport or server failure; a write's outcome is **unknown** | `Rpc { code, message }` |

**Redirects.** With a `peers` map (`node_id → address`) the SDK follows a `NotLeader` hint, at most
`max_redirects` hops, and sticks to the node that answered. `NotLeader` is the only failure any SDK retries
on its own: the server guarantees it executed nothing. A deadline or an unmarked `UNAVAILABLE` on a write is
reported as-is; replaying it is the caller's decision, because the write may have applied.

**Receipts.** Writes return the exact applied `(term, index)`. Compare the pair, never the index alone.

## Trying them against a local cluster

```bash
scripts/sdk-e2e.sh            # builds kv9, boots three nodes, creates a keyspace, runs every SDK's smoke
```

Each SDK's smoke test reads `KV9_ADDR`, `KV9_TOKEN`, `KV9_KEYSPACE` and optionally
`KV9_PEERS` (`1=127.0.0.1:20161,2=…`); the harness sets them.
