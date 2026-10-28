# kv9-repl

An interactive shell for kv9 built on the [Rust SDK](../kv9-client/). One binary, no extra dependencies.

```bash
cargo build -p kv9-repl
KV9_CLIENT_TOKEN=my-client-token target/debug/kv9-repl --addr 127.0.0.1:20161 \
    --peers 1=127.0.0.1:20161,2=127.0.0.1:20162,3=127.0.0.1:20163 [--keyspace 100] [--timeout-ms 5000]
```

```text
kv9> keyspaces
     100  tenant=0  app
kv9> use 100
keyspace 100 selected (epoch 1/1 until refreshed)
kv9[100]> put user/1 alice
applied (term=1, index=7)
kv9[100]> batch put user/2 bob, del user/1
applied (term=1, index=8)
kv9[100]> mget user/1 user/2
user/1 => (nil)
user/2 => bob
kv9[100]> scan user/ user0
user/2 => bob
(1 row(s))
kv9[100]> delrange user/ user0
deleted: 1 chunk(s), last applied (term=1, index=9)
```

| Command | Meaning |
| --- | --- |
| `keyspaces`, `info`, `create <name> [tenant]`, `use <id>` | catalog and selection |
| `region [key]`, `refresh` | region/epoch of the selected keyspace; re-read it |
| `get <k>`, `mget <k>...` | point read; one quorum-confirmed multi-key read |
| `put <k> <v>`, `del <k>`, `mput <k> <v>...` | single writes; put-only atomic batch |
| `batch put <k> <v>, del <k>, ...` | the atomic range batch: one Raft entry, in order, all or nothing |
| `scan [start [end [limit]]]`, `delrange <start> <end>` | one page of a range; chunked range delete |
| `help`, `quit` | |

Tokens are plain words, `"quoted strings"` (may contain spaces) or `hex:<bytes>` for binary keys and
values; results that are not printable text are shown as `hex:…`. Every write prints its applied
`(term, index)`. Typed refusals are explained in words (not leader + hint, read unconfirmed, admission
refused, partial write, stale epoch / cross-region batch); an unmarked RPC failure on a write is reported
as "write outcome unknown", exactly as the SDK reports it.

**Scripted use.** When stdin is not a terminal the prompt is suppressed and the process exits non-zero if
any command failed, so `printf 'put a 1\nget a\n' | kv9-repl --addr …` works in shell pipelines.
`scripts/sdk-e2e.sh` runs such a session against a fresh cluster.
