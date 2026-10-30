#!/usr/bin/env bash
# Scale acceptance for the scoped metadata mode (docs/META-REPLICA-MODES.md §7, §8.4).
#
# Three metadata voters and KV9_SCALE_SCOPED scoped nodes (default 24) on one host. Data
# groups of three replicas are placed on disjoint triples of scoped nodes, one keyspace
# each, with a few writes. Measured and checked:
#   - the metadata Raft group stays at three voters and no learners (M4/R1);
#   - every scoped slice is current and holds exactly one group's region;
#   - a slice holds a small fraction of the full catalog (R3), and the fraction does not
#     grow with the number of groups elsewhere (R2);
#   - subscribers spread over the full nodes (R4);
#   - when the full node serving the most subscribers stops, only its subscribers move,
#     and every slice is current again.
# KV9_SCALE_MODE=full is the baseline: the same nodes join as full metadata learners
# (KV9_META_FULL_LEARNERS raised), and only the metadata leader's send counters are compared.
# A JSON summary is written to $artifact_dir/scale-summary.json.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/root_provision.sh"

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
artifact_dir="${KV9_SCALE_DIR:-$(mktemp -d /tmp/kv9-scoped-scale.XXXXXX)}"
scoped="${KV9_SCALE_SCOPED:-24}"
mode="${KV9_SCALE_MODE:-scoped}"
[[ "$mode" == scoped || "$mode" == full ]] || { echo "KV9_SCALE_MODE must be scoped or full" >&2; exit 2; }
(( scoped % 3 == 0 )) || { echo "KV9_SCALE_SCOPED must be a multiple of 3" >&2; exit 2; }
# Below the kernel ephemeral range (32768+): with hundreds of nodes, outbound sockets would
# otherwise take listening ports before their nodes bind them.
base_port="${KV9_BASE_PORT:-$((21000 + ($$ % 500)))}"
cluster_token="scale-cluster-token"; admin_token="scale-admin-token"; app_token="scale-app-token"
bootstrap_token="scale-bootstrap-token"
root_path="$artifact_dir/root.bin"
root_voters="1@127.0.0.1:$((base_port + 1)),2@127.0.0.1:$((base_port + 2)),3@127.0.0.1:$((base_port + 3))"
last=$((3 + scoped))
declare -A pids=()
node_environment=(KV9_CLUSTER_TOKEN="$cluster_token" KV9_CLIENT_TOKENS="admin=$admin_token,app=$app_token"
  KV9_ADMIN_PRINCIPALS=admin KV9_SCOPE_IDLE_MS=1000 KV9_META_FULL_LEARNERS=100000)

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
}
trap stop_all EXIT
wait_until() {
  local label="$1" timeout="$2"; shift 2
  local deadline=$((SECONDS + timeout))
  while (( SECONDS < deadline )); do "$@" && return 0; sleep 0.5; done
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
  [[ "$mode" == full ]] && return 0
  local node
  for node in $(seq 4 "$last"); do
    [[ -n "${pids[$node]:-}" ]] || continue
    test "$(status_value "$node" catalog_scope 2>/dev/null || true)" = "current" || return 1
  done
}

echo "Building..."
bin="$(build_fixture_binary "$repo_dir" "$artifact_dir/build.jsonl")"
cargo build --locked --manifest-path "$repo_dir/Cargo.toml" -p kv9-admin --bin kv9-admin >"$artifact_dir/build-admin.log" 2>&1
admin_bin="$(dirname "$bin")/kv9-admin"
KV9_BOOTSTRAP_TOKEN="$bootstrap_token" "$bin" root-create --output "$root_path" \
  --voters "$root_voters" --store-incarnations "$(prepare_root_stores "$bin" "$artifact_dir" "$root_voters")" >"$artifact_dir/root-create.log"
