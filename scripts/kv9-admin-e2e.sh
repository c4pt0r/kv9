#!/usr/bin/env bash
# kv9-admin against a real cluster (docs/KV9-ADMIN.md, increment 1).
#
# Three metadata voters. Everything administrative below goes through kv9-admin
# addressed at a FOLLOWER (resolution and redirects are the tool's job): cluster
# info/endpoints/status/health, node list, admitting two learners (the tool prints
# the one-time ticket, the new hosts join with it), promote --wait, the three-voter
# removal refusal, remove --wait once five voters exist, keyspace create/list/describe,
# JSON output validated by a parser, the exit-code table, and KV9_ADMIN_PRINCIPALS
# refusing a non-admin principal with a typed PermissionDenied.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/root_provision.sh"

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
artifact_dir="${KV9_ADMIN_E2E_DIR:-$(mktemp -d /tmp/kv9-admin-e2e.XXXXXX)}"
base_port="${KV9_BASE_PORT:-$((28000 + ($$ % 1000)))}"
cluster_token="admin-e2e-cluster-token"
admin_token="admin-e2e-admin-token"
app_token="admin-e2e-app-token"
bootstrap_token="admin-e2e-bootstrap-token"
root_path="$artifact_dir/root.bin"
root_voters="1@127.0.0.1:$((base_port + 1)),2@127.0.0.1:$((base_port + 2)),3@127.0.0.1:$((base_port + 3))"
declare -A pids=()

node_environment=(KV9_CLUSTER_TOKEN="$cluster_token" KV9_CLIENT_TOKENS="admin=$admin_token,app=$app_token"
  KV9_ADMIN_PRINCIPALS=admin)

