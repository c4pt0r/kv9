#!/usr/bin/env bash
# The scoped metadata mode end to end (docs/META-REPLICA-MODES.md).
#
# Three metadata voters. Nodes 4-6 are admitted with `--meta scoped`: they join
# without becoming metadata Raft members and keep only their catalog slice,
# streamed from a full node. Checked: the scoped nodes Serve with an empty scope;
# a data group on the voters stays out of their slices; a data group created on
# the scoped nodes alone becomes active, binds a keyspace and serves raw traffic
# through a scoped node; each slice holds that keyspace and not the other; a
# scoped node restarts from its slice; a stopped full node's subscribers move to
# another full node.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/root_provision.sh"

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
artifact_dir="${KV9_SCOPED_E2E_DIR:-$(mktemp -d /tmp/kv9-scoped-e2e.XXXXXX)}"
base_port="${KV9_BASE_PORT:-$((29000 + ($$ % 1000)))}"
cluster_token="scoped-e2e-cluster-token"
admin_token="scoped-e2e-admin-token"
app_token="scoped-e2e-app-token"
bootstrap_token="scoped-e2e-bootstrap-token"
root_path="$artifact_dir/root.bin"
root_voters="1@127.0.0.1:$((base_port + 1)),2@127.0.0.1:$((base_port + 2)),3@127.0.0.1:$((base_port + 3))"
declare -A pids=()
declare -A tickets=()

minio_port=$((base_port + 900))
minio_name="kv9-scoped-e2e-minio-$base_port"
export KV9_OBJECT_STORE_ENDPOINT="http://127.0.0.1:$minio_port" KV9_OBJECT_STORE_BUCKET=kv9-scoped-e2e
export KV9_OBJECT_STORE_ACCESS_KEY=kv9scoped KV9_OBJECT_STORE_SECRET_KEY=kv9scoped-secret-key
node_environment=(KV9_CLUSTER_TOKEN="$cluster_token" KV9_CLIENT_TOKENS="admin=$admin_token,app=$app_token"
  KV9_ADMIN_PRINCIPALS=admin KV9_SCOPE_IDLE_MS=500 KV9_STORAGE=minio KV9_SCOPE_RING_ENTRIES=32)

status_value() {
  local file="$artifact_dir/n${1}/status"
  test -f "$file" || return 1
  awk -F= -v wanted="$2" '$1 == wanted { print substr($0, length($1) + 2); found=1 } END { if (!found) exit 1 }' "$file"
}

start_voter() {
  local node="$1"
  mkdir -p "$artifact_dir/n${node}"
  if [[ ! -e "$artifact_dir/n${node}/kv9-store-identity" ]]; then
    KV9_BOOTSTRAP_TOKEN="$bootstrap_token" "$bin" init --root "$root_path" --node-id "$node" \
      --data-dir "$artifact_dir/n${node}" >>"$artifact_dir/n${node}.log" 2>&1
  fi
  env "${node_environment[@]}" "$bin" start --node-id "$node" --addr "127.0.0.1:$((base_port + node))" \
    --data-dir "$artifact_dir/n${node}" >>"$artifact_dir/n${node}.log" 2>&1 &
  pids[$node]=$!
}

start_joiner() {
  local node="$1"
  env "${node_environment[@]}" KV9_JOIN_TICKET="${tickets[$node]}" "$bin" start --node-id "$node" \
    --addr "127.0.0.1:$((base_port + node))" --data-dir "$artifact_dir/n${node}" >>"$artifact_dir/n${node}.log" 2>&1 &
  pids[$node]=$!
}

stop_node() {
  local node="$1"
  kill "${pids[$node]}" 2>/dev/null || true
  wait "${pids[$node]}" 2>/dev/null || true
  unset "pids[$node]"
}

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
  while (( SECONDS < deadline )); do
    if "$@"; then return 0; fi
    sleep 0.3
  done
  echo "FAIL: timed out waiting for $label" >&2
  for f in "$artifact_dir"/n*/status; do
    echo "--- $f"
    grep -E "^(node_id|bootstrap_state|role|leader_id|meta_voters|meta_learners|fatal|meta_mode|catalog_scope[a-z_]*|group_control_error)=" "$f"
  done >&2 || true
  exit 1
}