for node in 1 2 3; do
  KV9_BOOTSTRAP_TOKEN="$bootstrap_token" "$bin" init --root "$root_path" --node-id "$node" --data-dir "$artifact_dir/n$node" >>"$artifact_dir/n$node.log" 2>&1
  env "${node_environment[@]}" "$bin" start --node-id "$node" --addr "$(endpoint_of "$node")" --data-dir "$artifact_dir/n$node" >>"$artifact_dir/n$node.log" 2>&1 &
  pids[$node]=$!
done
wait_until "three voters Serving" 60 voters_up

started=$SECONDS
for node in $(seq 4 "$last"); do
  ticket=""
  for attempt in 1 2 3 4 5 6; do
    # A metadata election under the join load can refuse one admission; an operator retries.
    ticket="$(admin node admit "$node" --addr "$(endpoint_of "$node")" --ttl 600 --meta "$mode" | jq_py 'd["result"]["join_ticket"]' 2>/dev/null)" && break
    sleep 2
  done
  [[ "$ticket" =~ ^[0-9a-f]{64}$ ]] || { echo "FAIL: admit $node" >&2; exit 1; }
  mkdir -p "$artifact_dir/n$node"
  KV9_JOIN_TICKET="$ticket" "$bin" join --root "$root_path" --node-id "$node" --addr "$(endpoint_of "$node")" \
    --data-dir "$artifact_dir/n$node" >"$artifact_dir/n$node.join" 2>&1
  env "${node_environment[@]}" KV9_JOIN_TICKET="$ticket" "$bin" start --node-id "$node" --addr "$(endpoint_of "$node")" \
    --data-dir "$artifact_dir/n$node" >>"$artifact_dir/n$node.log" 2>&1 &
  pids[$node]=$!
done
wait_until "$scoped scoped nodes Serving" 300 serving $(seq 4 "$last")
wait_until "every slice current" 120 all_current
join_secs=$((SECONDS - started))
echo "$scoped scoped nodes admitted and Serving in ${join_secs}s."

leader_sent() {
  local l; l="$(status_value 1 leader_id)"
  echo "$(status_value "$l" meta_raft_sent_msgs) $(status_value "$l" meta_raft_sent_bytes) $l"
}
read -r create_msgs0 create_bytes0 _ < <(leader_sent)
groups=$((scoped / 3))
declare -A group_region=()
for g in $(seq 0 $((groups - 1))); do
  a=$((4 + 3 * g)); voters="$a,$((a + 1)),$((a + 2))"
  task="$(admin group create --voters "$voters" --label "scale-$g" | jq_py 'd["result"]["creation_task"]')"
  for _ in $(seq 60); do
    admin keyspace create --name "scale-$g" --group "$task" >"$artifact_dir/bind-$g.json" || true
    jq_py 'd["result"]["id"]' <"$artifact_dir/bind-$g.json" >/dev/null 2>&1 && break
    sleep 1
  done
  group_region[$g]="$(jq_py 'd["result"]["region_id"]' <"$artifact_dir/bind-$g.json")"
done
echo "$groups groups created and bound, one per scoped triple."
one_region_each() {
  local g a node
  for g in $(seq 0 $((groups - 1))); do
    a=$((4 + 3 * g))
    for node in $a $((a + 1)) $((a + 2)); do
      test "$(status_value "$node" catalog_scope_region_ids)" = "${group_region[$g]}" || return 1
    done
  done
}
read -r create_msgs1 create_bytes1 _ < <(leader_sent)
read -r idle_msgs0 idle_bytes0 leader0 < <(leader_sent)
sleep 30   # steady window; also lets the full nodes recount their catalogs (every 30 s)
read -r idle_msgs1 idle_bytes1 leader1 < <(leader_sent)
sleep 1.5
python3 - "$artifact_dir/leader-load.json" "$mode" "$scoped" "$groups" "$create_msgs0" "$create_msgs1" "$create_bytes0" "$create_bytes1" "$idle_msgs0" "$idle_msgs1" "$idle_bytes0" "$idle_bytes1" "$leader0" "$leader1" <<'PY'
import json, sys
o, mode, n, g, cm0, cm1, cb0, cb1, im0, im1, ib0, ib1, l0, l1 = sys.argv[1:]
load = {"mode": mode, "nodes_beyond_voters": int(n), "groups": int(g), "leader_stable": l0 == l1,
        "create_phase": {"msgs": int(cm1) - int(cm0), "bytes": int(cb1) - int(cb0)},
        "steady_30s": {"msgs_per_s": round((int(im1) - int(im0)) / 30, 1), "bytes_per_s": round((int(ib1) - int(ib0)) / 30, 1)}}