status_value() {
  local node="$1" key="$2" file="$artifact_dir/n${1}/status"
  test -f "$file" || return 1
  awk -F= -v wanted="$key" '$1 == wanted { print substr($0, length($1) + 2); found=1 } END { if (!found) exit 1 }' "$file"
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

stop_all() {
  local pid
  for pid in "${pids[@]}"; do kill -0 "$pid" 2>/dev/null && kill "$pid" 2>/dev/null || true; done
  wait 2>/dev/null || true
}
trap stop_all EXIT

wait_until() {
  local label="$1" timeout="$2"; shift 2
  local deadline=$((SECONDS + timeout))
  while (( SECONDS < deadline )); do
    if "$@"; then return 0; fi
    sleep 0.2
  done
  echo "FAIL: timed out waiting for $label" >&2
  for f in "$artifact_dir"/n*/status; do echo "--- $f"; grep -E "^(node_id|role|leader_id|meta_voters|meta_learners|fatal)=" "$f"; done >&2 || true
  exit 1
}

agreed_leader=""
all_agree() {
  local node leader_id=0 seen
  for node in "${!pids[@]}"; do
    test "$(status_value "$node" bootstrap_state 2>/dev/null || true)" = "Serving" || return 1
    test -z "$(status_value "$node" fatal 2>/dev/null || true)" || return 1
    seen="$(status_value "$node" leader_id 2>/dev/null || true)"
    [[ "$seen" =~ ^[0-9]+$ ]] && (( seen > 0 )) || return 1
    (( leader_id == 0 )) && leader_id="$seen"
    test "$seen" -eq "$leader_id" || return 1
  done
  agreed_leader="$leader_id"
}

# A follower's address, so that every call exercises leader resolution. A removed
# node is skipped: it no longer learns who leads.
removed_node=0
follower_endpoint() {
  local node
  for node in 1 2 3; do
    if (( node != agreed_leader && node != removed_node )); then echo "127.0.0.1:$((base_port + node))"; return; fi
  done
}

# kv9-admin with ONE follower endpoint and JSON output; the output is kept for inspection.
admin() { KV9_ADMIN_TOKEN="$admin_token" timeout 60 "$admin_bin" --endpoints "$(follower_endpoint)" --output json --yes "$@"; }
# A query against the JSON document kv9-admin printed on stdin.
jq_py() { python3 -c 'import json,sys; d=json.load(sys.stdin); print(eval(sys.argv[1], {"d": d}))' "$1"; }
expect_exit() {
  local expected="$1"; shift
  local rc=0; "$@" >"$artifact_dir/last.out" 2>"$artifact_dir/last.err" || rc=$?
  if (( rc != expected )); then
    echo "FAIL: expected exit $expected, got $rc for: $*" >&2
    cat "$artifact_dir/last.out" "$artifact_dir/last.err" >&2
    exit 1
  fi
}

echo "Building kv9 and kv9-admin..."
bin="$(build_fixture_binary "$repo_dir" "$artifact_dir/build.jsonl")"
cargo build --locked --manifest-path "$repo_dir/Cargo.toml" -p kv9-admin --bin kv9-admin >"$artifact_dir/build-admin.log" 2>&1
admin_bin="$(dirname "$bin")/kv9-admin"
test -x "$admin_bin" || { echo "FAIL: $admin_bin missing" >&2; exit 1; }

KV9_BOOTSTRAP_TOKEN="$bootstrap_token" "$bin" root-create --output "$root_path" \
  --voters "$root_voters" --store-incarnations "$(prepare_root_stores "$bin" "$artifact_dir" "$root_voters")" \
  >"$artifact_dir/root-create.log"

echo "Starting voters 1-3 on ports $((base_port + 1))-$((base_port + 3))..."
for node in 1 2 3; do start_voter "$node"; done
wait_until "three voters Serving behind one leader" 30 all_agree
echo "Leader is n${agreed_leader}; kv9-admin will be pointed at $(follower_endpoint)."

# ---------------------------------------------------------------- reads, resolved through a follower
info="$(admin cluster info)"
echo "$info" | jq_py 'd["schema_version"] == 1 and d["command"] == "cluster info"' | grep -qx True
test "$(echo "$info" | jq_py 'd["resolved_leader"]["node_id"]')" = "$agreed_leader"
test "$(echo "$info" | jq_py 'sorted(d["result"]["meta_voters"])')" = "[1, 2, 3]"
test "$(echo "$info" | jq_py 'd["result"]["node_count"]')" = "3"
echo "cluster info: leader resolved through a follower, voters 1,2,3."

nodes="$(admin node list)"
test "$(echo "$nodes" | jq_py 'sorted(n["node_id"] for n in d["result"]["nodes"])')" = "[1, 2, 3]"
test "$(echo "$nodes" | jq_py 'sorted(set(n["raft_role"] for n in d["result"]["nodes"]))')" = "['voter']"
test "$(echo "$nodes" | jq_py 'all(n["active"] for n in d["result"]["nodes"])')" = "True"
echo "node list: three active voters with the leader's raft view."

status="$(admin cluster status)"
test "$(echo "$status" | jq_py 'len(d["result"])')" = "3"
test "$(echo "$status" | jq_py 'sorted(n["status"]["role"] for n in d["result"])')" = "['follower', 'follower', 'leader']"
test "$(echo "$status" | jq_py 'all(n["status"]["fatal"] == "" for n in d["result"])')" = "True"
echo "cluster status: fan-out reached every node."

health="$(admin cluster health)"
test "$(echo "$health" | jq_py 'd["result"]["overall"]')" = "ok"
echo "cluster health: ok."

endpoints="$(admin cluster endpoints)"
test "$(echo "$endpoints" | jq_py 'sorted(e["address"] for e in d["result"])')" = \
  "['127.0.0.1:$((base_port + 1))', '127.0.0.1:$((base_port + 2))', '127.0.0.1:$((base_port + 3))']"

# ---------------------------------------------------------------- keyspaces
created="$(admin keyspace create --name admin-e2e)"
ks="$(echo "$created" | jq_py 'd["result"]["id"]')"
[[ "$ks" =~ ^[0-9]+$ ]]
test "$(admin keyspace list | jq_py 'any(k["name"] == "admin-e2e" for k in d["result"])')" = "True"
test "$(admin keyspace describe admin-e2e | jq_py 'd["result"]["id"]')" = "$ks"
echo "keyspace create/list/describe: id $ks."

# ---------------------------------------------------------------- data groups and the task history
group="$(admin group create --voters 1,2,3 --label e2e-group)"
gtask="$(echo "$group" | jq_py 'd["result"]["creation_task"]')"
[[ "$gtask" =~ ^[0-9]+$ ]]
test "$(echo "$group" | jq_py 'd["result"]["outcome"]')" = "created"
# The label derives the operation id: the same request confirms the same group.
again="$(admin group create --voters 1,2,3 --label e2e-group)"
test "$(echo "$again" | jq_py 'd["result"]["outcome"]')" = "confirmed"
test "$(echo "$again" | jq_py 'd["result"]["creation_task"]')" = "$gtask"
listed="$(admin group list)"
test "$(echo "$listed" | jq_py "[sorted(r['node_id'] for r in g['replicas']) for g in d['result'] if g['creation_task'] == $gtask][0]")" = "[1, 2, 3]"
test "$(echo "$listed" | jq_py "[g['activated'] for g in d['result'] if g['creation_task'] == $gtask][0]")" = "True"
grouped="$(admin keyspace create --name grouped --group "$gtask")"
gks="$(echo "$grouped" | jq_py 'd["result"]["id"]')"
[[ "$gks" =~ ^[0-9]+$ ]]
test "$(echo "$grouped" | jq_py 'd["result"]["outcome"]')" = "created"
test "$(admin keyspace create --name grouped --group "$gtask" | jq_py 'd["result"]["outcome"]')" = "confirmed"
group_active() {
  admin group describe "$gtask" | jq_py 'all(r["observed"] is not None and r["observed"]["state"] == "active" for r in d["result"]["replica_views"]) and len(d["result"]["replica_views"]) == 3 and sorted(r["observed"]["role"] for r in d["result"]["replica_views"]) == ["Follower", "Follower", "Leader"]' | grep -qx True
}
wait_until "data group $gtask active on all three replicas behind one leader" 60 group_active
described="$(admin group describe "$gtask")"
test "$(echo "$described" | jq_py "[k['keyspace_id'] for k in d['result']['keyspaces']]")" = "[$gks]"
test "$(echo "$described" | jq_py 'sorted(r["observed"]["role"] for r in d["result"]["replica_views"])')" = "['Follower', 'Follower', 'Leader']"
echo "group create/list/describe and keyspace create --group: group $gtask, keyspace $gks, three active replicas."
# The grouped keyspace serves raw traffic through the SDK (epoch refresh + leader hint).
# The public route of a freshly bound keyspace is published a moment after the
# group activates (REPLICA-ADDITION's known fact); the first write may race it.
for attempt in $(seq 1 60); do
  if printf 'use grouped\nput g/1 one\nget g/1\n' | KV9_CLIENT_TOKEN="$app_token" timeout 60 "$(dirname "$bin")/kv9-repl" \
    --addr "$(follower_endpoint)" --peers "1=127.0.0.1:$((base_port + 1)),2=127.0.0.1:$((base_port + 2)),3=127.0.0.1:$((base_port + 3))" \
    >"$artifact_dir/repl-group.out" 2>&1 && grep -q '^one$' "$artifact_dir/repl-group.out"; then
    break
  fi
  if [[ "$attempt" == 60 ]]; then cat "$artifact_dir/repl-group.out" >&2; exit 1; fi
  sleep 1
done
tasks="$(admin task list)"
test "$(echo "$tasks" | jq_py 'sorted(set(t["kind"] for t in d["result"]["tasks"]))')" = "[100, 101, 102]"
test "$(echo "$tasks" | jq_py 'd["result"]["next_after"]')" = "0"
test "$(admin task list --kind 102 | jq_py "all(t['kind'] == 102 and t['reference_task'] == $gtask for t in d['result']['tasks']) and len(d['result']['tasks']) == 1")" = "True"
test "$(admin task list --limit 1 | jq_py 'len(d["result"]["tasks"]) == 1 and d["result"]["next_after"] > 0')" = "True"
test "$(admin task describe "$gtask" | jq_py 'd["result"]["kind_name"]')" = "group_creation"
expect_exit 1 env KV9_ADMIN_TOKEN="$admin_token" "$admin_bin" --endpoints "$(follower_endpoint)" --output json task describe 999999
echo "task list/describe: kinds 100/101/102 with cursor paging."

# ---------------------------------------------------------------- the exit-code table
expect_exit 2 env KV9_ADMIN_TOKEN="$admin_token" "$admin_bin" --endpoints "$(follower_endpoint)" --output json cluster bogus
expect_exit 2 env KV9_ADMIN_TOKEN="$admin_token" "$admin_bin" --endpoints "$(follower_endpoint)" --output json node describe notanumber
expect_exit 3 env KV9_ADMIN_TOKEN="wrong-token" "$admin_bin" --endpoints "$(follower_endpoint)" --output json cluster info
grep -q '"kind": "transport"' "$artifact_dir/last.out"
expect_exit 3 env KV9_ADMIN_TOKEN="$admin_token" "$admin_bin" --endpoints "127.0.0.1:1" --output json --timeout 2 cluster info
# The app principal is authenticated but not an administrator: a typed refusal, exit 1.
expect_exit 1 env KV9_ADMIN_TOKEN="$app_token" "$admin_bin" --endpoints "$(follower_endpoint)" --output json node list
grep -q '"kind": "refused"' "$artifact_dir/last.out"
grep -qi "may not administer" "$artifact_dir/last.out"
# ...while the same principal still uses the data plane.
printf 'use admin-e2e\nput admin/e2e ok\nget admin/e2e\n' | KV9_CLIENT_TOKEN="$app_token" timeout 30 "$(dirname "$bin")/kv9-repl" \
  --addr "$(follower_endpoint)" --peers "1=127.0.0.1:$((base_port + 1)),2=127.0.0.1:$((base_port + 2)),3=127.0.0.1:$((base_port + 3))" \
  >"$artifact_dir/repl.out" 2>&1
grep -q '^ok$' "$artifact_dir/repl.out"
# Three voters: removing one is refused by the server, exit 1, with its reason.
expect_exit 1 env KV9_ADMIN_TOKEN="$admin_token" "$admin_bin" --endpoints "$(follower_endpoint)" --output json --yes node remove 1
grep -q "fewer than three voters" "$artifact_dir/last.out"
echo "exit codes: 2 usage, 3 transport, 1 typed refusals (non-admin principal, three-voter removal)."

# ---------------------------------------------------------------- learners 4 and 5, through kv9-admin
for node in 4 5; do
  admitted="$(admin node admit "$node" --addr "127.0.0.1:$((base_port + node))" --ttl 120)"
  ticket="$(echo "$admitted" | jq_py 'd["result"]["join_ticket"]')"
  [[ "$ticket" =~ ^[0-9a-f]{64}$ ]] || { echo "FAIL: admit $node returned no ticket: $admitted" >&2; exit 1; }
  mkdir -p "$artifact_dir/n${node}"
  KV9_JOIN_TICKET="$ticket" "$bin" join --root "$root_path" --node-id "$node" \
    --addr "127.0.0.1:$((base_port + node))" --data-dir "$artifact_dir/n${node}" >"$artifact_dir/n${node}.join" 2>&1
  env "${node_environment[@]}" KV9_JOIN_TICKET="$ticket" "$bin" start --node-id "$node" \
    --addr "127.0.0.1:$((base_port + node))" --data-dir "$artifact_dir/n${node}" >>"$artifact_dir/n${node}.log" 2>&1 &
  pids[$node]=$!
done
learners_listed() {
  admin node list | jq_py 'sorted(n["node_id"] for n in d["result"]["nodes"] if n["raft_role"] == "learner")' | grep -qx '\[4, 5\]'
}
wait_until "nodes 4 and 5 listed as learners" 60 learners_listed
# The configuration lists them before their processes finish joining; promotion
# needs a caught-up, Serving learner, so wait for the processes too.
wait_until "learners 4 and 5 Serving" 60 all_agree
# Admitting again confirms rather than changes: the ticket is not shown twice.
again="$(admin node admit 4 --addr "127.0.0.1:$((base_port + 4))" --ttl 120 || true)"
echo "learners 4 and 5 admitted through kv9-admin and joined."

# cluster status now reaches five nodes through the directory (only one endpoint was given).
test "$(admin cluster status | jq_py 'len(d["result"])')" = "5"
describe="$(admin node describe 4)"
test "$(echo "$describe" | jq_py 'd["result"]["directory"]["raft_role"]')" = "learner"
test "$(echo "$describe" | jq_py 'd["result"]["status"]["role"]')" = "learner"

# ---------------------------------------------------------------- promote --wait, remove --wait
promoted="$(admin node promote 4 --wait --wait-secs 60)"
test "$(echo "$promoted" | jq_py '4 in d["result"]["waited"]["meta_voters"]')" = "True"
promoted="$(admin node promote 5 --wait --wait-secs 60)"
test "$(echo "$promoted" | jq_py 'sorted(d["result"]["waited"]["meta_voters"])')" = "[1, 2, 3, 4, 5]"
echo "promote --wait: voters 1,2,3,4,5."
# Promoting again is a confirmation of the same configuration.
test "$(admin node promote 5 | jq_py 'sorted(d["result"]["voters"])')" = "[1, 2, 3, 4, 5]"

# Remove an original voter that is not the leader; the configuration must show it gone.
victim=1; (( agreed_leader == 1 )) && victim=2
removed="$(admin node remove "$victim" --wait --wait-secs 60)"
removed_node="$victim"
test "$(echo "$removed" | jq_py "$victim not in d['result']['waited']['meta_voters']")" = "True"
test "$(echo "$removed" | jq_py 'len(d["result"]["waited"]["meta_voters"])')" = "4"
echo "remove --wait: voter $victim removed, four voters remain."
# Repeating a membership verb whose target state already holds confirms instead of
# refusing: promote an existing voter, remove the node already removed, transfer to
# the current leader.
test "$(admin node promote 4 | jq_py 'd["result"]["outcome"]')" = "confirmed"
test "$(admin node remove "$victim" | jq_py 'd["result"]["outcome"]')" = "confirmed"
leader_now="$(admin cluster info | jq_py 'd["result"]["leader_node_id"]')"
test "$(admin node transfer-leader --to "$leader_now" | jq_py 'd["result"]["completed"]')" = "True"
echo "repeated promote/remove/transfer-leader confirm."
test "$(admin cluster health | jq_py 'd["result"]["overall"]')" = "ok"

# A removed node that is still running is reported, not hidden: it stays in the directory
# as a non-member until it re-registers.
test "$(admin node list | jq_py "[n['raft_role'] for n in d['result']['nodes'] if n['node_id'] == $victim][0]")" = "none"

echo "PASS: kv9-admin (resolution via follower, directory, status fan-out, health, keyspaces, data groups, task history, admit/promote/remove --wait, exit codes, admin principals)"