serving() {
  local node
  for node in "$@"; do
    # The status file of a process that died stays behind; only the live one counts.
    test "$(status_value "$node" pid 2>/dev/null || true)" = "${pids[$node]:-none}" || return 1
    test "$(status_value "$node" bootstrap_state 2>/dev/null || true)" = "Serving" || return 1
    test -z "$(status_value "$node" fatal 2>/dev/null || true)" || return 1
  done
}

leader=""
voters_agree() {
  local node seen first=0
  for node in 1 2 3; do
    [[ -n "${pids[$node]:-}" ]] || continue
    serving "$node" || return 1
    seen="$(status_value "$node" leader_id 2>/dev/null || true)"
    [[ "$seen" =~ ^[0-9]+$ ]] && (( seen > 0 )) || return 1
    (( first == 0 )) && first="$seen"
    test "$seen" -eq "$first" || return 1
  done
  leader="$first"
}

endpoint_of() { echo "127.0.0.1:$((base_port + $1))"; }
any_voter() {
  local node
  for node in 1 2 3; do [[ -n "${pids[$node]:-}" ]] && { endpoint_of "$node"; return; }; done
}
admin() { KV9_ADMIN_TOKEN="$admin_token" timeout 60 "$admin_bin" --endpoints "$(any_voter)" --output json --yes "$@"; }
# Verbs that --wait for a whole operation (backup, restore, drain).
admin_long() { KV9_ADMIN_TOKEN="$admin_token" timeout 1200 "$admin_bin" --endpoints "$(any_voter)" --output json --yes "$@"; }
jq_py() { python3 -c 'import json,sys; d=json.load(sys.stdin); print(eval(sys.argv[1], {"d": d}))' "$1"; }

scope_current() {
  local node
  for node in "$@"; do
    test "$(status_value "$node" meta_mode 2>/dev/null || true)" = "scoped" || return 1
    test "$(status_value "$node" catalog_scope 2>/dev/null || true)" = "current" || return 1
  done
}

echo "Starting MinIO on port $minio_port (replica plans and splits need object storage)..."
docker rm -f "$minio_name" >/dev/null 2>&1 || true
docker run -d --name "$minio_name" -p "127.0.0.1:$minio_port:9000" -e MINIO_ROOT_USER="$KV9_OBJECT_STORE_ACCESS_KEY" \
  -e MINIO_ROOT_PASSWORD="$KV9_OBJECT_STORE_SECRET_KEY" quay.io/minio/minio:latest server /data >/dev/null
for _ in $(seq 120); do curl -sf "$KV9_OBJECT_STORE_ENDPOINT/minio/health/live" >/dev/null && break; sleep 0.5; done
docker exec -e "MC_HOST_d=http://$KV9_OBJECT_STORE_ACCESS_KEY:$KV9_OBJECT_STORE_SECRET_KEY@127.0.0.1:9000" "$minio_name" \
  mc mb --ignore-existing "d/$KV9_OBJECT_STORE_BUCKET" >/dev/null

echo "Building kv9 and kv9-admin..."
bin="$(build_fixture_binary "$repo_dir" "$artifact_dir/build.jsonl")"
cargo build --locked --manifest-path "$repo_dir/Cargo.toml" -p kv9-admin --bin kv9-admin >"$artifact_dir/build-admin.log" 2>&1
admin_bin="$(dirname "$bin")/kv9-admin"
repl_bin="$(dirname "$bin")/kv9-repl"

KV9_BOOTSTRAP_TOKEN="$bootstrap_token" "$bin" root-create --output "$root_path" \
  --voters "$root_voters" --store-incarnations "$(prepare_root_stores "$bin" "$artifact_dir" "$root_voters")" \
  >"$artifact_dir/root-create.log"

echo "Starting voters 1-3 on ports $((base_port + 1))-$((base_port + 3))..."
for node in 1 2 3; do start_voter "$node"; done
wait_until "three voters Serving behind one leader" 30 voters_agree
echo "Leader is n${leader}."

# ---------------------------------------------------------------- phase A: scoped admission and join
for node in 4 5 6; do
  admitted="$(admin node admit "$node" --addr "$(endpoint_of "$node")" --ttl 300 --meta scoped)"
  test "$(echo "$admitted" | jq_py 'd["result"]["meta_mode"]')" = "scoped" || { echo "FAIL: admit $node: $admitted" >&2; exit 1; }
  tickets[$node]="$(echo "$admitted" | jq_py 'd["result"]["join_ticket"]')"
  mkdir -p "$artifact_dir/n${node}"
  KV9_JOIN_TICKET="${tickets[$node]}" "$bin" join --root "$root_path" --node-id "$node" \
    --addr "$(endpoint_of "$node")" --data-dir "$artifact_dir/n${node}" >"$artifact_dir/n${node}.join" 2>&1
  start_joiner "$node"
