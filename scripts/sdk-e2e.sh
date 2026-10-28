#!/usr/bin/env bash
# Boots a real three-process kv9 cluster, creates a raw keyspace, and runs every SDK's
# smoke test against it (Python, Rust, C++, TypeScript) plus a scripted kv9-repl session. Each smoke reads KV9_ADDR,
# KV9_TOKEN, KV9_KEYSPACE and KV9_PEERS; this script sets them.
#
#   scripts/sdk-e2e.sh                 # all SDKs that have a toolchain installed
#   KV9_SDKS="python rust" scripts/sdk-e2e.sh
#   KV9_PYTHON=/path/to/venv/bin/python scripts/sdk-e2e.sh   # a python with grpcio + pytest
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/root_provision.sh"

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
artifact_dir="${KV9_SDK_E2E_DIR:-$(mktemp -d /tmp/kv9-sdk-e2e.XXXXXX)}"
base_port="${KV9_BASE_PORT:-$((24000 + ($$ % 1000)))}"
cluster_token="sdk-e2e-cluster-token"
client_token="sdk-e2e-client-token"
bootstrap_token="sdk-e2e-bootstrap-token"
root_path="$artifact_dir/root.bin"
root_voters="1@127.0.0.1:$((base_port + 1)),2@127.0.0.1:$((base_port + 2)),3@127.0.0.1:$((base_port + 3))"
sdks="${KV9_SDKS:-python rust cpp typescript repl}"
python_bin="${KV9_PYTHON:-python3}"
declare -A pids=()

status_value() {
  local node="$1" key="$2" file="$artifact_dir/n${1}/status"
  test -f "$file" || return 1
  awk -F= -v wanted="$key" '$1 == wanted { print substr($0, length($1) + 2); found=1 } END { if (!found) exit 1 }' "$file"
}

start_node() {
  local node="$1"
  mkdir -p "$artifact_dir/n${node}"
  if [[ ! -e "$artifact_dir/n${node}/kv9-store-identity" ]]; then
    KV9_BOOTSTRAP_TOKEN="$bootstrap_token" "$bin" init --root "$root_path" --node-id "$node" \
      --data-dir "$artifact_dir/n${node}" >>"$artifact_dir/n${node}.log" 2>&1
  fi
  KV9_CLUSTER_TOKEN="$cluster_token" KV9_CLIENT_TOKENS="sdk=$client_token" "$bin" start \
    --node-id "$node" --addr "127.0.0.1:$((base_port + node))" --data-dir "$artifact_dir/n${node}" \
    >>"$artifact_dir/n${node}.log" 2>&1 &
  pids[$node]=$!
}

stop_all() {
  local pid
  for pid in "${pids[@]}"; do kill -0 "$pid" 2>/dev/null && kill "$pid" 2>/dev/null || true; done
  wait 2>/dev/null || true
}
trap stop_all EXIT

all_serving() {
  local node leaders=0 leader_id=0 seen
  for node in 1 2 3; do
    test "$(status_value "$node" bootstrap_state 2>/dev/null || true)" = "Serving" || return 1
    test -z "$(status_value "$node" fatal 2>/dev/null || true)" || return 1
    seen="$(status_value "$node" leader_id)"
    test "$seen" -gt 0 || return 1
    (( leader_id == 0 )) && leader_id="$seen"
    test "$seen" -eq "$leader_id" || return 1
    test "$(status_value "$node" role)" = leader && leaders=$((leaders + 1))
  done
  test "$leaders" -eq 1 || return 1
  agreed_leader="$leader_id"
}

echo "Building kv9..."
bin="$(build_fixture_binary "$repo_dir" "$artifact_dir/build.jsonl")"
KV9_BOOTSTRAP_TOKEN="$bootstrap_token" "$bin" root-create --output "$root_path" \
  --voters "$root_voters" --store-incarnations "$(prepare_root_stores "$bin" "$artifact_dir" "$root_voters")" \
  >"$artifact_dir/root-create.log"

