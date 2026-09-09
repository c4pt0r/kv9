# Run the basic distributed KV

Run from the repository root. Prerequisites: Rust stable, working `protoc`, a C/C++ toolchain, cmake and Python 3.
Real MinIO acceptance also requires Docker. Linux/WSL is the current test environment.

## One-command three-replica MinIO acceptance

```bash
./scripts/minio-kv-e2e.sh
```

The script builds the binary, starts a digest-pinned MinIO container, creates a temporary bucket and runs three
independent kv9 processes. It checks writes/deletes, failover, checkpoint adoption, catalog WAL reclamation,
full restart and unflushed-tail recovery. It cleans up its own processes and container and retains the directory
printed as `Artifacts:`. Default node ports are 22401-22403; MinIO uses 19450.

Set `KV9_BASE_PORT` to change node ports. To use an existing MinIO service, configure the variables below and set
`KV9_MINIO_EXTERNAL=1`; the script then does not manage the service or bucket.

The exclusive completion marker is:

```text
PASS: minio-kv-e2e (3 replicas, failover, remote checkpoint, reclaimed WAL, live tail, deletes)
```

## Exercise pending-flush process crashes

```bash
./scripts/minio-pending-e2e.sh
```

This explicitly builds the `checkpoint-testing` feature. It pauses after pending-file fsync but before proposal,
and after successful apply but before pending cleanup, then kills and restarts the process. While that replica is
down, the majority advances manifests multiple generations. Recovery must retain and settle the original identity.
Corrupt pending files and terms inconsistent with committed Raft history must refuse startup.

These gates are compiled out of the default binary. After the test, run `cargo build --workspace` to restore the
ordinary development binary.

## Run a persistent cluster

Prepare a readable/writable MinIO bucket and configure the same settings on all three nodes:

| Variable | Meaning |
|---|---|
| `KV9_STORAGE` | `minio`; unset selects local WAL mode |
| `KV9_OBJECT_STORE_ENDPOINT` | For example `http://127.0.0.1:19450`; remote nodes need an address reachable from their hosts |
| `KV9_OBJECT_STORE_BUCKET` | Existing bucket name |
| `KV9_OBJECT_STORE_ACCESS_KEY` | MinIO access key |
| `KV9_OBJECT_STORE_SECRET_KEY` | MinIO secret key |
| `KV9_FLUSH_INTERVAL_MS` | Optional, default `5000`, minimum `100`; dirty-state/adoption polling interval |

Without an existing service, start a local persistent container:

```bash
export KV9_STORAGE=minio
export KV9_OBJECT_STORE_ENDPOINT=http://127.0.0.1:19450
export KV9_OBJECT_STORE_BUCKET=kv9-local
export KV9_OBJECT_STORE_ACCESS_KEY="kv9$(python3 -c 'import secrets; print(secrets.token_hex(8))')"
export KV9_OBJECT_STORE_SECRET_KEY="$(python3 -c 'import secrets; print(secrets.token_hex(24))')"
export MINIO_ROOT_USER="$KV9_OBJECT_STORE_ACCESS_KEY"
export MINIO_ROOT_PASSWORD="$KV9_OBJECT_STORE_SECRET_KEY"
docker run -d --name kv9-local-minio -p 127.0.0.1:19450:9000 \
  -v kv9-local-minio:/data -e MINIO_ROOT_USER -e MINIO_ROOT_PASSWORD \
  quay.io/minio/minio@sha256:14cea493d9a34af32f524e538b8346cf79f3321eff8e708c1e2960462bd8936e server /data
# Wait for HTTP 200 before creating the bucket.
curl --fail http://127.0.0.1:19450/minio/health/live
docker exec -e "MC_HOST_local=http://$KV9_OBJECT_STORE_ACCESS_KEY:$KV9_OBJECT_STORE_SECRET_KEY@127.0.0.1:9000" \
  kv9-local-minio mc mb --ignore-existing "local/$KV9_OBJECT_STORE_BUCKET"
```

Keep the object-store environment for restart. Reuse the original MinIO credentials with an existing volume.

The following Bash commands create a local three-node cluster. Nodes inherit the MinIO settings.
Use separate, uninitialized data directories. Each directory must be prepared on its actual disk with
`kv9 store-prepare --node-id ID --data-dir PATH` before creating the root. The helper below prepares
three local directories and collects their public incarnation identifiers. For different hosts, run
that command on each host, then pass the collected `ID=INCARNATION,...` map to `root-create`.
The descriptor records these identities; it cannot recreate a lost voter's identity on a new disk.
The generated authentication file has mode 600 and must be retained.