done
wait_until "scoped nodes 4-6 Serving" 90 serving 4 5 6
wait_until "scoped slices current" 30 scope_current 4 5 6
for node in 4 5 6; do
  test "$(status_value "$node" raft_owner_started)" = "false" || { echo "FAIL: n$node started a metadata Raft owner" >&2; exit 1; }
  test -e "$artifact_dir/n${node}/kv9-meta-scoped"
done
nodes="$(admin node list)"
test "$(echo "$nodes" | jq_py 'sorted((n["node_id"], n["raft_role"], n["meta_mode"]) for n in d["result"]["nodes"] if n["node_id"] > 3)')" = \
  "[(4, 'none', 'scoped'), (5, 'none', 'scoped'), (6, 'none', 'scoped')]"
test "$(echo "$nodes" | jq_py 'all(n["active"] for n in d["result"]["nodes"])')" = "True"
info="$(admin cluster info)"
test "$(echo "$info" | jq_py 'sorted(d["result"]["meta_voters"])')" = "[1, 2, 3]"
test "$(echo "$info" | jq_py 'd["result"]["meta_learners"]')" = "[]"
echo "phase A: nodes 4-6 admitted scoped, Serving, no metadata Raft owner, slices current."

# ---------------------------------------------------------------- phase B: another keyspace stays out of the slices
group_a="$(admin group create --voters 1,2,3 --label scoped-e2e-a | jq_py 'd["result"]["creation_task"]')"
ks_a="$(admin keyspace create --name on-voters --group "$group_a" | jq_py 'd["result"]["id"]')"
group_active() {
  local task="$1"
  admin group describe "$task" | jq_py 'len(d["result"]["replica_views"]) == 3 and all(r["observed"] is not None and r["observed"]["state"] == "active" for r in d["result"]["replica_views"])' | grep -qx True
}
wait_until "group $group_a active" 60 group_active "$group_a"
slice_index_past() {
  local node want="$1"; shift
  for node in "$@"; do
    (( $(status_value "$node" catalog_scope_index) >= want )) || return 1
  done
}
mark="$(admin cluster info | jq_py 'd["result"]["applied_index"]' 2>/dev/null || echo 0)"
sleep 2
for node in 4 5 6; do
  test "$(status_value "$node" catalog_scope_regions)" = "0" || { echo "FAIL: n$node slice holds regions of the voters' keyspace" >&2; exit 1; }
done
echo "phase B: group $group_a and keyspace $ks_a on the voters; scoped slices hold no region of it."

# ---------------------------------------------------------------- phase C: a group on scoped nodes only
group_b="$(admin group create --voters 4,5,6 --label scoped-e2e-b | jq_py 'd["result"]["creation_task"]')"
wait_until "group $group_b active on scoped nodes 4-6" 90 group_active "$group_b"
bind_scoped_keyspace() {
  admin keyspace create --name on-scoped --group "$group_b" >"$artifact_dir/bind-b.json" 2>"$artifact_dir/bind-b.err" || true
  ks_b="$(jq_py 'd["result"]["id"]' <"$artifact_dir/bind-b.json" 2>/dev/null)" && [[ "$ks_b" =~ ^[0-9]+$ ]]
}
wait_until "keyspace on-scoped bound to group $group_b ($(cat "$artifact_dir/bind-b.json" 2>/dev/null | head -c 300))" 60 bind_scoped_keyspace
slices_hold_one_region() {
  local node
  for node in 4 5 6; do test "$(status_value "$node" catalog_scope_regions)" = "1" || return 1; done
}
wait_until "scoped slices hold the scoped group's region" 30 slices_hold_one_region
peers="1=$(endpoint_of 1),2=$(endpoint_of 2),3=$(endpoint_of 3),4=$(endpoint_of 4),5=$(endpoint_of 5),6=$(endpoint_of 6)"
for attempt in $(seq 1 60); do
  if printf 'use on-scoped\nput s/1 one\nput s/2 two\nget s/1\nscan s/ s0\n' | KV9_CLIENT_TOKEN="$app_token" timeout 60 "$repl_bin" \
    --addr "$(endpoint_of 4)" --peers "$peers" >"$artifact_dir/repl-scoped.out" 2>&1 && grep -q '^one$' "$artifact_dir/repl-scoped.out"; then
    break
  fi
  if [[ "$attempt" == 60 ]]; then cat "$artifact_dir/repl-scoped.out" >&2; exit 1; fi
  sleep 1
