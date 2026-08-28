#!/usr/bin/env bash
set -euo pipefail

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bin="$repo_dir/target/debug/kv9"
artifact_dir="${KV9_MEMBERSHIP_DIR:-$(mktemp -d /tmp/kv9-membership.XXXXXX)}"
base_port="${KV9_BASE_PORT:-$((23000 + ($$ % 1000)))}"
cluster_token="membership-cluster-token"
client_token="membership-client-token"
initial_join="1@127.0.0.1:$((base_port + 1)),2@127.0.0.1:$((base_port + 2)),3@127.0.0.1:$((base_port + 3))"
declare -A pids=()

if [[ ! "$base_port" =~ ^[0-9]+$ ]] || ((base_port < 1024 || base_port + 5 > 65535)); then
  echo "FAIL: KV9_BASE_PORT must leave five valid non-privileged ports" >&2
  exit 2
fi
if [[ -r /proc/sys/net/ipv4/ip_local_port_range ]]; then
  read -r ephemeral_low ephemeral_high </proc/sys/net/ipv4/ip_local_port_range
  if ((base_port + 5 >= ephemeral_low && base_port + 1 <= ephemeral_high)); then
    echo "FAIL: ports $((base_port + 1))-$((base_port + 5)) overlap host ephemeral range ${ephemeral_low}-${ephemeral_high}" >&2
    exit 2
  fi
fi

status_value() {
  local node key file
  node="$1"
  key="$2"
  file="$artifact_dir/n${node}/status"
  test -f "$file" || return 1
  awk -F= -v wanted="$key" '$1 == wanted { print substr($0, length($1) + 2); found=1 } END { if (!found) exit 1 }' "$file"
}

start_node() {
  local node="$1" cluster_id="${2:-}" args=()
  mkdir -p "$artifact_dir/n${node}"
  args=(
    --node-id "$node"
    --addr "127.0.0.1:$((base_port + node))"
    --data-dir "$artifact_dir/n${node}"
    --join "$initial_join"
  )
  if [[ -n "$cluster_id" ]]; then args+=(--cluster-id "$cluster_id"); fi
  KV9_CLUSTER_TOKEN="$cluster_token" KV9_CLIENT_TOKENS="acceptance=$client_token" \
    "$bin" "${args[@]}" >"$artifact_dir/n${node}.log" 2>&1 &
  pids[$node]=$!
}

stop_node() {
  local node signal pid
  node="$1"
  signal="${2:-TERM}"
  pid="${pids[$node]}"
  kill "-$signal" "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  unset 'pids[$node]'
}

stop_all() {
  local node pid
  for node in "${!pids[@]}"; do
    pid="${pids[$node]}"
    if kill -0 "$pid" 2>/dev/null; then kill "$pid" 2>/dev/null || true; fi
  done
  wait 2>/dev/null || true
}
trap stop_all EXIT

wait_until() {
  local label="$1" timeout_seconds="$2"
  shift 2
  local deadline=$((SECONDS + timeout_seconds))
  until "$@"; do
    if ((SECONDS >= deadline)); then
      echo "FAIL: timed out waiting for $label" >&2
      find "$artifact_dir" -name status -type f -print -exec sed -n '1,18p' {} \; >&2 || true
      return 1
    fi
    sleep 0.05
  done
}

leader_id() {
  local node leader pid
  for node in 1 2 3 4 5; do
    [[ -v "pids[$node]" ]] || continue
    pid="${pids[$node]}"
    kill -0 "$pid" 2>/dev/null || continue
    test -f "$artifact_dir/n${node}/status" || continue
    if test "$(status_value "$node" role 2>/dev/null || true)" = leader; then
      leader="$node"
      printf '%s' "$leader"
      return 0
    fi
  done
  return 1
}

membership_converged() {
  local count="$1" voters="$2" learners="$3" node
  for ((node = 1; node <= count; node++)); do
    test "$(status_value "$node" bootstrap_state 2>/dev/null || true)" = Serving || return 1
    test "$(status_value "$node" meta_voters 2>/dev/null || true)" = "$voters" || return 1
    test "$(status_value "$node" meta_learners 2>/dev/null || true)" = "$learners" || return 1
    test -z "$(status_value "$node" fatal 2>/dev/null || true)" || return 1
  done
  test -n "$(leader_id 2>/dev/null || true)"
}

client() {
  timeout 15s env KV9_CLIENT_TOKEN="$client_token" "$bin" client "$@"
}

admit_and_join() {
  local node="$1" cluster_id="$2" leader output
  leader="$(leader_id)"
  output="$(client admit-node \
    --addr "127.0.0.1:$((base_port + leader))" \
    --node-id "$node" \
    --node-addr "127.0.0.1:$((base_port + node))" \
    --ttl-seconds 120)"
  test "$(awk -F= '$1 == "applied_index" {print $2}' <<<"$output")" -gt 0
  start_node "$node" "$cluster_id"
}

promote() {
  local node="$1" leader output conf_index
  leader="$(leader_id)"
  output="$(client promote-node \
    --addr "127.0.0.1:$((base_port + leader))" \
    --node-id "$node")"
  conf_index="$(awk -F= '$1 == "applied_index" {print $2}' <<<"$output")"
  test "$conf_index" -gt 0
}

echo "Building kv9..."
cargo build --quiet --manifest-path "$repo_dir/Cargo.toml"

for node in 1 2 3; do start_node "$node"; done
wait_until "initial three voters Serving" 20 membership_converged 3 "1,2,3" ""
cluster_id="$(status_value 1 cluster_id)"
[[ "$cluster_id" =~ ^[0-9a-f]{32}$ ]]

