#!/usr/bin/env bash
set -euo pipefail

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bin="$repo_dir/target/debug/kv9"
artifact_dir="${KV9_ACCEPTANCE_DIR:-$(mktemp -d /tmp/kv9-phase1-final.XXXXXX)}"
root_artifact_dir="$artifact_dir"
# Keep the default listener range below Linux's usual ephemeral range. A
# listener precheck alone is racy: an outbound connection can claim an
# ephemeral source port between the check and bind, producing a misleading
# `Address already in use` acceptance failure.
base_port="${KV9_BASE_PORT:-$((22000 + ($$ % 1000)))}"
meta_node_count="${KV9_META_NODES:-3}"
cluster_token="phase1-cluster-token"
client_token="phase1-client-token"
declare -A pids=()

if [[ ! "$meta_node_count" =~ ^[0-9]+$ ]] || (( meta_node_count < 3 || meta_node_count % 2 == 0 )); then
  echo "FAIL: KV9_META_NODES must be an odd integer >= 3 (acceptance covers 3 and 5)" >&2
  exit 2
fi
port_span=$((meta_node_count > 3 ? meta_node_count : 3))
if [[ ! "$base_port" =~ ^[0-9]+$ ]] || (( base_port < 1024 || base_port + port_span > 65535 )); then
  echo "FAIL: KV9_BASE_PORT must leave ${port_span} valid non-privileged ports" >&2
  exit 2
fi
if [[ -r /proc/sys/net/ipv4/ip_local_port_range ]]; then
  read -r ephemeral_low ephemeral_high </proc/sys/net/ipv4/ip_local_port_range
  if (( base_port + port_span >= ephemeral_low && base_port + 1 <= ephemeral_high )); then
    echo "FAIL: ports $((base_port + 1))-$((base_port + port_span)) overlap the host ephemeral range ${ephemeral_low}-${ephemeral_high}" >&2
    exit 2
  fi
fi

join_set() {
  local count="$1" node declaration=""
  for ((node = 1; node <= count; node++)); do
    if [[ -n "$declaration" ]]; then declaration+=","; fi
    declaration+="${node}@127.0.0.1:$((base_port + node))"
  done
  printf '%s' "$declaration"
}

cluster_join="$(join_set "$meta_node_count")"
negative_join="$(join_set 3)"
expected_voters="$(seq -s, 1 "$meta_node_count")"

status_value() {
  local node="$1" key="$2"
  local file="$artifact_dir/n${node}/status"
  test -f "$file" || return 1
  awk -F= -v wanted="$key" '$1 == wanted { print substr($0, length($1) + 2); found=1 } END { if (!found) exit 1 }' "$file"
}

start_node() {
  local node="$1"
  local declared_join="${2:-$cluster_join}"
  local port=$((base_port + node))
  mkdir -p "$artifact_dir/n${node}"
  KV9_CLUSTER_TOKEN="$cluster_token" KV9_CLIENT_TOKENS="acceptance=$client_token" "$bin" \
    --node-id "$node" \
    --addr "127.0.0.1:${port}" \
    --data-dir "$artifact_dir/n${node}" \
    --join "$declared_join" \
    >"$artifact_dir/n${node}.log" 2>&1 &
  pids[$node]=$!
}

stop_all() {
  local node pid
  for node in "${!pids[@]}"; do
    pid="${pids[$node]}"
    if kill -0 "$pid" 2>/dev/null; then
      kill "$pid" 2>/dev/null || true
    fi
  done
  wait 2>/dev/null || true
}
trap stop_all EXIT

wait_until() {
  local label="$1" timeout="$2"
  shift 2
  local deadline=$((SECONDS + timeout))
  until "$@"; do
    if (( SECONDS >= deadline )); then
      echo "FAIL: timed out waiting for $label" >&2
      find "$artifact_dir" -name status -type f -print -exec sed -n '1,12p' {} \; >&2 || true
      return 1
    fi
    sleep 0.05
  done
}