```bash
cargo build --workspace
umask 077
mkdir -p ./kv9-local
python3 - <<'PY' > ./kv9-local/auth.env
import secrets
for name in ('KV9_BOOTSTRAP_TOKEN', 'KV9_CLUSTER_TOKEN', 'KV9_CLIENT_TOKEN'):
    print('export '+name+'='+secrets.token_hex(24))
print('export KV9_CLIENT_TOKENS="admin=$KV9_CLIENT_TOKEN"')
PY
source ./kv9-local/auth.env
source scripts/root_provision.sh
kv9_voters=1@127.0.0.1:22401,2@127.0.0.1:22402,3@127.0.0.1:22403
kv9_prepared="$(prepare_root_stores ./target/debug/kv9 ./kv9-local "$kv9_voters")"
./target/debug/kv9 root-create --output ./kv9-local/root.bin \
  --voters "$kv9_voters" --store-incarnations "$kv9_prepared"
for n in 1 2 3; do
  ./target/debug/kv9 init --root ./kv9-local/root.bin --node-id "$n" --data-dir "./kv9-local/n$n"
  ./target/debug/kv9 start --node-id "$n" --addr "127.0.0.1:$((22400+n))" \
    --data-dir "./kv9-local/n$n" > "./kv9-local/n$n.log" 2>&1 &
  echo "$!" > "./kv9-local/n$n.pid"
done
```

Wait until all three `kv9-local/n*/status` files report `bootstrap_state=Serving`, agree on `leader_id`, and have
an empty `fatal=` value. Send requests to that leader. This example assumes node 1 is leader; after failover,
use `not_leader=true` and `leader_node_id` responses to choose the new address.

```bash
kv9_addr=127.0.0.1:22401
./target/debug/kv9 client create-keyspace --addr "$kv9_addr" --name demo --api-type raw
# Replace this ID with the keyspace_id returned by create-keyspace.
kv9_keyspace=100
./target/debug/kv9 client raw-put --addr "$kv9_addr" --keyspace "$kv9_keyspace" --key-hex 68656c6c6f --value-hex 776f726c64
./target/debug/kv9 client raw-get --addr "$kv9_addr" --keyspace "$kv9_keyspace" --key-hex 68656c6c6f
./target/debug/kv9 client raw-scan --addr "$kv9_addr" --keyspace "$kv9_keyspace" --start-hex '' --end-hex '' --limit 10
./target/debug/kv9 client raw-delete --addr "$kv9_addr" --keyspace "$kv9_keyspace" --key-hex 68656c6c6f
```

Reading `hello` returns `value_hex=776f726c64`; after deletion it returns `found=false`.
The CLI does not automatically follow hints. Reads reporting `read_unconfirmed=true` may be retried.
An ordinary write timeout means the result is unknown, not that the write definitely failed.

A `checkpoint generation=... through=... adopted` log means that replica installed the checkpoint and completed
catalog WAL reclamation. In each node directory:

- `catalog.pending` retains an unresolved flush identity and is cleared only after authoritative settlement.
- `catalog.checkpoint` points to an applied recovery descriptor.
- `catalog.wal` contains the surviving state-machine tail.
- `raft/` still contains protocol state and Raft history.

Stop the processes you started and restart with the original environment and identical `start` commands.
Do not rerun `root-create` or `init`, and preserve every durable data file.

A directory with a remote checkpoint needs MinIO configuration to reopen. Missing/corrupt objects or a checkpoint
inconsistent with committed history refuse startup. Original cluster identity and Raft logs are still required;
bucket-only cluster reconstruction is not supported.

## Implementation limits

All data currently shares one Raft group and remains in memory. Checkpoints are full-state with a 48 MiB serialized
limit; hitting it defers checkpointing. Automatic admission/backpressure is not implemented, so this is not a
production-capacity guarantee. SSTs and historical manifests are retained; Raft logs are not truncated.
Transactions, split/merge, TTL and block cache are future work.

[ROADMAP.md](ROADMAP.md) orders the work needed to remove these limits.

## Public backend limits

Each node defaults to 64 reserved/queued/running public backend requests and a
64 MiB sum of encoded request weights. Set `KV9_PUBLIC_MAX_REQUESTS` and
`KV9_PUBLIC_MAX_ENCODED_BYTES` before startup to change these positive integer
limits. Saturation returns an explicit pre-execution refusal; an in-flight
cancelled backend job retains its reservation until it finishes. The status file
exports fixed admission counters and gauges. See [PUBLIC-ADMISSION.md](PUBLIC-ADMISSION.md)
for units, protocol details and the separate transport/response memory obligations.
