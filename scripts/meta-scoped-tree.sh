#!/usr/bin/env bash
# Per-region scopes at scale (docs/META-REPLICA-MODES.md S1).
#
# One keyspace, split into a balanced tree of KV9_TREE_LEAVES leaves (default 8, a power of
# two), each leaf moved onto its own triple of scoped nodes. Checked: every slice is current;
# a leaf's hosts hold exactly the leaf, its ancestors and the ancestors' other children
# (2 * depth + 1 regions), never the keyspace; data in every leaf serves through its hosts.
# A JSON summary is written to $artifact_dir/tree-summary.json.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/root_provision.sh"

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
artifact_dir="${KV9_TREE_DIR:-$(mktemp -d /tmp/kv9-scoped-tree.XXXXXX)}"
leaves="${KV9_TREE_LEAVES:-8}"
depth=0; while (( (1 << depth) < leaves )); do depth=$((depth + 1)); done
(( (1 << depth) == leaves )) || { echo "KV9_TREE_LEAVES must be a power of two" >&2; exit 2; }
# Below the kernel ephemeral range (32768+).
base_port="${KV9_BASE_PORT:-$((23000 + ($$ % 500)))}"
cluster_token="tree-cluster-token"; admin_token="tree-admin-token"; app_token="tree-app-token"
bootstrap_token="tree-bootstrap-token"
root_path="$artifact_dir/root.bin"
root_voters="1@127.0.0.1:$((base_port + 1)),2@127.0.0.1:$((base_port + 2)),3@127.0.0.1:$((base_port + 3))"
scoped=$((3 + 3 * leaves)); last=$((3 + scoped))
minio_port=$((base_port + 900)); minio_name="kv9-scoped-tree-minio-$base_port"
export KV9_OBJECT_STORE_ENDPOINT="http://127.0.0.1:$minio_port" KV9_OBJECT_STORE_BUCKET=kv9-scoped-tree
export KV9_OBJECT_STORE_ACCESS_KEY=kv9tree KV9_OBJECT_STORE_SECRET_KEY=kv9tree-secret-key
declare -A pids=()
node_environment=(KV9_CLUSTER_TOKEN="$cluster_token" KV9_CLIENT_TOKENS="admin=$admin_token,app=$app_token"
  KV9_ADMIN_PRINCIPALS=admin KV9_SCOPE_IDLE_MS=1000 KV9_STORAGE=minio
  KV9_DRIVER_BACKOFF_PCT="${KV9_E2E_BACKOFF_PCT:-10}")
profile=(); [[ "${KV9_E2E_RELEASE:-0}" == 1 ]] && profile=(--release)

status_value() {
  local file="$artifact_dir/n${1}/status"
  test -f "$file" || return 1
  awk -F= -v wanted="$2" '$1 == wanted { print substr($0, length($1) + 2); found=1 } END { if (!found) exit 1 }' "$file"
}
endpoint_of() { echo "127.0.0.1:$((base_port + $1))"; }
stop_all() {
  local pid
  for pid in "${pids[@]}"; do kill -0 "$pid" 2>/dev/null && kill "$pid" 2>/dev/null || true; done
  wait 2>/dev/null || true
  docker rm -f "$minio_name" >/dev/null 2>&1 || true
}
trap stop_all EXIT
wait_until() {
  local label="$1" timeout="$2"; shift 2
  local deadline=$((SECONDS + timeout))
  while (( SECONDS < deadline )); do "$@" && return 0; sleep 1; done
  echo "FAIL: timed out waiting for $label" >&2; exit 1
}
serving() {
  local node
  for node in "$@"; do
    test "$(status_value "$node" pid 2>/dev/null || true)" = "${pids[$node]:-none}" || return 1
    test "$(status_value "$node" bootstrap_state 2>/dev/null || true)" = "Serving" || return 1
  done
}
voters_up() { serving 1 2 3 && [[ "$(status_value 1 leader_id)" =~ ^[123]$ ]]; }
any_voter() { local n; for n in 1 2 3; do [[ -n "${pids[$n]:-}" ]] && { endpoint_of "$n"; return; }; done; }
admin() { KV9_ADMIN_TOKEN="$admin_token" timeout 60 "$admin_bin" --endpoints "$(any_voter)" --output json --yes "$@" 2>/dev/null; }
jq_py() { python3 -c 'import json,sys; d=json.load(sys.stdin); print(eval(sys.argv[1], {"d": d}))' "$1"; }
all_current() {
  local node
  for node in $(seq 4 "$last"); do
    test "$(status_value "$node" catalog_scope 2>/dev/null || true)" = "current" || return 1
  done
}