all_serving() {
  local node leaders=0 leader_id=0 seen
  for ((node = 1; node <= meta_node_count; node++)); do
    test "$(status_value "$node" bootstrap_state 2>/dev/null || true)" = "Serving" || return 1
    test -z "$(status_value "$node" fatal 2>/dev/null || true)" || return 1
    test "$(status_value "$node" meta_voters)" = "$expected_voters" || return 1
    test "$(status_value "$node" applied_index)" -gt 0 || return 1
    seen="$(status_value "$node" leader_id)"
    test "$seen" -gt 0 || return 1
    if (( leader_id == 0 )); then leader_id="$seen"; fi
    test "$seen" -eq "$leader_id" || return 1
    if test "$(status_value "$node" role)" = leader; then leaders=$((leaders + 1)); fi
    case "$(status_value "$node" role)" in leader|follower) ;; *) return 1 ;; esac
  done
  test "$leaders" -eq 1
}

survivors_elected() {
  local old_leader="$1" old_commit="$2" node leaders=0 leader_id=0 seen
  for ((node = 1; node <= meta_node_count; node++)); do
    test "$node" -eq "$old_leader" && continue
    test "$(status_value "$node" bootstrap_state 2>/dev/null || true)" = "Serving" || return 1
    test -z "$(status_value "$node" fatal 2>/dev/null || true)" || return 1
    test "$(status_value "$node" meta_voters)" = "$expected_voters" || return 1
    seen="$(status_value "$node" leader_id)"
    test "$seen" -gt 0 || return 1
    test "$seen" -ne "$old_leader" || return 1
    if (( leader_id == 0 )); then leader_id="$seen"; fi
    test "$seen" -eq "$leader_id" || return 1
    if test "$(status_value "$node" role)" = leader; then
      test "$(status_value "$node" raft_committed)" -gt "$old_commit" || return 1
      leaders=$((leaders + 1))
    fi
  done
  test "$leaders" -eq 1
}

all_caught_up() {
  all_serving || return 1
  local leader leader_commit leader_applied node
  leader="$(status_value 1 leader_id)"
  leader_commit="$(status_value "$leader" raft_committed)"
  leader_applied="$(status_value "$leader" applied_index)"
  for ((node = 1; node <= meta_node_count; node++)); do
    test "$(status_value "$node" raft_committed)" -ge "$leader_commit" || return 1
    test "$(status_value "$node" applied_index)" -ge "$leader_applied" || return 1
  done
}

echo "Building kv9..."
cargo build --quiet --manifest-path "$repo_dir/Cargo.toml"

# Negative gate: one member of a declared three-voter set may listen, but it
# must never turn silence into an 'uninitialized' quorum.
artifact_dir="$root_artifact_dir/quorum-negative"
start_node 1 "$negative_join"
wait_until "single node status surface" 5 test -f "$artifact_dir/n1/status"
negative_deadline=$((SECONDS + 2))
while (( SECONDS < negative_deadline )); do
  if test "$(status_value 1 bootstrap_state 2>/dev/null || true)" = "Serving"; then
    echo "FAIL: one of three voters bootstrapped without quorum" >&2
    exit 1
  fi
  kill -0 "${pids[1]}"
  sleep 0.05
done
kill "${pids[1]}"
wait "${pids[1]}" 2>/dev/null || true
unset 'pids[1]'

# Negative gate: a syntactically valid answer from an overlapping but different
# voter declaration is not a vote for this cluster. Node 1 declares {1,2,3};
# the process at address 3 declares {1,2,9}. Neither may use the other's answer.
artifact_dir="$root_artifact_dir/fingerprint-negative"
start_node 1 "$negative_join"
mkdir -p "$artifact_dir/n9"
KV9_CLUSTER_TOKEN="$cluster_token" KV9_CLIENT_TOKENS="acceptance=$client_token" "$bin" \
  --node-id 9 \
  --addr "127.0.0.1:$((base_port + 3))" \
  --data-dir "$artifact_dir/n9" \
    --join "1@127.0.0.1:$((base_port + 1)),2@127.0.0.1:$((base_port + 2)),9@127.0.0.1:$((base_port + 3))" \
  >"$artifact_dir/n9.log" 2>&1 &
