#!/usr/bin/env bash
# Run a command against a throwaway MinIO container: exports the KV9_OBJECT_STORE_*
# variables (and KV9_STORAGE=minio), creates the bucket, runs "$@", removes the
# container. KV9_MINIO_EXTERNAL=1 skips the container and uses the variables as set.
#   scripts/with-minio.sh python3 scripts/removal-e2e.py --bin target/debug/kv9 --output /tmp/out
set -euo pipefail
container=""
cleanup() { if [[ -n "$container" ]]; then docker rm -f "$container" >/dev/null 2>&1 || true; fi; }
trap cleanup EXIT
if [[ "${KV9_MINIO_EXTERNAL:-0}" != "1" ]]; then
  port="${KV9_MINIO_PORT:-$((19700 + ($$ % 200)))}"
  export KV9_OBJECT_STORE_ENDPOINT="http://127.0.0.1:$port"
  export KV9_OBJECT_STORE_BUCKET="${KV9_OBJECT_STORE_BUCKET:-kv9-e2e}"
  export KV9_OBJECT_STORE_ACCESS_KEY="kv9$(python3 -c 'import secrets; print(secrets.token_hex(8))')"
  export KV9_OBJECT_STORE_SECRET_KEY="$(python3 -c 'import secrets; print(secrets.token_hex(24))')"
  container="kv9-minio-$$"
  env_file="$(mktemp)"
  printf 'MINIO_ROOT_USER=%s\nMINIO_ROOT_PASSWORD=%s\n' "$KV9_OBJECT_STORE_ACCESS_KEY" "$KV9_OBJECT_STORE_SECRET_KEY" >"$env_file"
  docker run -d --name "$container" -p "127.0.0.1:$port:9000" --env-file "$env_file" \
    quay.io/minio/minio@sha256:14cea493d9a34af32f524e538b8346cf79f3321eff8e708c1e2960462bd8936e server /data >/dev/null
  rm -f "$env_file"
  for _ in $(seq 1 60); do curl -sf "$KV9_OBJECT_STORE_ENDPOINT/minio/health/live" >/dev/null && break; sleep 0.5; done
  docker exec -e "MC_HOST_e2e=http://$KV9_OBJECT_STORE_ACCESS_KEY:$KV9_OBJECT_STORE_SECRET_KEY@127.0.0.1:9000" \
    "$container" mc mb --ignore-existing "e2e/$KV9_OBJECT_STORE_BUCKET" >/dev/null
fi
export KV9_STORAGE=minio
"$@"
