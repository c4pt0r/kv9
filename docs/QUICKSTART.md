# Running kv9

How to build kv9, bring up a cluster, talk to it, and run it on object storage. Everything here is
bash from the repository root. Prerequisites: Rust stable, a working `protoc`, a C/C++ toolchain, cmake
and Python 3; Linux or WSL is the tested environment. A persistent cluster additionally needs an
S3-compatible object store (the examples use MinIO in Docker).

## 1. Build

```bash
cargo build --workspace          # ./target/debug/kv9 (server + CLI) and ./target/debug/kv9-repl
```

## 2. A local three-node cluster

kv9 has no external control plane, so a cluster is bootstrapped from a **root descriptor** that names
the first three voters and the store identity of each. Three tokens separate the roles involved:

| Variable | Used by | Meaning |
| --- | --- | --- |
| `KV9_BOOTSTRAP_TOKEN` | `root-create`, `init` | authorizes provisioning of the root and the initial stores |
| `KV9_CLUSTER_TOKEN` | `start` | node-to-node authentication inside the cluster |
| `KV9_CLIENT_TOKENS` | `start` | `principal=token,...`: the bearer tokens clients may present |
| `KV9_CLIENT_TOKEN` | `kv9 client`, `kv9-repl`, SDKs | the token a client presents |

Generate real secrets for anything that outlives a demo (`python3 -c 'import secrets; print(secrets.token_hex(24))'`).

```bash
mkdir -p kv9-local
export KV9_BOOTSTRAP_TOKEN=bootstrap-secret KV9_CLUSTER_TOKEN=cluster-secret KV9_CLIENT_TOKEN=client-secret
export KV9_CLIENT_TOKENS="admin=$KV9_CLIENT_TOKEN"

# Prepare one store per node (on the disk it will live on) and record their identities in the root.
source scripts/root_provision.sh
voters=1@127.0.0.1:20161,2@127.0.0.1:20162,3@127.0.0.1:20163
./target/debug/kv9 root-create --output kv9-local/root.bin --voters "$voters" \
    --store-incarnations "$(prepare_root_stores ./target/debug/kv9 kv9-local "$voters")"

# Initialize each store from the root, then start the node.
for n in 1 2 3; do
  ./target/debug/kv9 init  --root kv9-local/root.bin --node-id $n --data-dir kv9-local/n$n
  ./target/debug/kv9 start --node-id $n --addr 127.0.0.1:2016$n --data-dir kv9-local/n$n \
      > kv9-local/n$n.log 2>&1 &
done

# Wait until every node serves.
for n in 1 2 3; do until grep -q '^bootstrap_state=Serving' kv9-local/n$n/status 2>/dev/null; do sleep 0.5; done; done
```

What the steps do:

- **`store-prepare`** (called by `prepare_root_stores`) creates an empty, locked data directory and
  mints its **store incarnation**, a public identity the root descriptor records. On separate hosts run
  `kv9 store-prepare --node-id ID --data-dir PATH` on each host and pass the collected
  `ID=INCARNATION,...` map to `root-create` yourself. A lost voter's identity cannot be recreated on a
  new disk; it is replaced through membership (below).
- **`root-create`** writes the canonical root descriptor (cluster id, bootstrap generation, voters,
  incarnations). Copy the same file to every voter.
- **`init`** binds a prepared store to the root. **`start`** runs the node: the three voters elect a
  leader, the winner initializes the catalog, and every node reports `bootstrap_state=Serving`.
- `--addr` is the gRPC address clients and peers use (`--advertise-addr` if the bind address differs).

### The status file

Every node rewrites `<data-dir>/status` about once a second. The fields an operator reads first:

```
bootstrap_state=Serving     Joining until the node has registered; Serving once it participates
role=leader|follower|learner|candidate
leader_id=2                 the metadata leader this node knows (0: none yet)
meta_voters=1,2,3           committed voters of the metadata group
meta_learners=4             committed learners
applied_term=1 applied_index=42
fatal=                      non-empty means the process has stopped making progress; read the log
```

`<data-dir>/metrics.json` holds fixed-inventory latency histograms (`LATENCY-OBSERVABILITY.md`).

### Stop, restart, and add nodes

Stop a node with `kill` and restart it with the **same** `start` command and environment; the data
directory holds its Raft log, catalog WAL and identity. Never rerun `root-create` or `init` for an
existing store.