json.dump(load, open(o, "w"), indent=2); print(json.dumps(load))
PY
if [[ "$mode" == full ]]; then
  info="$(admin cluster info)"
  test "$(echo "$info" | jq_py 'len(d["result"]["meta_learners"])')" = "$scoped" || { echo "FAIL: baseline learners" >&2; exit 1; }
  echo "PASS: full-learner baseline ($scoped learners; $artifact_dir/leader-load.json)"
  exit 0
fi
wait_until "each slice holds exactly its own group's region" 120 one_region_each
wait_until "every slice current" 60 all_current

info="$(admin cluster info)"
voters_now="$(echo "$info" | jq_py 'sorted(d["result"]["meta_voters"])')"
learners_now="$(echo "$info" | jq_py 'd["result"]["meta_learners"]')"
test "$voters_now" = "[1, 2, 3]" && test "$learners_now" = "[]" || { echo "FAIL: metadata membership $voters_now / $learners_now" >&2; exit 1; }

collect() {
  python3 - "$artifact_dir" "$scoped" "$1" <<'PY'
import json, sys, os
d, scoped, label = sys.argv[1], int(sys.argv[2]), sys.argv[3]
def status(n):
    out = {}
    try:
        for line in open(os.path.join(d, f"n{n}", "status")):
            k, _, v = line.rstrip("\n").partition("=")
            out[k] = v
    except FileNotFoundError:
        pass
    return out
full = {n: status(n) for n in (1, 2, 3)}
sc = {n: status(n) for n in range(4, 4 + scoped)}
catalog = max(int(s.get("catalog_keys", 0) or 0) for s in full.values())
rows = [int(s.get("catalog_scope_rows", 0) or 0) for s in sc.values()]
served = {n: int(s.get("catalog_scope_streams", 0) or 0) for n, s in full.items()}
servers = {}
for n, s in sc.items():
    servers.setdefault(s.get("catalog_scope_server", "?"), []).append(n)
summary = {
    "label": label, "scoped_nodes": scoped, "full_catalog_keys": catalog,
    "slice_rows_min": min(rows), "slice_rows_max": max(rows), "slice_rows_mean": round(sum(rows) / len(rows), 1),
    "slice_fraction_max": round(max(rows) / catalog, 4) if catalog else None,
    "streams_per_full_node": served,
    "subscribers_by_server": {k: sorted(v) for k, v in sorted(servers.items())},
    "bytes_sent_per_full_node": {n: int(s.get("catalog_scope_bytes_sent", 0) or 0) for n, s in full.items()},
    "snapshots_sent_per_full_node": {n: int(s.get("catalog_scope_snapshots_sent", 0) or 0) for n, s in full.items()},
    "bytes_received_max": max(int(s.get("catalog_scope_bytes_received", 0) or 0) for s in sc.values()),
}
print(json.dumps(summary, indent=2))
PY
}
collect steady >"$artifact_dir/scale-steady.json"
cat "$artifact_dir/scale-steady.json"
python3 - "$artifact_dir/scale-steady.json" <<'PY'
import json, sys
s = json.load(open(sys.argv[1]))
assert s["slice_rows_max"] < s["full_catalog_keys"] / 2, "a slice holds half the catalog or more"
assert len([v for v in s["streams_per_full_node"].values() if v > 0]) >= 2, "subscribers did not spread over the full nodes"
PY