done
echo "phase C: group $group_b on scoped nodes 4-6 is active; keyspace $ks_b serves raw traffic through n4."

# ---------------------------------------------------------------- phase D: what each slice holds
region_a="$(admin group describe "$group_a" | jq_py 'd["result"]["region_id"]')"
region_b="$(admin group describe "$group_b" | jq_py 'd["result"]["region_id"]')"
for node in 4 5 6; do
  test "$(status_value "$node" catalog_scope_region_ids)" = "$region_b" || {
    echo "FAIL: n$node slice regions $(status_value "$node" catalog_scope_region_ids), want $region_b (not $region_a)" >&2; exit 1; }
done
echo "phase D: each scoped slice holds region $region_b of its own group and not region $region_a."

repl_on() {
  local node="$1" lines="$2" out="$3"
  printf '%s' "$lines" | KV9_CLIENT_TOKEN="$app_token" timeout 60 "$repl_bin" --addr "$(endpoint_of "$node")" --peers "$peers" >"$out" 2>&1
}
read_back() {
  local node="$1" key="$2" want="$3"
  repl_on "$node" "use on-scoped
get $key
" "$artifact_dir/read-$node.out" && grep -qx "$want" "$artifact_dir/read-$node.out"
}

# ---------------------------------------------------------------- phase E: a scoped node restarts from its slice
stop_node 5
write_while_down() {
  repl_on 4 'use on-scoped
put e/1 while-five-down
' "$artifact_dir/repl-e.out" && grep -q '^applied' "$artifact_dir/repl-e.out"
}
# n5 may have led the group: the write waits out the election among 4 and 6.
wait_until "a write while n5 is down" 60 write_while_down
start_joiner 5
wait_until "n5 Serving again" 60 serving 5
wait_until "n5 slice current again" 30 scope_current 5
test "$(status_value 5 raft_owner_started)" = "false"
wait_until "n5 serves the write made while it was down" 60 read_back 5 e/1 while-five-down
echo "phase E: n5 restarted from its slice (no metadata Raft owner) and serves data written while it was down."

# ---------------------------------------------------------------- phase F: a subscriber's full node stops
served_by="$(status_value 4 catalog_scope_server)"
if (( served_by >= 1 && served_by <= 3 )); then
  stop_node "$served_by"
  moved() { local now; now="$(status_value 4 catalog_scope_server)"; [[ "$now" =~ ^[123]$ ]] && (( now != served_by )) && scope_current 4; }
  wait_until "n4 moves off stopped full node $served_by" 60 moved
  echo "phase F: full node $served_by stopped; n4 now streams from node $(status_value 4 catalog_scope_server)."
  start_voter "$served_by"
  wait_until "voters agree again" 60 voters_agree
else
  echo "FAIL: n4 streams from node $served_by, not a full node" >&2; exit 1
fi

# ---------------------------------------------------------------- phase G: move a replica onto a new scoped node
admitted="$(admin node admit 7 --addr "$(endpoint_of 7)" --ttl 300 --meta scoped)"
tickets[7]="$(echo "$admitted" | jq_py 'd["result"]["join_ticket"]')"
mkdir -p "$artifact_dir/n7"
KV9_JOIN_TICKET="${tickets[7]}" "$bin" join --root "$root_path" --node-id 7 --addr "$(endpoint_of 7)" \
  --data-dir "$artifact_dir/n7" >"$artifact_dir/n7.join" 2>&1
start_joiner 7
wait_until "n7 Serving" 90 serving 7
peers="$peers,7=$(endpoint_of 7)"
plan="$(admin group move-replica "$group_b" --from 6 --to 7 --label scoped-move-6-7)"
voters_are() {
  admin group describe "$group_b" | jq_py 'next(iter([sorted(v["observed"]["voters"]) for v in d["result"]["replica_views"] if (v.get("observed") or {}).get("role") == "Leader" and v["observed"].get("state") == "active"]), None)' | grep -qx "\[$1\]"
}
wait_until "group $group_b voters 4,5,7" 300 voters_are "4, 5, 7"
wait_until "n7 slice holds region $region_b" 30 bash -c "grep -qx 'catalog_scope_region_ids=$region_b' '$artifact_dir/n7/status'"
wait_until "n7 serves the moved group" 60 read_back 7 s/1 one
echo "phase G: replica moved from scoped n6 to scoped n7; n7's slice holds the group and serves it."