# Gate 3 sensitivity: knowing the cluster token and public ClusterId is not an
# admission. Before the leader commits node 4's row, discovery/registration
# must leave it visibly outside Serving.
start_node 4 "$cluster_id"
wait_until "unadmitted joiner status" 5 test -f "$artifact_dir/n4/status"
unadmitted_deadline=$((SECONDS + 2))
while ((SECONDS < unadmitted_deadline)); do
  test "$(status_value 4 bootstrap_state 2>/dev/null || true)" != Serving
  kill -0 "${pids[4]}"
  sleep 0.05
done
stop_node 4
mv "$artifact_dir/n4" "$artifact_dir/n4-unadmitted"

# Gate 2 sensitivity: after admission, the same node/address presenting a
# different 128-bit cluster identity must fail closed before registration.
leader="$(leader_id)"
admit_output="$(client admit-node \
  --addr "127.0.0.1:$((base_port + leader))" \
  --node-id 4 \
  --node-addr "127.0.0.1:$((base_port + 4))" \
  --ttl-seconds 120)"
test "$(awk -F= '$1 == "applied_index" {print $2}' <<<"$admit_output")" -gt 0
wrong_cluster="00000000000000000000000000000000"
if test "$wrong_cluster" = "$cluster_id"; then
  wrong_cluster="11111111111111111111111111111111"
fi
start_node 4 "$wrong_cluster"
wrong_deadline=$((SECONDS + 8))
while kill -0 "${pids[4]}" 2>/dev/null && ((SECONDS < wrong_deadline)); do
  test "$(status_value 4 bootstrap_state 2>/dev/null || true)" != Serving
  sleep 0.05
done
if kill -0 "${pids[4]}" 2>/dev/null; then
  echo "FAIL: wrong-cluster joiner did not fail closed" >&2
  exit 1
fi
wait "${pids[4]}" 2>/dev/null || true
unset 'pids[4]'
grep -q "does not match" "$artifact_dir/n4.log"
mv "$artifact_dir/n4" "$artifact_dir/n4-wrong-cluster"

# Admission for node 4 is already committed above; now use the correct id.
start_node 4 "$cluster_id"
wait_until "node 4 registered and caught up as learner" 20 membership_converged 4 "1,2,3" "4"
promote 4
wait_until "node 4 promoted to voter" 20 membership_converged 4 "1,2,3,4" ""

admit_and_join 5 "$cluster_id"
wait_until "node 5 registered and caught up as learner" 20 membership_converged 5 "1,2,3,4" "5"
promote 5
wait_until "five voters converged" 20 membership_converged 5 "1,2,3,4,5" ""

old_leader="$(leader_id)"
stop_node "$old_leader" KILL
new_leader_ready() {
  local leader
  leader="$(leader_id 2>/dev/null || true)"
  test -n "$leader" && test "$leader" -ne "$old_leader"
}
wait_until "five-voter failover" 20 new_leader_ready
new_leader="$(leader_id)"
write_output="$(client create-keyspace \
  --addr "127.0.0.1:$((base_port + new_leader))" \
  --name post-membership-failover \
  --api-type raw)"
write_term="$(awk -F= '$1 == "proposed_term" {print $2}' <<<"$write_output")"
write_index="$(awk -F= '$1 == "proposed_index" {print $2}' <<<"$write_output")"
test "$write_term" -gt 0
test "$write_index" -gt 0

survivors_applied() {
  local node
  for node in 1 2 3 4 5; do
    test "$node" -eq "$old_leader" && continue
    test "$(status_value "$node" applied_term 2>/dev/null || true)" = "$write_term" || return 1
    test "$(status_value "$node" applied_index 2>/dev/null || true)" = "$write_index" || return 1
  done
}
wait_until "post-failover command at exact term/index" 20 survivors_applied

restarted_voter_applied() {
  membership_converged 5 "1,2,3,4,5" "" || return 1
  test "$(status_value "$old_leader" applied_term 2>/dev/null || true)" = "$write_term" || return 1
  test "$(status_value "$old_leader" applied_index 2>/dev/null || true)" = "$write_index"
}

if ((old_leader <= 3)); then
  start_node "$old_leader"
else
  start_node "$old_leader" "$cluster_id"
fi
wait_until "killed voter restart and exact catch-up" 20 restarted_voter_applied

# A full-cluster restart proves the initial-voter and already-admitted joiner
# modes both recover from their durable identity/ConfState without re-admission.
for node in 1 2 3 4 5; do stop_node "$node"; done
for node in 1 2 3; do start_node "$node"; done
for node in 4 5; do start_node "$node" "$cluster_id"; done
all_restarted_exact() {
  local node
  membership_converged 5 "1,2,3,4,5" "" || return 1
  for node in 1 2 3 4 5; do
    test "$(status_value "$node" applied_term 2>/dev/null || true)" = "$write_term" || return 1
    test "$(status_value "$node" applied_index 2>/dev/null || true)" = "$write_index" || return 1
  done
}
wait_until "full five-voter durable restart at exact term/index" 25 all_restarted_exact

for node in 1 2 3 4 5; do
  test "$(status_value "$node" cluster_id)" = "$cluster_id"
  test -s "$artifact_dir/n${node}/raft/raft.log"
  test -s "$artifact_dir/n${node}/catalog.wal"
  test -s "$artifact_dir/n${node}/kv9-initialized"
done

echo "PASS: 3 voters -> admit/register learners -> explicit promotion -> 5-voter failover and full restart"
echo "Artifacts: $artifact_dir"