pids[9]=$!
wait_until "overlapping-declaration status surfaces" 5 test -f "$artifact_dir/n9/status"
fingerprint_deadline=$((SECONDS + 2))
while (( SECONDS < fingerprint_deadline )); do
  for node in 1 9; do
    if test "$(status_value "$node" bootstrap_state 2>/dev/null || true)" = "Serving"; then
      echo "FAIL: mismatched voter-set fingerprint counted as bootstrap evidence" >&2
      exit 1
    fi
    kill -0 "${pids[$node]}"
  done
  sleep 0.05
done
kill "${pids[1]}" "${pids[9]}"
wait "${pids[1]}" 2>/dev/null || true
wait "${pids[9]}" 2>/dev/null || true
unset 'pids[1]'
unset 'pids[9]'

# Use fresh data dirs after deliberate non-pristine negative runs.
artifact_dir="$root_artifact_dir/cluster"
mkdir -p "$artifact_dir"
for ((node = 1; node <= meta_node_count; node++)); do
  start_node "$node"
done
wait_until "${meta_node_count} meta voters Serving behind one leader" 20 all_serving

old_leader="$(status_value 1 leader_id)"
old_commit="$(status_value "$old_leader" raft_committed)"
old_pid="${pids[$old_leader]}"
kill -9 "$old_pid"
wait "$old_pid" 2>/dev/null || true
unset 'pids[$old_leader]'

wait_until "survivor failover and new-term commit" 15 survivors_elected "$old_leader" "$old_commit"
new_leader=0
for ((node = 1; node <= meta_node_count; node++)); do
  test "$node" -eq "$old_leader" && continue
  if test "$(status_value "$node" role)" = leader; then new_leader="$node"; fi
done
test "$new_leader" -ne 0

# A no-op election barrier proves consensus liveness but never reaches the state
# machine. Submit a real public gRPC mutation after failover and correlate its
# exact (term,index) on every surviving apply loop.
create_output="$(KV9_CLIENT_TOKEN="$client_token" "$bin" client create-keyspace \
  --addr "127.0.0.1:$((base_port + new_leader))" \
  --name "post-failover" \
  --api-type raw)"
proposal_term="$(awk -F= '$1 == "proposed_term" { print $2 }' <<<"$create_output")"
proposal_index="$(awk -F= '$1 == "proposed_index" { print $2 }' <<<"$create_output")"
test "$proposal_term" -gt 0
test "$proposal_index" -gt 0

post_failover_applied() {
  local node
  for ((node = 1; node <= meta_node_count; node++)); do
    test "$node" -eq "$old_leader" && continue
    test "$(status_value "$node" applied_index)" -eq "$proposal_index" || return 1
    test "$(status_value "$node" applied_term)" -eq "$proposal_term" || return 1
  done
}
wait_until "post-failover gRPC mutation applied at exact term/index" 15 post_failover_applied

start_node "$old_leader"
wait_until "killed member restart and raft/catalog catch-up" 15 all_caught_up
test "$(status_value "$old_leader" applied_index)" -eq "$proposal_index"
test "$(status_value "$old_leader" applied_term)" -eq "$proposal_term"

for ((node = 1; node <= meta_node_count; node++)); do
  test -s "$artifact_dir/n${node}/raft/raft.log"
  test -s "$artifact_dir/n${node}/catalog.wal"
  test -s "$artifact_dir/n${node}/kv9-initialized"
done
echo "PASS: quorum fencing, ${meta_node_count}-voter meta bootstrap, leader kill/failover, and durable restart/catch-up"
echo "Artifacts: $root_artifact_dir"
for ((node = 1; node <= meta_node_count; node++)); do
  echo "node $node: role=$(status_value "$node" role) voters=$(status_value "$node" meta_voters) leader=$(status_value "$node" leader_id) term=$(status_value "$node" term) committed=$(status_value "$node" raft_committed) applied=$(status_value "$node" applied_index) state=$(status_value "$node" bootstrap_state)"
done