# ---------------------------------------------------------------- phase H: split the scoped group
split="$(admin group split "$group_b" --at s/2 --label scoped-split)"
stask="$(echo "$split" | jq_py 'd["result"]["intent_task"]')"
split_published() {
  admin group describe "$group_b" | jq_py "[s['stage'] for s in d['result']['splits'] if s['task'] == $stask]" | grep -qx "\['published'\]"
}
wait_until "split $stask published" 600 split_published
three_regions() {
  local node
  for node in 4 5 7; do test "$(status_value "$node" catalog_scope_regions)" = "3" || return 1; done
}
wait_until "scoped slices hold the parent and both children" 60 three_regions
wait_until "reads on both sides of the split through n4" 60 read_back 4 s/2 two
read_back 4 s/1 one
echo "phase H: the scoped group split at s/2; slices hold parent and children; both sides serve."

# ---------------------------------------------------------------- phase I: a request outside the slice is redirected
printf 'use on-voters\nput v/1 via-n4\n' | KV9_CLIENT_TOKEN="$app_token" timeout 30 "$repl_bin" --addr "$(endpoint_of 4)" \
  >"$artifact_dir/repl-redirect-bare.out" 2>&1 || true
grep -q 'not leader (hint: node [123])' "$artifact_dir/repl-redirect-bare.out" || {
  echo "FAIL: n4 did not redirect a keyspace outside its slice to a full node" >&2; cat "$artifact_dir/repl-redirect-bare.out" >&2; exit 1; }
redirected_write() {
  printf 'use on-voters\nput v/1 via-n4\nget v/1\n' | KV9_CLIENT_TOKEN="$app_token" timeout 30 "$repl_bin" --addr "$(endpoint_of 4)" \
    --peers "$peers" >"$artifact_dir/repl-redirect.out" 2>&1 && grep -qx 'via-n4' "$artifact_dir/repl-redirect.out"
}
wait_until "a client at n4 is served for the voters' keyspace through the redirect" 60 redirected_write
for node in 4 5 7; do
  test "$(status_value "$node" catalog_scope_regions)" = "3" || { echo "FAIL: n$node slice grew from a redirected request" >&2; exit 1; }
done
echo "phase I: n4 answers a keyspace outside its slice with a redirect to a full node; a client with peers is served."

# ---------------------------------------------------------------- phase J: merge the scoped children back
live_children() {
  admin group list | python3 -c '
import json,sys; ks=int(sys.argv[1]); gs=json.load(sys.stdin)["result"]
live=sorted((min(k["start"] for k in g["keyspaces"] if k["keyspace_id"]==ks), g["creation_task"]) for g in gs if any(k["keyspace_id"]==ks and not k["sealed"] for k in g["keyspaces"]))
print(" ".join(str(t) for _, t in live))' "$ks_b"
}
read -r low high <<<"$(live_children)"
[[ -n "$low" && -n "$high" ]] || { echo "FAIL: two live children expected, got '$(live_children)'" >&2; exit 1; }
merge="$(admin group merge "$low" "$high" --label scoped-merge)"
mtask="$(echo "$merge" | jq_py 'd["result"]["intent_task"]')"
merge_published() {
  admin group describe "$low" | jq_py "[m['stage'] for m in d['result']['merges'] if m['task'] == $mtask]" | grep -qx "\['published'\]"
}
wait_until "merge $mtask published" 600 merge_published
merged="$(live_children)"
[[ "$merged" =~ ^[0-9]+$ ]] || { echo "FAIL: one live group expected after the merge, got '$merged'" >&2; exit 1; }
wait_until "reads on both sides of the old boundary through n7" 60 read_back 7 s/2 two
read_back 7 s/1 one
four_regions() {
  local node
  for node in 4 5 7; do test "$(status_value "$node" catalog_scope_regions)" = "4" || return 1; done
}
wait_until "scoped slices hold parent, both children and the merged group" 60 four_regions
echo "phase J: the scoped children merged into group $merged; both sides serve through n7; slices hold all four regions."

