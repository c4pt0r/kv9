# kv9-client (Rust)

Async client for kv9 on `tonic`. Lives in the workspace (`sdk/rust/kv9-client`) and generates its
protocol types from `crates/server/proto/kv9.proto` at build time, so it always speaks exactly what the
server in the same checkout serves.

```toml
[dependencies]
kv9-client = { path = "sdk/rust/kv9-client" }
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

```rust
use kv9_client::{Client, Mutation};

let client = Client::new("127.0.0.1:20161", "my-client-token")?
    .with_peers([(1, "127.0.0.1:20161"), (2, "127.0.0.1:20162"), (3, "127.0.0.1:20163")])
    .with_timeout(std::time::Duration::from_secs(5));
client.create_keyspace("app", 0).await?;                             // once
let ks = client.keyspace_named("app").await?;                        // by name; keyspace(id) by id

let applied = ks.put(b"user/1".to_vec(), b"alice".to_vec()).await?; // Applied { term, index }
ks.batch(vec![                                                       // ONE Raft entry, in order
    Mutation::Put { key: b"user/2".to_vec(), value: b"bob".to_vec() },
    Mutation::Delete { key: b"user/1".to_vec() },
]).await?;
assert_eq!(ks.batch_get(vec![b"user/1".to_vec(), b"user/2".to_vec()]).await?,
           vec![None, Some(b"bob".to_vec())]);
let rows = ks.scan_all(b"user/".to_vec(), b"user0".to_vec(), 1024).await?;
ks.delete_range(b"user/".to_vec(), b"user0".to_vec()).await?;
```

`Keyspace` starts at epoch `(1, 1)` and refreshes it once through `GetRegion` when the server answers
`FAILED_PRECONDITION` without a marker (`with_auto_refresh(false)` disables that; `refresh_epoch` does it by
hand). `Client` is `Clone` and shares its lazily dialled channels; it follows `NotLeader` hints through the
`peers` map (bounded by `with_max_redirects`, default 3) and sticks to the node that answered.

## Errors

`Error` is one enum; its variants mirror the shared contract in `sdk/README.md`:

| Variant | When | Safe to retry? |
| --- | --- | --- |
| `NotLeader { leader }` | not the leader; nothing executed | yes (already followed when `peers` knows the node) |
| `ReadUnconfirmed { phase }` | the read barrier did not establish | yes |
| `AdmissionRefused { reason }` | refused before execution | after backing off |
| `PartialWrite { committed_chunks, last_applied, cause }` | `delete_range` committed some chunks, then failed | continue from `last_applied` |
| `Rpc { code, message }` | unmarked status; `is_precondition()` for stale epoch / cross-region batch | a write's outcome is unknown |
| `Bounds(_)` | local bound violated; nothing sent | fix the request |
| `Protocol(_)` / `Transport(_)` | unreadable marker / dial failure | no |

Bounds: key ≤ 4096 B, value ≤ 65 536 B, 1..=256 items per batch, ≤ 1 MiB per message, ≤ 30 s per call.

## Tests

```bash
cargo test -p kv9-client          # marker classification, local bounds, doctest
scripts/sdk-e2e.sh                # boots a cluster and runs examples/smoke.rs against it
```