echo "Starting three nodes on ports $((base_port + 1))-$((base_port + 3))..."
for node in 1 2 3; do start_node "$node"; done
agreed_leader=""
deadline=$((SECONDS + 30))
until all_serving; do
  (( SECONDS < deadline )) || { echo "FAIL: cluster did not reach Serving" >&2; exit 1; }
  sleep 0.1
done
leader_addr="127.0.0.1:$((base_port + agreed_leader))"
echo "Leader is n${agreed_leader} at ${leader_addr}."

# The keyspace comes from the kv9 CLI so the SDKs are measured against the reference client.
create_output="$(KV9_CLIENT_TOKEN="$client_token" "$bin" client create-keyspace --addr "$leader_addr" \
  --name sdk-e2e --api-type raw 2>&1)" || { echo "$create_output" >&2; exit 1; }
keyspace="$(awk -F= '$1 == "keyspace_id" { print $2 }' <<<"$create_output")"
test -n "$keyspace" || { echo "FAIL: no keyspace_id in: $create_output" >&2; exit 1; }
echo "Created raw keyspace ${keyspace}."

export KV9_ADDR="127.0.0.1:$((base_port + 1))"   # deliberately node 1: the SDKs must follow the leader hint
export KV9_TOKEN="$client_token"
export KV9_KEYSPACE="$keyspace"
export KV9_PEERS="1=127.0.0.1:$((base_port + 1)),2=127.0.0.1:$((base_port + 2)),3=127.0.0.1:$((base_port + 3))"

failed=()
run_sdk() {
  local name="$1"; shift
  echo "== $name"
  if "$@"; then echo "== $name: PASS"; else echo "== $name: FAIL" >&2; failed+=("$name"); fi
}

for sdk in $sdks; do
  case "$sdk" in
    python)
      if "$python_bin" -c "import grpc, pytest" 2>/dev/null; then
        run_sdk python env PYTHONPATH="$repo_dir/sdk/python" "$python_bin" -m pytest -q "$repo_dir/sdk/python/tests/test_smoke.py"
      else
        echo "== python: SKIP ($python_bin lacks grpcio/pytest; set KV9_PYTHON)"
      fi ;;
    rust)
      run_sdk rust cargo run -q --manifest-path "$repo_dir/Cargo.toml" -p kv9-client --example smoke ;;
    repl)
      # Scripted mode: one command per line on stdin; a non-zero exit means a command failed.
      run_sdk repl bash -c "printf '%s\n' 'use $keyspace' 'put repl/a 1' 'batch put repl/b 2, del repl/a' \
          'get repl/b' 'mget repl/a repl/b' 'scan repl/ repl0' 'delrange repl/ repl0' 'info' \
        | cargo run -q --manifest-path '$repo_dir/Cargo.toml' -p kv9-repl -- --addr \"\$KV9_ADDR\" --peers \"\$KV9_PEERS\" --token \"\$KV9_TOKEN\"" ;;
    cpp)
      if command -v cmake >/dev/null && pkg-config --exists grpc++ 2>/dev/null; then
        run_sdk cpp bash -c "cmake -S '$repo_dir/sdk/cpp' -B '$repo_dir/sdk/cpp/build' -DKV9_BUILD_TESTS=OFF >/dev/null && cmake --build '$repo_dir/sdk/cpp/build' -j >/dev/null && '$repo_dir/sdk/cpp/build/kv9_smoke'"
      else
        echo "== cpp: SKIP (cmake or grpc++ missing)"
      fi ;;
    typescript)
      if command -v npm >/dev/null; then
        run_sdk typescript bash -c "cd '$repo_dir/sdk/typescript' && { test -d node_modules || npm install --no-audit --no-fund >/dev/null; } && npm run -s smoke"
      else
        echo "== typescript: SKIP (npm missing)"
      fi ;;
    *) echo "unknown SDK '$sdk'" >&2; exit 2 ;;
  esac
done

if (( ${#failed[@]} )); then
  echo "FAIL: ${failed[*]} (artifacts in $artifact_dir)" >&2
  exit 1
fi
echo "PASS: sdk-e2e ($sdks) against leader n${agreed_leader}; artifacts in $artifact_dir"