A new node joins as a learner and is promoted when it has caught up. `kv9-admin` (build it with
`cargo build -p kv9-admin`; any node's address will do, it finds the leader) admits the node and
prints a one-time join ticket; the new host provisions with it:

```bash
export KV9_ADMIN_TOKEN=client-secret
./target/debug/kv9-admin --endpoints 127.0.0.1:20161 node admit 4 --addr 127.0.0.1:20164 --ttl 600
#   → join_ticket: <hex>
# On the new host, with the root descriptor copied over:
KV9_JOIN_TICKET=<hex> ./target/debug/kv9 join  --root kv9-local/root.bin --node-id 4 --addr 127.0.0.1:20164 --data-dir kv9-local/n4
KV9_JOIN_TICKET=<hex> ./target/debug/kv9 start --node-id 4 --addr 127.0.0.1:20164 --data-dir kv9-local/n4 &
./target/debug/kv9-admin --endpoints 127.0.0.1:20161 node list            # 4 appears as a learner
./target/debug/kv9-admin --endpoints 127.0.0.1:20161 node promote 4 --wait
./target/debug/kv9-admin --endpoints 127.0.0.1:20161 node remove 1 --wait # refused below three voters
./target/debug/kv9-admin --endpoints 127.0.0.1:20161 cluster health
```

The same verbs exist as `kv9 client admit-node / promote-node / remove-node --addr <leader>` for
scripts that predate the tool. `KV9_ADMIN_PRINCIPALS=admin` in the nodes' environment limits the
administrative RPCs to that principal (the name before `=` in `KV9_CLIENT_TOKENS`).

With `KV9_AUTO_REPLACE_VOTER=1` in every node's environment the leader does the promote-then-remove
itself when a voter has been silent for `KV9_VOTER_LOST_SECS` (default 300) and a caught-up learner
exists ([VOTER-REPLACEMENT](VOTER-REPLACEMENT.md)). Without it, or with no learners, the cluster
survives one voter loss and waits for that voter to return.

## 3. First requests

The REPL follows leader hints through its peer map, so any node works as `--addr`:

```bash
cargo build -p kv9-repl
KV9_CLIENT_TOKEN=client-secret ./target/debug/kv9-repl --addr 127.0.0.1:20161 \
    --peers 1=127.0.0.1:20161,2=127.0.0.1:20162,3=127.0.0.1:20163
kv9> create app
keyspace app (id 100) created; selected
kv9[app]> put user/1 alice
applied (term=1, index=5)
kv9[app]> get user/1
alice
```

The `kv9 client` CLI is the reference client for scripts. Keys and values are hex, keyspaces are
named (`--keyspace-id <n>` is the explicit id form), and the command must be sent to the current
leader (it reports `not_leader=true leader_node_id=N` otherwise):

```bash
export KV9_CLIENT_TOKEN=client-secret
kv9_addr=127.0.0.1:20162
./target/debug/kv9 client create-keyspace --addr $kv9_addr --name demo --api-type raw   # → keyspace_id=101
./target/debug/kv9 client raw-put   --addr $kv9_addr --keyspace demo --key-hex 68656c6c6f --value-hex 776f726c64
./target/debug/kv9 client raw-get   --addr $kv9_addr --keyspace demo --key-hex 68656c6c6f     # value_hex=776f726c64
./target/debug/kv9 client raw-batch --addr $kv9_addr --keyspace demo --mutation put:6b31:7631 --mutation delete:68656c6c6f
./target/debug/kv9 client raw-scan  --addr $kv9_addr --keyspace demo --start-hex '' --end-hex '' --limit 10
./target/debug/kv9 client raw-delete-range --addr $kv9_addr --keyspace demo --start-hex '' --end-hex ''
```

A read that answers `read_unconfirmed=true` may be retried; a write that times out has an **unknown**
outcome, not a failed one. Applications should use an SDK (§5), which encodes these rules.

A keyspace created this way is served by the metadata group. Keyspaces on their own Raft group (a
**data group**, [GROUP-CONTROL](GROUP-CONTROL.md)) are created with `kv9-admin`:

```bash
./target/debug/kv9-admin --endpoints 127.0.0.1:20161 group create --voters 1,2,3 --label orders
#   → creation_task=<task> region_id=<region> (created; ...)
./target/debug/kv9-admin --endpoints 127.0.0.1:20161 keyspace create --name orders --group <task>
./target/debug/kv9-admin --endpoints 127.0.0.1:20161 group describe <task>   # wait for every replica to be active
./target/debug/kv9-admin --endpoints 127.0.0.1:20161 task list                # the committed history behind it
```

## 4. A persistent cluster on object storage

With `KV9_STORAGE=minio`, leaders upload SST checkpoints to a bucket, replicas adopt them, the covered
WAL is reclaimed, and a cold restart recovers from the bucket plus the surviving WAL tail. Configure the
same settings on every node:

| Variable | Meaning |
|---|---|
| `KV9_STORAGE` | `minio`; unset selects local WAL mode (§2) |
| `KV9_OBJECT_STORE_ENDPOINT` | for example `http://127.0.0.1:19450`; reachable from every node |
| `KV9_OBJECT_STORE_BUCKET` | an existing bucket |
| `KV9_OBJECT_STORE_ACCESS_KEY`, `KV9_OBJECT_STORE_SECRET_KEY` | credentials |
| `KV9_FLUSH_INTERVAL_MS` | optional, default `5000`, minimum `100`: dirty-state/adoption polling interval |

Without an existing service, start a local MinIO container and a bucket:

```bash
export KV9_STORAGE=minio
export KV9_OBJECT_STORE_ENDPOINT=http://127.0.0.1:19450
export KV9_OBJECT_STORE_BUCKET=kv9-local
export KV9_OBJECT_STORE_ACCESS_KEY="kv9$(python3 -c 'import secrets; print(secrets.token_hex(8))')"
export KV9_OBJECT_STORE_SECRET_KEY="$(python3 -c 'import secrets; print(secrets.token_hex(24))')"
export MINIO_ROOT_USER="$KV9_OBJECT_STORE_ACCESS_KEY" MINIO_ROOT_PASSWORD="$KV9_OBJECT_STORE_SECRET_KEY"
docker run -d --name kv9-local-minio -p 127.0.0.1:19450:9000 \
  -v kv9-local-minio:/data -e MINIO_ROOT_USER -e MINIO_ROOT_PASSWORD \
  quay.io/minio/minio@sha256:14cea493d9a34af32f524e538b8346cf79f3321eff8e708c1e2960462bd8936e server /data
until curl -sf http://127.0.0.1:19450/minio/health/live; do sleep 1; done
docker exec -e "MC_HOST_local=http://$KV9_OBJECT_STORE_ACCESS_KEY:$KV9_OBJECT_STORE_SECRET_KEY@127.0.0.1:9000" \
  kv9-local-minio mc mb --ignore-existing "local/$KV9_OBJECT_STORE_BUCKET"
```

Then provision and start the nodes exactly as in §2 (fresh data directories; the nodes inherit the
`KV9_*` environment). Keep the object-store environment and credentials: a data directory that holds a
remote checkpoint needs them to reopen.

What you will see in a node directory once checkpoints flow: `catalog.checkpoint` (the applied recovery
descriptor), `catalog.wal` (the surviving state-machine tail), `catalog.pending` (an unresolved flush,
cleared after settlement) and `raft/` (protocol state and history). A log line
`checkpoint generation=… through=… adopted` means that replica installed the checkpoint and reclaimed
the covered WAL.

Recovery limits: missing or corrupt objects, or a checkpoint inconsistent with committed Raft history,
refuse startup rather than serve. The original cluster identity and Raft logs are still required;
rebuilding a cluster from the bucket alone is not supported.

## 5. Clients and SDKs

SDKs for Python, Rust, C++ and TypeScript live in `sdk/` and implement one contract
([`sdk/README.md`](../sdk/README.md)): bearer-token auth, keyspace handle with region epoch and a
one-shot refresh, typed refusal markers from gRPC trailers, local bounds checks (key ≤ 4096 B, value ≤
64 KiB, 1..=256 items per batch, ≤ 1 MiB per message, ≤ 30 s per call) and `NotLeader` redirects
through a peer map. Each SDK's README has a complete example; the repository README shows Python and
Rust. `scripts/sdk-e2e.sh` boots a cluster and runs every SDK's smoke test and a scripted REPL session.

## 6. Limits

- The full dataset stays in memory on every replica; SSTs are checkpoints, not yet a read path.
- Range split and merge are not implemented; the metadata catalog is one Raft group.
- Automatic tenant admission and fairness are partial; TLS is not implemented, so run kv9 on a private
  network and treat the client token as the only credential.
- Writes not yet checkpointed depend on the replicas' local disks.

Public backend admission per node defaults to 64 reserved/queued/running requests and 64 MiB of encoded
request weight (`KV9_PUBLIC_MAX_REQUESTS`, `KV9_PUBLIC_MAX_ENCODED_BYTES`); saturation is a typed
pre-execution refusal ([PUBLIC-ADMISSION](PUBLIC-ADMISSION.md)). [ROADMAP](ROADMAP.md) orders the work
that removes these limits.

## 7. Acceptance scripts

These exercise the paths above end to end; the hosted CI runs them on manual dispatch ([TESTING](TESTING.md)):

| Script | What it proves |
| --- | --- |
| `scripts/quickstart-smoke.sh` | §2–§3 as written: three nodes, keyspace, raw requests |
| `scripts/raw-kv-e2e.sh` | raw KV through failover, deletes, restart catch-up (local WAL mode) |
| `scripts/minio-kv-e2e.sh` | three replicas on a real MinIO container: failover, checkpoint adoption, WAL reclamation, full restart, unflushed-tail recovery (`PASS: minio-kv-e2e …`) |
| `scripts/minio-pending-e2e.sh` | process crashes around pending-flush fsync and apply (builds the `checkpoint-testing` feature; rebuild afterwards) |
| `scripts/voter-replacement-e2e.sh` | automatic voter replacement while two of three original voters die in turn |
| `scripts/sdk-e2e.sh` | every SDK and the REPL against a fresh cluster |

`KV9_BASE_PORT` moves the ports the scripts use; `KV9_MINIO_EXTERNAL=1` with the §4 variables points
the MinIO scripts at an existing service.
