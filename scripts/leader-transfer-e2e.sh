#!/usr/bin/env bash
# Leader transfer (docs/LEADER-TRANSFER.md): three metadata voters hosting a data group.
# `kv9-admin node transfer-leader --to <follower>` moves metadata leadership in one bounded
# step with writes continuing; `group transfer-leader` moves a data group's leadership;
# a learner, a non-member and the leader itself are refused.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/root_provision.sh"

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
artifact_dir="${KV9_TRANSFER_E2E_DIR:-$(mktemp -d /tmp/kv9-leader-transfer.XXXXXX)}"
base_port="${KV9_BASE_PORT:-$((28500 + ($$ % 400)))}"
cluster_token="transfer-e2e-cluster-token"
admin_token="transfer-e2e-admin-token"
bootstrap_token="transfer-e2e-bootstrap-token"
root_path="$artifact_dir/root.bin"
root_voters="1@127.0.0.1:$((base_port + 1)),2@127.0.0.1:$((base_port + 2)),3@127.0.0.1:$((base_port + 3))"
declare -A pids=()
node_environment=(KV9_CLUSTER_TOKEN="$cluster_token" KV9_CLIENT_TOKENS="admin=$admin_token")

status_value() {
  local node="$1" key="$2" file="$artifact_dir/n${1}/status"
  test -f "$file" || return 1
  awk -F= -v wanted="$key" '$1 == wanted { print substr($0, length($1) + 2); found=1 } END { if (!found) exit 1 }' "$file"
}
start_node() {
  local node="$1" ticket="${2:-}"
  mkdir -p "$artifact_dir/n${node}"
  if [[ -n "$ticket" ]]; then
    KV9_JOIN_TICKET="$ticket" "$bin" join --root "$root_path" --node-id "$node" --addr "127.0.0.1:$((base_port + node))" \
      --data-dir "$artifact_dir/n${node}" >"$artifact_dir/n${node}.join" 2>&1
    env "${node_environment[@]}" KV9_JOIN_TICKET="$ticket" "$bin" start --node-id "$node" --addr "127.0.0.1:$((base_port + node))" \
      --data-dir "$artifact_dir/n${node}" >>"$artifact_dir/n${node}.log" 2>&1 &
  else
    if [[ ! -e "$artifact_dir/n${node}/kv9-store-identity" ]]; then
      KV9_BOOTSTRAP_TOKEN="$bootstrap_token" "$bin" init --root "$root_path" --node-id "$node" \
        --data-dir "$artifact_dir/n${node}" >>"$artifact_dir/n${node}.log" 2>&1
    fi
    env "${node_environment[@]}" "$bin" start --node-id "$node" --addr "127.0.0.1:$((base_port + node))" \
      --data-dir "$artifact_dir/n${node}" >>"$artifact_dir/n${node}.log" 2>&1 &
  fi
  pids[$node]=$!
}
stop_all() { local pid; for pid in "${pids[@]}"; do kill -0 "$pid" 2>/dev/null && kill "$pid" 2>/dev/null || true; done; wait 2>/dev/null || true; }
trap stop_all EXIT
wait_until() {
  local label="$1" timeout="$2"; shift 2
  local deadline=$((SECONDS + timeout))
  while (( SECONDS < deadline )); do if "$@"; then return 0; fi; sleep 0.2; done
  echo "FAIL: timed out waiting for $label" >&2
  for f in "$artifact_dir"/n*/status; do echo "--- $f"; grep -E "^(node_id|role|leader_id|term|meta_voters|meta_learners|fatal)=" "$f"; done >&2 || true
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
admin() { KV9_ADMIN_TOKEN="$admin_token" timeout 60 "$admin_bin" --endpoints "127.0.0.1:$((base_port + 1))" --output json --yes "$@"; }
jq_py() { python3 -c 'import json,sys; t=sys.stdin.read(); d=json.loads(t[t.find("{"):]); print(eval(sys.argv[1], {"d": d}))' "$1"; }
expect_exit() {
  local expected="$1"; shift
  local rc=0; "$@" >"$artifact_dir/last.out" 2>"$artifact_dir/last.err" || rc=$?
  if (( rc != expected )); then echo "FAIL: expected exit $expected, got $rc for: $*" >&2; cat "$artifact_dir/last.out" "$artifact_dir/last.err" >&2; exit 1; fi
}

echo "Building kv9, kv9-admin and kv9-repl..."
bin="$(build_fixture_binary "$repo_dir" "$artifact_dir/build.jsonl")"
cargo build --locked --manifest-path "$repo_dir/Cargo.toml" -p kv9-admin -p kv9-repl --bin kv9-admin --bin kv9-repl >"$artifact_dir/build-tools.log" 2>&1
admin_bin="$(dirname "$bin")/kv9-admin"; repl_bin="$(dirname "$bin")/kv9-repl"
KV9_BOOTSTRAP_TOKEN="$bootstrap_token" "$bin" root-create --output "$root_path" \
  --voters "$root_voters" --store-incarnations "$(prepare_root_stores "$bin" "$artifact_dir" "$root_voters")" >"$artifact_dir/root-create.log"
for node in 1 2 3; do start_node "$node"; done
wait_until "three voters Serving behind one leader" 30 all_agree
echo "Leader is n${agreed_leader}."

# ---------------------------------------------------------------- metadata group
orig_leader="$agreed_leader"
follower=$(( agreed_leader == 1 ? 2 : 1 ))
# Refusals first: the leader itself, a non-member.
expect_exit 1 env KV9_ADMIN_TOKEN="$admin_token" "$admin_bin" --endpoints "127.0.0.1:$((base_port + 1))" --output json --yes node transfer-leader --to "$agreed_leader"
grep -q "transfer to itself" "$artifact_dir/last.out"
expect_exit 1 env KV9_ADMIN_TOKEN="$admin_token" "$admin_bin" --endpoints "127.0.0.1:$((base_port + 1))" --output json --yes node transfer-leader --to 9
grep -q "not a voter" "$artifact_dir/last.out"
# A learner is not a voter either.
admitted="$(admin node admit 4 --addr "127.0.0.1:$((base_port + 4))" --ttl 120)"
ticket="$(echo "$admitted" | jq_py 'd["result"]["join_ticket"]')"
start_node 4 "$ticket"
learner_listed() { admin node list | jq_py 'any(n["node_id"] == 4 and n["raft_role"] == "learner" for n in d["result"]["nodes"])' | grep -qx True; }
wait_until "node 4 listed as a learner" 60 learner_listed
wait_until "learner 4 Serving" 60 all_agree
expect_exit 1 env KV9_ADMIN_TOKEN="$admin_token" "$admin_bin" --endpoints "127.0.0.1:$((base_port + 1))" --output json --yes node transfer-leader --to 4
grep -q "not a voter" "$artifact_dir/last.out"

# The transfer: one bounded step, observed by every node, writes continue.
before_term="$(status_value "$agreed_leader" term)"
moved="$(admin node transfer-leader --to "$follower" --wait)"
test "$(echo "$moved" | jq_py 'd["result"]["completed"]')" = "True"
test "$(echo "$moved" | jq_py 'd["result"]["leader"]')" = "$follower"
test "$(echo "$moved" | jq_py 'd["result"]["previous_leader"]')" = "$orig_leader"
all_on_new_leader() { all_agree && test "$agreed_leader" = "$follower"; }
wait_until "every node sees node $follower leading" 30 all_on_new_leader
after_term="$(status_value "$follower" term)"
test "$after_term" -gt "$before_term"
echo "metadata leadership moved to n$follower (term $before_term → $after_term)."
created="$(admin keyspace create --name after-transfer)"
ks="$(echo "$created" | jq_py 'd["result"]["id"]')"
printf 'use after-transfer\nput a 1\nget a\n' | KV9_CLIENT_TOKEN="$admin_token" timeout 30 "$repl_bin" --addr "127.0.0.1:$((base_port + 1))" \
  --peers "1=127.0.0.1:$((base_port + 1)),2=127.0.0.1:$((base_port + 2)),3=127.0.0.1:$((base_port + 3))" >"$artifact_dir/repl.out" 2>&1
grep -qx 1 "$artifact_dir/repl.out"
# Transferring back works too (the previous leader is a caught-up voter again).
back="$(admin node transfer-leader --to "$orig_leader" --wait)"
test "$(echo "$back" | jq_py 'd["result"]["completed"]')" = "True"
echo "metadata leadership moved back; keyspace $ks written through both leaders."

# ---------------------------------------------------------------- a data group
group="$(admin group create --voters 1,2,3 --label transfer-e2e)"
gtask="$(echo "$group" | jq_py 'd["result"]["creation_task"]')"
# A group is addressable through the raw directory once it serves a keyspace.
admin keyspace create --name grouped --group "$gtask" >/dev/null
group_leader() {
  admin group describe "$gtask" | jq_py '[r["node_id"] for r in d["result"]["replica_views"] if (r["observed"] or {}).get("role") == "Leader" and (r["observed"] or {}).get("state") == "active"]'
}
group_elected() { [[ "$(group_leader)" =~ ^\[[0-9]+\]$ ]]; }
wait_until "data group $gtask active behind a leader" 60 group_elected
# The public route (what the transfer verb resolves the group through) is published a
# moment after activation: probe until a write lands.
route_served() {
  printf 'use grouped\nput p 1\n' | KV9_CLIENT_TOKEN="$admin_token" timeout 20 "$repl_bin" --addr "127.0.0.1:$((base_port + 1))" \
    --peers "1=127.0.0.1:$((base_port + 1)),2=127.0.0.1:$((base_port + 2)),3=127.0.0.1:$((base_port + 3))" 2>&1 | grep -q applied
}
wait_until "grouped keyspace routable" 60 route_served
gleader="$(group_leader | tr -d '[]')"
gtarget=$(( gleader == 1 ? 2 : 1 ))
expect_exit 1 env KV9_ADMIN_TOKEN="$admin_token" "$admin_bin" --endpoints "127.0.0.1:$((base_port + 1))" --output json --yes group transfer-leader "$gtask" --to 4
gmoved="$(admin group transfer-leader "$gtask" --to "$gtarget")"
test "$(echo "$gmoved" | jq_py 'd["result"]["completed"]')" = "True"
test "$(echo "$gmoved" | jq_py 'd["result"]["leader"]')" = "$gtarget"
group_on_target() { test "$(group_leader)" = "[$gtarget]"; }
wait_until "data group led by node $gtarget" 30 group_on_target
echo "data group $gtask leadership moved from n$gleader to n$gtarget."

echo "PASS: leader transfer (metadata group to a follower and back with writes continuing; data group to a replica; learner, non-member and self refused)"
