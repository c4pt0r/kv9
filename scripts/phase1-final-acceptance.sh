#!/usr/bin/env bash
set -euo pipefail

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bin="$repo_dir/target/debug/kv9"
artifact_dir="${KV9_ACCEPTANCE_DIR:-$(mktemp -d /tmp/kv9-phase1-final.XXXXXX)}"
root_artifact_dir="$artifact_dir"
base_port="${KV9_BASE_PORT:-$((33000 + ($$ % 1000)))}"
declare -A pids=()

status_value() {
  local node="$1" key="$2"
  local file="$artifact_dir/n${node}/status"
  test -f "$file" || return 1
  awk -F= -v wanted="$key" '$1 == wanted { print substr($0, length($1) + 2); found=1 } END { if (!found) exit 1 }' "$file"
}

start_node() {
  local node="$1"
  local port=$((base_port + node))
  mkdir -p "$artifact_dir/n${node}"
  "$bin" \
    --node-id "$node" \
    --addr "127.0.0.1:${port}" \
    --data-dir "$artifact_dir/n${node}" \
    --join "1@127.0.0.1:$((base_port + 1)),2@127.0.0.1:$((base_port + 2)),3@127.0.0.1:$((base_port + 3))" \
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
  for node in 1 2 3; do
    test "$(status_value "$node" bootstrap_state 2>/dev/null || true)" = "Serving" || return 1
    test -z "$(status_value "$node" fatal 2>/dev/null || true)" || return 1
    test "$(status_value "$node" applied_index)" -gt 0 || return 1
    seen="$(status_value "$node" leader_id)"
    test "$seen" -gt 0 || return 1
    if (( leader_id == 0 )); then leader_id="$seen"; fi
    test "$seen" -eq "$leader_id" || return 1
    if test "$(status_value "$node" role)" = leader; then leaders=$((leaders + 1)); fi
  done
  test "$leaders" -eq 1
}

survivors_elected() {
  local old_leader="$1" old_commit="$2" node leaders=0 leader_id=0 seen
  for node in 1 2 3; do
    test "$node" -eq "$old_leader" && continue
    test "$(status_value "$node" bootstrap_state 2>/dev/null || true)" = "Serving" || return 1
    test -z "$(status_value "$node" fatal 2>/dev/null || true)" || return 1
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
  for node in 1 2 3; do
    test "$(status_value "$node" raft_committed)" -ge "$leader_commit" || return 1
    test "$(status_value "$node" applied_index)" -ge "$leader_applied" || return 1
  done
}

echo "Building kv9..."
cargo build --quiet --manifest-path "$repo_dir/Cargo.toml"

# Negative gate: one member of a declared three-voter set may listen, but it
# must never turn silence into an 'uninitialized' quorum.
artifact_dir="$root_artifact_dir/quorum-negative"
start_node 1
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
start_node 1
mkdir -p "$artifact_dir/n9"
"$bin" \
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
start_node 1
start_node 2
start_node 3
wait_until "three members Serving behind one leader" 15 all_serving

old_leader="$(status_value 1 leader_id)"
old_commit="$(status_value "$old_leader" raft_committed)"
old_pid="${pids[$old_leader]}"
kill -9 "$old_pid"
wait "$old_pid" 2>/dev/null || true
unset 'pids[$old_leader]'

wait_until "survivor failover and new-term commit" 15 survivors_elected "$old_leader" "$old_commit"
new_leader=0
for node in 1 2 3; do
  test "$node" -eq "$old_leader" && continue
  if test "$(status_value "$node" role)" = leader; then new_leader="$node"; fi
done
test "$new_leader" -ne 0
start_node "$old_leader"
wait_until "killed member restart and raft/catalog catch-up" 15 all_caught_up

for node in 1 2 3; do
  test -s "$artifact_dir/n${node}/raft/raft.log"
  test -s "$artifact_dir/n${node}/catalog.wal"
  test -s "$artifact_dir/n${node}/kv9-initialized"
done
echo "PASS: quorum fencing, 3-process bootstrap, leader kill/failover, and durable restart/catch-up"
echo "Artifacts: $root_artifact_dir"
for node in 1 2 3; do
  echo "node $node: role=$(status_value "$node" role) leader=$(status_value "$node" leader_id) term=$(status_value "$node" term) committed=$(status_value "$node" raft_committed) applied=$(status_value "$node" applied_index) state=$(status_value "$node" bootstrap_state)"
done