echo "Starting MinIO (splits and moves need object storage)..."
docker rm -f "$minio_name" >/dev/null 2>&1 || true
docker run -d --name "$minio_name" -p "127.0.0.1:$minio_port:9000" -e MINIO_ROOT_USER="$KV9_OBJECT_STORE_ACCESS_KEY" \
  -e MINIO_ROOT_PASSWORD="$KV9_OBJECT_STORE_SECRET_KEY" quay.io/minio/minio:latest server /data >/dev/null
for _ in $(seq 120); do curl -sf "$KV9_OBJECT_STORE_ENDPOINT/minio/health/live" >/dev/null && break; sleep 0.5; done
docker exec -e "MC_HOST_d=http://$KV9_OBJECT_STORE_ACCESS_KEY:$KV9_OBJECT_STORE_SECRET_KEY@127.0.0.1:9000" "$minio_name" \
  mc mb --ignore-existing "d/$KV9_OBJECT_STORE_BUCKET" >/dev/null
echo "Building..."
bin="$(build_fixture_binary "$repo_dir" "$artifact_dir/build.jsonl" "${profile[@]}")"
cargo build --locked --manifest-path "$repo_dir/Cargo.toml" "${profile[@]}" -p kv9-admin -p kv9-repl >"$artifact_dir/build-admin.log" 2>&1
admin_bin="$(dirname "$bin")/kv9-admin"; repl_bin="$(dirname "$bin")/kv9-repl"
KV9_BOOTSTRAP_TOKEN="$bootstrap_token" "$bin" root-create --output "$root_path" \
  --voters "$root_voters" --store-incarnations "$(prepare_root_stores "$bin" "$artifact_dir" "$root_voters")" >"$artifact_dir/root-create.log"
for node in 1 2 3; do
  KV9_BOOTSTRAP_TOKEN="$bootstrap_token" "$bin" init --root "$root_path" --node-id "$node" --data-dir "$artifact_dir/n$node" >>"$artifact_dir/n$node.log" 2>&1
  env "${node_environment[@]}" "$bin" start --node-id "$node" --addr "$(endpoint_of "$node")" --data-dir "$artifact_dir/n$node" >>"$artifact_dir/n$node.log" 2>&1 &
  pids[$node]=$!
done
wait_until "three voters Serving" 60 voters_up
for node in $(seq 4 "$last"); do
  ticket=""
  for _ in 1 2 3 4 5 6; do
    ticket="$(admin node admit "$node" --addr "$(endpoint_of "$node")" --ttl 900 --meta scoped | jq_py 'd["result"]["join_ticket"]' 2>/dev/null)" && break
    sleep 2
  done
  mkdir -p "$artifact_dir/n$node"
  KV9_JOIN_TICKET="$ticket" "$bin" join --root "$root_path" --node-id "$node" --addr "$(endpoint_of "$node")" \
    --data-dir "$artifact_dir/n$node" >"$artifact_dir/n$node.join" 2>&1
  env "${node_environment[@]}" KV9_JOIN_TICKET="$ticket" "$bin" start --node-id "$node" --addr "$(endpoint_of "$node")" \
    --data-dir "$artifact_dir/n$node" >>"$artifact_dir/n$node.log" 2>&1 &
  pids[$node]=$!
done
wait_until "$scoped scoped nodes Serving" 300 serving $(seq 4 "$last")
peers="$(for n in $(seq 1 "$last"); do printf '%s=%s,' "$n" "$(endpoint_of "$n")"; done)"; peers="${peers%,}"

# One keyspace on nodes 4-6, one key per future leaf.
root_task="$(admin group create --voters 4,5,6 --label tree-root | jq_py 'd["result"]["creation_task"]')"
for _ in $(seq 60); do
  admin keyspace create --name tree --group "$root_task" >"$artifact_dir/bind.json" || true
  ks="$(jq_py 'd["result"]["id"]' <"$artifact_dir/bind.json" 2>/dev/null)" && break
  sleep 1
done
key_of() { printf 'k/%03d' "$1"; }
seed() {
  local lines="use tree" i
  for i in $(seq 0 $((leaves - 1))); do lines="$lines
put $(key_of "$i") leaf-$i"; done
  printf '%s\n' "$lines" | KV9_CLIENT_TOKEN="$app_token" timeout 120 "$repl_bin" --addr "$(endpoint_of 4)" --peers "$peers" >"$artifact_dir/seed.out" 2>&1 \
    && test "$(grep -c '^applied' "$artifact_dir/seed.out")" = "$leaves"
}
wait_until "one key per future leaf written" 120 seed

