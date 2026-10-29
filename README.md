# kv9

A **multi-tenant, cloud-native distributed key-value store** in one binary. Inspired by TiKV, without the
separate control plane: every `kv9` process is storage node, metadata member and router at once; cluster
metadata is replicated by kv9's own Raft group; object storage holds the durable checkpoints.

> Status: **v0.** A working replicated Raw KV with multiple Raft groups, linearizable reads, atomic range
> batches, TTL, MinIO checkpoints and recovery. APIs and on-disk formats are not stable yet; see
> [Status and limits](#status-and-limits).

## Quick start

Three nodes on one machine, local disks only (no object storage). Prerequisites: Rust stable, `protoc`,
a C/C++ toolchain, cmake, Python 3; Linux or WSL. Everything below is bash, run from the repository root.

```bash
cargo build --workspace
mkdir -p kv9-local

# Tokens: bootstrap (provisioning), cluster (node-to-node), client (applications). Use your own values.
export KV9_BOOTSTRAP_TOKEN=bootstrap-secret KV9_CLUSTER_TOKEN=cluster-secret KV9_CLIENT_TOKEN=client-secret
export KV9_CLIENT_TOKENS="admin=$KV9_CLIENT_TOKEN"

# 1. Prepare one store per node and record their identities in a root descriptor.
source scripts/root_provision.sh
voters=1@127.0.0.1:20161,2@127.0.0.1:20162,3@127.0.0.1:20163
./target/debug/kv9 root-create --output kv9-local/root.bin --voters "$voters" \
    --store-incarnations "$(prepare_root_stores ./target/debug/kv9 kv9-local "$voters")"

# 2. Initialize and start the three nodes.
for n in 1 2 3; do
  ./target/debug/kv9 init  --root kv9-local/root.bin --node-id $n --data-dir kv9-local/n$n
  ./target/debug/kv9 start --node-id $n --addr 127.0.0.1:2016$n --data-dir kv9-local/n$n \
      > kv9-local/n$n.log 2>&1 &
done

# 3. Wait until every node serves (each writes a status file next to its data).
for n in 1 2 3; do until grep -q '^bootstrap_state=Serving' kv9-local/n$n/status 2>/dev/null; do sleep 0.5; done; done
```

The cluster elects a leader among the three voters, initializes its catalog and serves gRPC on the three
`--addr` ports. Stopping and restarting a node is `kill` plus the same `start` command; the data directory
holds everything it needs. [QUICKSTART](docs/QUICKSTART.md) explains each step, the status file, and how to
run the same cluster on MinIO so that checkpoints and recovery use object storage.

## Using the REPL

`kv9-repl` is an interactive shell built on the Rust SDK. Give it any node and the peer map; it follows
leader hints by itself.

```bash
cargo build -p kv9-repl
KV9_CLIENT_TOKEN=client-secret ./target/debug/kv9-repl --addr 127.0.0.1:20161 \
    --peers 1=127.0.0.1:20161,2=127.0.0.1:20162,3=127.0.0.1:20163
```

```text
kv9> create app
keyspace app (id 100) created; selected
kv9[app]> put user/1 alice
applied (term=1, index=5)
kv9[app]> batch put user/2 bob, del user/1
applied (term=1, index=6)
kv9[app]> mget user/1 user/2
user/1 => (nil)
user/2 => bob
kv9[app]> scan user/ user0
user/2 => bob
(1 row(s))
kv9[app]> delrange user/ user0
deleted: 1 chunk(s), last applied (term=1, index=7)
```

| Command | Meaning |
| --- | --- |
| `keyspaces`, `info`, `create <name> [tenant]`, `use <name>` (`use id:<n>` for an id) | catalog and selection |
| `get <k>`, `mget <k>...` | point read; one quorum-confirmed multi-key read |
| `put <k> <v>`, `del <k>`, `mput <k> <v>...` | single writes; put-only atomic batch |
| `batch put <k> <v>, del <k>, ...` | the atomic range batch: one Raft entry, in order, all or nothing |
| `scan [start [end [limit]]]`, `delrange <start> <end>` | one page of a range; chunked range delete |
| `region [key]`, `refresh`, `help`, `quit` | region/epoch of the keyspace; re-read it |

Tokens are plain words, `"quoted strings"` or `hex:<bytes>`; every write prints its applied `(term, index)`.
With stdin not a terminal the REPL runs scripts: `printf 'put a 1\nget a\n' | kv9-repl --addr …`.
Details: [`sdk/rust/kv9-repl/README.md`](sdk/rust/kv9-repl/README.md).

## Client SDKs

Four SDKs share one contract ([`sdk/README.md`](sdk/README.md)): bearer-token auth, a keyspace handle that
carries the region epoch and refreshes it once on a stale-epoch answer, typed refusals read from gRPC
trailers (never from message text), local bounds checks, and `NotLeader` redirects through a peer map.

| Language | Package | Transport |
| --- | --- | --- |
| Python 3.10+ | [`sdk/python`](sdk/python/) (`pip install -e sdk/python`) | `grpcio`, sync |
| Rust | [`sdk/rust/kv9-client`](sdk/rust/kv9-client/) | `tonic`, async |
| TypeScript / Node 18+ | [`sdk/typescript`](sdk/typescript/) (`@kv9/client`) | `@grpc/grpc-js`, async |
| C++17 | [`sdk/cpp`](sdk/cpp/) (CMake target `kv9_client`) | `grpc++`, sync |

Python:

```python
import kv9

client = kv9.Client("127.0.0.1:20161", token="client-secret",
                    peers={1: "127.0.0.1:20161", 2: "127.0.0.1:20162", 3: "127.0.0.1:20163"})
client.create_keyspace("app")                    # once; returns the id
ks = client.keyspace_named("app")                # keyspaces are used by name

ks.put(b"user/1", b"alice")                      # -> Applied(term, index)
ks.get(b"user/1")                                # -> b"alice" | None
ks.batch([kv9.Mutation.put(b"user/2", b"bob"),   # one Raft entry, in order, all or nothing
          kv9.Mutation.delete(b"user/1")])
ks.batch_get([b"user/1", b"user/2"])             # -> [None, b"bob"], one quorum-confirmed view
for key, value in ks.scan_all(b"user/", b"user0"):
    print(key, value)
ks.delete_range(b"user/", b"user0")              # chunked; PartialWriteError reports progress
```

Rust:

```rust
use kv9_client::{Client, Mutation};

let client = Client::new("127.0.0.1:20161", "client-secret")?
    .with_peers([(1, "127.0.0.1:20161"), (2, "127.0.0.1:20162"), (3, "127.0.0.1:20163")]);
client.create_keyspace("app", 0).await?;                              // once
let ks = client.keyspace_named("app").await?;                         // by name

let applied = ks.put(b"user/1".to_vec(), b"alice".to_vec()).await?;   // Applied { term, index }
ks.batch(vec![
    Mutation::Put { key: b"user/2".to_vec(), value: b"bob".to_vec() },
    Mutation::Delete { key: b"user/1".to_vec() },
]).await?;
let values = ks.batch_get(vec![b"user/1".to_vec(), b"user/2".to_vec()]).await?;
let rows = ks.scan_all(b"user/".to_vec(), b"user0".to_vec(), 1024).await?;
```

TypeScript and C++ follow the same shape (`ks.batch([...])`, `ks.scanAll(...)` / `ks.ScanAll(...)`); each
SDK README has the full example and its error table. `scripts/sdk-e2e.sh` boots a cluster and runs every
SDK's smoke test against it.

### What the API guarantees

- **Writes are atomic within a range.** `put`, `delete`, `batch_put` and `batch` are one fenced Raft entry:
  every mutation applies at one `(term, index)` or none does. All keys of a batch must lie in one region of
  one keyspace; a batch that crosses a region is refused whole, never split. Later mutations for the same
  key win. The response is the applied position. `delete_range` is the exception: it commits in bounded
  chunks and reports how many committed.
- **Reads are linearizable.** Every read establishes a quorum ReadIndex barrier first. A node that cannot
  answers with a typed refusal, `read_unconfirmed` or `not_leader` (with the leader's id), instead of a
  stale value. A plain transport error carries neither marker, so "the cluster refused" and "no answer"
  stay distinguishable.
- **Keyspaces are the tenant boundary.** Keys are encoded with the keyspace prefix and validated on every
  request; a region never spans a keyspace. People address keyspaces by **name** (the REPL, the CLI and
  every SDK resolve it once); the wire carries the numeric id, which the explicit forms (`use id:<n>`,
  `--keyspace-id`, `client.keyspace(id)`) still accept.

## Administration

`kv9-admin` ([`sdk/rust/kv9-admin`](sdk/rust/kv9-admin/README.md)) is the operator's tool. Give it any
node's address: it finds the metadata leader, reads the catalog, drives membership changes and waits
for them to be committed, and prints JSON for scripts.

```bash
cargo build -p kv9-admin
export KV9_ADMIN_TOKEN=client-secret
./target/debug/kv9-admin --endpoints 127.0.0.1:20161 cluster info
./target/debug/kv9-admin --endpoints 127.0.0.1:20161 node list
./target/debug/kv9-admin --endpoints 127.0.0.1:20161 cluster health        # exit 0 only when every verdict is ok
./target/debug/kv9-admin --endpoints 127.0.0.1:20161 keyspace create --name app
./target/debug/kv9-admin --endpoints 127.0.0.1:20161 group create --voters 1,2,3 --label orders   # a data group
./target/debug/kv9-admin --endpoints 127.0.0.1:20161 keyspace create --name orders --group <creation-task>
./target/debug/kv9-admin --endpoints 127.0.0.1:20161 group describe <creation-task>              # every replica's view
./target/debug/kv9-admin --endpoints 127.0.0.1:20161 group add-replica <creation-task> --to 4    # the cluster does the rest
./target/debug/kv9-admin --endpoints 127.0.0.1:20161 group move-replica <creation-task> --from 1 --to 5
./target/debug/kv9-admin --endpoints 127.0.0.1:20161 node admit 4 --addr 127.0.0.1:20164
#   → join_ticket=…; on the new host: KV9_JOIN_TICKET=… kv9 join …, then kv9 start …
./target/debug/kv9-admin --endpoints 127.0.0.1:20161 node promote 4 --wait  # learner → voter
./target/debug/kv9-admin --endpoints 127.0.0.1:20161 node remove 2 --wait   # keeps ≥ 3 voters
./target/debug/kv9-admin --endpoints 127.0.0.1:20161 node transfer-leader --to 3 --wait   # planned maintenance
```

`KV9_ADMIN_PRINCIPALS=admin` on the servers limits the administrative RPCs to that token's principal;
application tokens keep the data plane. Each node also writes `<data-dir>/status` and
`<data-dir>/metrics.json` once a second, and `cluster status` shows the same document over gRPC. With
`KV9_AUTO_REPLACE_VOTER=1` the metadata leader replaces a lost voter with a caught-up learner on its
own, promotion first and removal second ([VOTER-REPLACEMENT](docs/VOTER-REPLACEMENT.md)). Adding or
moving a data-group replica is one committed plan: the cluster attaches the learner, captures and
pins the image in object storage, installs it inside the destination process, records the evidence,
promotes the voter and retires the source by itself ([REPLICA-ADDITION](docs/REPLICA-ADDITION.md)).
With `KV9_AUTO_REPAIR_REPLICA=1` each data group's leader takes the same lost-voter verdict for its own
group and commits that move plan itself, to the least-loaded active store outside the group
([REPLICA-REPAIR](docs/REPLICA-REPAIR.md)). `kv9-admin node drain <id> --wait` takes a store out of
service in one verb: the cluster hands its leaderships away, moves every replica off it one plan at a
time, promotes a learner before removing it from a three-voter metadata group, and reports `drained`
([DRAIN-NODE](docs/DRAIN-NODE.md)).

## Architecture

Diagrams and the walk through one node: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md). The full design and
its rationale, section by section: [DESIGN.md](DESIGN.md). In short:

- **One binary, no placement driver.** A cluster is N identical processes. The catalog (tenants, keyspaces,
  regions, peers, membership) lives in a reserved system keyspace replicated by kv9's own metadata Raft
  group; nodes join as learners and are promoted to voters.
- **Regions are Raft groups.** A keyspace is served by the metadata group or bound to its own data group
  (region) with its own range and epoch; epoch fences and leader hints drive client re-routing. Each replica
  applies through a positioned WAL into an in-memory versioned index.
- **Object storage holds the checkpoints.** With `KV9_STORAGE=minio`, leaders stream immutable SST
  checkpoints to the bucket, publish manifests by compare-and-swap, replicas adopt them, the covered WAL is
  reclaimed and cold recovery reads the bucket plus the surviving WAL tail.
- **Committed authority for every change.** Membership, data-group creation, migration, retention and
  compaction are immutable task rows confirmed idempotently through Raft; executors act from local applied
  state. The Lean and TLA+ models under `proofs/` are pinned to the sources they describe.
- **Multi-tenancy and flow control are the model, not features.** Tenant → keyspace → region; one token
  currency for capacity, fairness and backpressure (DESIGN §7).

Concepts: a **tenant** owns **keyspaces**; a keyspace is a namespace encoded as a key prefix; a **region** is a
range shard and a Raft group that never spans a keyspace; a **range batch** is an ordered list of puts and
deletes applied as one Raft entry.

## Status and limits

- The full dataset lives in memory on every replica; SSTs are checkpoints, not yet a read path (no
  incremental LSM, no block cache).
- Range split and merge are design goals, not in this tree; metadata is one Raft group.
- Tenant admission and fairness are partial; TLS is not implemented (use a private network).
- Acknowledged writes that are not yet checkpointed depend on the Raft replicas' local disks.
- There are no transactions by design: the multi-key write primitive is the atomic range batch
  (DESIGN §1, §9.1).

The order of work is in [docs/ROADMAP.md](docs/ROADMAP.md); work packages are tracked in
[GitHub issue #9](https://github.com/c4pt0r/kv9/issues/9).

## Repository layout

```
src/                   the single `kv9` binary (provisioning, start, `kv9 client` verbs)
crates/
  common/  meta/  engine/  raft/  region/  raw/  server/
sdk/                   python/  rust/kv9-client  rust/kv9-repl  rust/kv9-admin  cpp/  typescript/
proofs/                Lean / TLA+ models pinned to the sources they describe
scripts/               acceptance, chaos and benchmark harnesses
docs/                  design packets; start with ARCHITECTURE.md, QUICKSTART.md, ROADMAP.md
DESIGN.md              the architecture and its rationale
```

Build: `cargo build --workspace`. Test: `cargo test --workspace` (the qualifying run is serial,
`-- --test-threads=1`); `./scripts/minio-kv-e2e.sh` runs three nodes against a real MinIO container with
failover and restart. [docs/TESTING.md](docs/TESTING.md) is the testing contract.

## License

Apache-2.0 — see [`LICENSE`](LICENSE).
