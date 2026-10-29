#!/usr/bin/env bash
# Replica addition/move acceptance on a real MinIO container (docs/REPLICA-ADDITION.md).
# KV9_MINIO_EXTERNAL=1 with the KV9_OBJECT_STORE_* variables uses an existing service.
set -euo pipefail
repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
artifact_dir="${KV9_ADDITION_E2E_DIR:-$(mktemp -d /tmp/kv9-replica-addition.XXXXXX)}"
base_port="${KV9_BASE_PORT:-$((29000 + ($$ % 500)))}"
minio_port="${KV9_MINIO_PORT:-$((19600 + ($$ % 300)))}"
container=""
cleanup() { if [[ -n "$container" ]]; then docker rm -f "$container" >/dev/null 2>&1 || true; fi; }
trap cleanup EXIT

cd "$repo_dir"
cargo build --locked -p kv9 -p kv9-admin -p kv9-repl --bin kv9 --bin kv9-admin --bin kv9-repl >"$artifact_dir/build.log" 2>&1
if [[ "${KV9_MINIO_EXTERNAL:-0}" != "1" ]]; then
  export KV9_OBJECT_STORE_ENDPOINT="http://127.0.0.1:$minio_port"
  export KV9_OBJECT_STORE_BUCKET="kv9-addition-e2e"
  export KV9_OBJECT_STORE_ACCESS_KEY="kv9$(python3 -c 'import secrets; print(secrets.token_hex(8))')"
  export KV9_OBJECT_STORE_SECRET_KEY="$(python3 -c 'import secrets; print(secrets.token_hex(24))')"
  container="kv9-addition-minio-$$"
  printf 'MINIO_ROOT_USER=%s\nMINIO_ROOT_PASSWORD=%s\n' "$KV9_OBJECT_STORE_ACCESS_KEY" "$KV9_OBJECT_STORE_SECRET_KEY" >"$artifact_dir/minio.env"
  docker run -d --name "$container" -p "127.0.0.1:$minio_port:9000" --env-file "$artifact_dir/minio.env" \
    quay.io/minio/minio@sha256:14cea493d9a34af32f524e538b8346cf79f3321eff8e708c1e2960462bd8936e server /data >/dev/null
  for _ in $(seq 1 60); do curl -sf "$KV9_OBJECT_STORE_ENDPOINT/minio/health/live" >/dev/null && break; sleep 0.5; done
  docker exec -e "MC_HOST_e2e=http://$KV9_OBJECT_STORE_ACCESS_KEY:$KV9_OBJECT_STORE_SECRET_KEY@127.0.0.1:9000" \
    "$container" mc mb --ignore-existing "e2e/$KV9_OBJECT_STORE_BUCKET" >/dev/null
fi
python3 scripts/replica-addition-e2e.py --bin target/debug/kv9 --admin-bin target/debug/kv9-admin \
  --repl-bin target/debug/kv9-repl --output "$artifact_dir/run" --base-port "$base_port"
echo "Artifacts: $artifact_dir"
