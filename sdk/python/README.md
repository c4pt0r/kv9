# kv9 Python SDK

Synchronous client for kv9 on `grpcio`. Python 3.10+.

```bash
pip install -e sdk/python            # from a checkout; regenerate stubs with sdk/python/gen.sh
```

```python
import kv9

client = kv9.Client(
    "127.0.0.1:20161",
    token="my-client-token",
    peers={1: "127.0.0.1:20161", 2: "127.0.0.1:20162", 3: "127.0.0.1:20163"},  # follows NotLeader hints
    timeout=5.0,
)
ks_id = client.create_keyspace("app")          # served by the metadata group, epoch (1, 1)
ks = client.keyspace(ks_id)

applied = ks.put(b"user/1", b"alice")          # Applied(term, index): the exact applied position
ks.get(b"user/1")                               # b"alice" | None
ks.batch([                                      # ONE Raft entry, all or nothing, in order
    kv9.Mutation.put(b"user/2", b"bob"),
    kv9.Mutation.delete(b"user/1"),
])
ks.batch_get([b"user/1", b"user/2"])            # [None, b"bob"], one quorum-confirmed view
for key, value in ks.scan_all(b"user/", b"user0"):
    ...
ks.delete_range(b"user/", b"user0")             # chunked; see DeleteRangeResult / PartialWriteError
client.close()
```

Keyspaces bound to their own data group carry a region epoch; `client.keyspace(id)` starts at `(1, 1)`
and refreshes it once through `GetRegion` when the server answers `FAILED_PRECONDITION` without a marker
(`auto_refresh=False` disables that; `ks.refresh_epoch()` does it by hand).

## Errors

| Exception | When | Safe to retry? |
| --- | --- | --- |
| `NotLeaderError(leader_node_id)` | the node is not the leader; nothing executed | yes — the client already follows the hint when `peers` knows the node |
| `ReadUnconfirmedError(phase)` | the read barrier did not establish | yes, it is a read |
| `AdmissionRefusedError(reason)` | refused before execution | yes, after backing off |
| `PartialWriteError(...)` | a `delete_range` committed some chunks, then failed | continue from the reported position |
| `RpcError(code, message)` | anything else; `.precondition` is `True` for stale epoch / cross-region batch | a write's outcome is unknown — decide yourself |
| `BoundsError` | local bound violated; nothing sent | fix the request |
| `ProtocolError` | the server sent a marker the SDK cannot interpret | no |

Bounds: key ≤ 4096 B, value ≤ 65 536 B, 1..=256 items per batch, ≤ 1 MiB per message, ≤ 30 s per call.

## Tests

```bash
/path/to/venv/bin/pytest sdk/python              # unit tests (marker classification)
scripts/sdk-e2e.sh                                # boots a cluster and runs tests/test_smoke.py too
```