# ---------------------------------------------------------------- phase K: back up the scoped keyspace while n5 is down
stop_node 5
snapshots_before="$(status_value 1 catalog_scope_snapshots_sent)"
backup="$(admin_long backup create --keyspace on-scoped --label scoped-backup --wait --wait-secs 600)"
btask="$(echo "$backup" | jq_py 'd["result"]["task"]')"
test "$(echo "$backup" | jq_py 'd["result"]["waited"]["stage"]')" = "complete" || { echo "FAIL: backup: $backup" >&2; exit 1; }
# Push the metadata log past the 32-entry ring so n5's restart cannot resume from it.
for i in $(seq 1 12); do admin keyspace create --name "filler-$i" >/dev/null; done
echo "phase K: backup $btask of the scoped keyspace complete with n5 down; the metadata log moved past the ring."

# ---------------------------------------------------------------- phase L: restart behind the retained ring
ring_last="$(status_value 1 catalog_ring_last)"
start_joiner 5
wait_until "n5 Serving after a restart behind the ring" 90 serving 5
caught_up() { (( $(status_value 5 catalog_scope_index) >= ring_last )) && scope_current 5; }
wait_until "n5 slice at or past index $ring_last" 60 caught_up
(( $(status_value 5 catalog_scope_snapshots) >= 1 ))
wait_until "n5 serves after the restart" 60 read_back 5 s/1 one
echo "phase L: n5 restarted with the log $((ring_last)) entries on (ring 32); it caught up through a snapshot and serves."

# ---------------------------------------------------------------- phase M: restore onto scoped voters
restore="$(admin_long restore create --backup "$btask" --as on-scoped-restored --voters 4,5,7 --wait --wait-secs 600)"
test "$(echo "$restore" | jq_py 'd["result"]["waited"]["stage"]')" = "published" || { echo "FAIL: restore: $restore" >&2; exit 1; }
restored_read() {
  printf 'use on-scoped-restored\nget s/1\nget s/2\n' | KV9_CLIENT_TOKEN="$app_token" timeout 30 "$repl_bin" --addr "$(endpoint_of 5)" \
    --peers "$peers" >"$artifact_dir/repl-restored.out" 2>&1 && grep -qx one "$artifact_dir/repl-restored.out" && grep -qx two "$artifact_dir/repl-restored.out"
}
wait_until "the restored keyspace serves through scoped n5" 120 restored_read
echo "phase M: restore of backup $btask onto scoped nodes 4,5,7 published; the new keyspace serves through n5."

# ---------------------------------------------------------------- phase N: drain a scoped node
admitted="$(admin node admit 8 --addr "$(endpoint_of 8)" --ttl 300 --meta scoped)"
tickets[8]="$(echo "$admitted" | jq_py 'd["result"]["join_ticket"]')"
mkdir -p "$artifact_dir/n8"
KV9_JOIN_TICKET="${tickets[8]}" "$bin" join --root "$root_path" --node-id 8 --addr "$(endpoint_of 8)" \
  --data-dir "$artifact_dir/n8" >"$artifact_dir/n8.join" 2>&1
start_joiner 8
wait_until "n8 Serving" 90 serving 8
peers="$peers,8=$(endpoint_of 8)"
admin_long node drain 4 --wait --wait-secs 1100 >"$artifact_dir/drain-4.json" || { cat "$artifact_dir/drain-4.json" >&2; exit 1; }
test "$(admin node list | jq_py '[n["drained"] for n in d["result"]["nodes"] if n["node_id"] == 4][0]')" = "True"
wait_until "the scoped keyspace still serves through n7 after the drain" 60 read_back 7 s/2 two
wait_until "the restored keyspace serves through n8's peers after the drain" 120 restored_read
echo "phase N: scoped n4 drained (its replicas moved, no metadata membership to release); both keyspaces serve."
health="$(admin cluster health || true)"
echo "$health" | jq_py '[v for v in d["result"]["verdicts"] if v["name"] == "catalog_scope"][0]' >"$artifact_dir/health-scope.txt"
grep -q "'level': 'ok'" "$artifact_dir/health-scope.txt" || { echo "FAIL: catalog_scope health verdict: $(cat "$artifact_dir/health-scope.txt")" >&2; exit 1; }
echo "cluster health: $(cat "$artifact_dir/health-scope.txt")"
echo "PASS: scoped metadata mode (admission, slices, isolation, data group on scoped nodes, restart, failover, move, split, redirect, merge, backup, restart behind the ring, restore, drain, health)"