live_groups() {
  admin group list | python3 -c '
import json,sys; ks=int(sys.argv[1]); gs=json.load(sys.stdin)["result"]
live=sorted((min(k["start"] for k in g["keyspaces"] if k["keyspace_id"]==ks), g["creation_task"]) for g in gs if any(k["keyspace_id"]==ks and not k["sealed"] for k in g["keyspaces"]))
print(" ".join(str(t) for _, t in live))' "$ks"
}
# Balanced splits, one level at a time: at level l every live group [i, i + width) splits at its middle key.
width=$leaves
for level in $(seq 1 "$depth"); do
  half=$((width / 2)); i=0; declare -a pending=()
  for task in $(live_groups); do
    out="$(admin group split "$task" --at "$(key_of $((i + half)))" --label "tree-$level-$i")"
    pending+=("$task:$(echo "$out" | jq_py 'd["result"]["intent_task"]')")
    i=$((i + width))
  done
  for p in "${pending[@]}"; do
    task="${p%%:*}"; stask="${p##*:}"
    done_split() { admin group describe "$task" | jq_py "[x['stage'] for x in d['result']['splits'] if x['task'] == $stask]" | grep -qx "\\['published'\\]"; }
    wait_until "split $stask (level $level) published" 900 done_split
  done
  unset pending
  width=$half
done
read -ra leaf_tasks <<<"$(live_groups)"
(( ${#leaf_tasks[@]} == leaves )) || { echo "FAIL: ${#leaf_tasks[@]} leaves, want $leaves" >&2; exit 1; }
echo "keyspace $ks split into $leaves leaves (depth $depth)."

# Move leaf i from nodes 4,5,6 onto its own triple 7+3i .. 9+3i, one replica per round, leaves in parallel.
group_voters() {
  admin group describe "$1" | jq_py 'next(iter([sorted(v["observed"]["voters"]) for v in d["result"]["replica_views"] if (v.get("observed") or {}).get("role") == "Leader" and v["observed"].get("state") == "active"]), None)'
}
for round in 0 1 2; do
  from=$((4 + round))
  for i in "${!leaf_tasks[@]}"; do
    to=$((7 + 3 * i + round))
    for _ in 1 2 3 4 5; do
      admin group move-replica "${leaf_tasks[$i]}" --from "$from" --to "$to" --label "tree-move-$i-$round" >/dev/null && break
      sleep 2
    done
  done
  round_done() {
    local i to
    for i in "${!leaf_tasks[@]}"; do
      to=$((7 + 3 * i + round))
      group_voters "${leaf_tasks[$i]}" | grep -q "\\b$to\\b" || return 1
      group_voters "${leaf_tasks[$i]}" | grep -q "\\b$from\\b" && return 1
    done
    return 0
  }
  wait_until "round $round of moves (off node $from)" 1200 round_done
done
wait_until "every slice current" 120 all_current
sleep 31   # full nodes recount their catalogs every 30 s

reads() {
  local i node
  for i in "${!leaf_tasks[@]}"; do
    node=$((7 + 3 * i))
    printf 'use tree\nget %s\n' "$(key_of "$i")" | KV9_CLIENT_TOKEN="$app_token" timeout 30 "$repl_bin" --addr "$(endpoint_of "$node")" \
      --peers "$peers" >"$artifact_dir/read-$i.out" 2>&1 && grep -qx "leaf-$i" "$artifact_dir/read-$i.out" || return 1
  done
}
wait_until "every leaf's key serves through its own hosts" 120 reads

admin group list >"$artifact_dir/groups.json"
python3 - "$artifact_dir" "$ks" "$leaves" "$depth" "${leaf_tasks[*]}" <<'PY'
import json, sys, os
d, ks, leaves, depth, leaf_tasks = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4]), sys.argv[5].split()
def status(n):
    out = {}
    for line in open(os.path.join(d, f"n{n}", "status")):
        k, _, v = line.rstrip("\n").partition("=")
        out[k] = v
    return out
groups = json.load(open(os.path.join(d, "groups.json")))["result"]
regions_in_keyspace = sorted({g["region_id"] for g in groups if any(k["keyspace_id"] == ks for k in g["keyspaces"])})
full = status(1)
expect = 2 * depth + 1
rows, regs = {}, {}
for i in range(leaves):
    for n in range(7 + 3 * i, 10 + 3 * i):
        s = status(n)
        regs[n] = int(s["catalog_scope_regions"])
        rows[n] = int(s["catalog_scope_rows"])
summary = {
    "leaves": leaves, "depth": depth, "regions_in_keyspace": len(regions_in_keyspace),
    "full_catalog_keys": int(full.get("catalog_keys", 0)),
    "leaf_host_regions": sorted(set(regs.values())), "expected_leaf_host_regions": expect,
    "leaf_host_rows_min": min(rows.values()), "leaf_host_rows_max": max(rows.values()),
    "root_host_regions": int(status(4)["catalog_scope_regions"]),
}
json.dump(summary, open(os.path.join(d, "tree-summary.json"), "w"), indent=2)
print(json.dumps(summary))
assert set(regs.values()) == {expect}, f"leaf hosts hold {sorted(set(regs.values()))} regions, want {expect}"
assert expect < len(regions_in_keyspace)
PY
echo "PASS: per-region scopes ($leaves leaves on disjoint scoped triples; $artifact_dir/tree-summary.json)"