# R7: cluster health over every node vs. over the full nodes alone (scoped slices judged
# from the full nodes' subscriber tables).
t0=$(date +%s%N); admin cluster health >"$artifact_dir/health-all.json" || true; t1=$(date +%s%N)
admin cluster health --full-only >"$artifact_dir/health-full.json" || true; t2=$(date +%s%N)
python3 - "$artifact_dir" "$t0" "$t1" "$t2" <<'PY'
import json, sys
d, t0, t1, t2 = sys.argv[1], *map(int, sys.argv[2:])
def scope(f):
    v = [x for x in json.load(open(f"{d}/{f}"))["result"]["verdicts"] if x["name"] == "catalog_scope"][0]
    return v["level"], v["detail"][:120]
out = {"all_nodes_ms": (t1 - t0) // 1_000_000, "full_only_ms": (t2 - t1) // 1_000_000,
       "all_nodes_scope": scope("health-all.json"), "full_only_scope": scope("health-full.json")}
json.dump(out, open(f"{d}/health-timing.json", "w")); print(json.dumps(out))
assert out["all_nodes_scope"][0] == "ok" and out["full_only_scope"][0] == "ok", out
PY

# The full node serving the most subscribers stops; only its subscribers move.
busiest="$(python3 -c 'import json,sys; s=json.load(open(sys.argv[1])); print(max(((len(v), k) for k, v in s["subscribers_by_server"].items() if k in ("1","2","3")))[1])' "$artifact_dir/scale-steady.json")"
moved_expected="$(python3 -c 'import json,sys; s=json.load(open(sys.argv[1])); print(" ".join(map(str, s["subscribers_by_server"][sys.argv[2]])))' "$artifact_dir/scale-steady.json" "$busiest")"
kill "${pids[$busiest]}"; wait "${pids[$busiest]}" 2>/dev/null || true; unset "pids[$busiest]"
moved_off() {
  local node
  for node in $moved_expected; do
    [[ "$(status_value "$node" catalog_scope_server)" =~ ^[123]$ ]] && test "$(status_value "$node" catalog_scope_server)" != "$busiest" || return 1
  done
  all_current
}
failover_started=$SECONDS
wait_until "subscribers of node $busiest move" 120 moved_off
failover_secs=$((SECONDS - failover_started))
collect "after stopping node $busiest" >"$artifact_dir/scale-failover.json"
python3 - "$artifact_dir/scale-steady.json" "$artifact_dir/scale-failover.json" "$busiest" "$join_secs" "$failover_secs" "$artifact_dir/scale-summary.json" <<'PY'
import json, sys
before, after = json.load(open(sys.argv[1])), json.load(open(sys.argv[2]))
busiest = sys.argv[3]
stayed = {k: v for k, v in before["subscribers_by_server"].items() if k != busiest}
for server, nodes in stayed.items():
    for n in nodes:
        assert n in after["subscribers_by_server"].get(server, []), f"node {n} moved off healthy server {server}"
summary = {"steady": before, "failover": after, "stopped_full_node": int(busiest),
           "leader_load": json.load(open(sys.argv[6].replace("scale-summary.json", "leader-load.json"))),
           "health": json.load(open(sys.argv[6].replace("scale-summary.json", "health-timing.json"))),
           "join_seconds": int(sys.argv[4]), "failover_seconds": int(sys.argv[5])}
json.dump(summary, open(sys.argv[6], "w"), indent=2)
print(json.dumps({"stopped": busiest, "moved": before["subscribers_by_server"][busiest], "failover_seconds": int(sys.argv[5])}))
PY
echo "PASS: scale ($scoped scoped nodes behind 3 voters; summary in $artifact_dir/scale-summary.json)"
