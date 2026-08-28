#!/usr/bin/env bash
# Raw KV data path, end to end, across three real processes (task #25).
#
# What this proves, and why it is shaped this way:
#
#   * Reads are leader-only, so "the value is on node 2" cannot be shown by reading
#     node 2 — a follower refuses. Instead the leader is killed and the value is read
#     from the node that *wins the next election*. A node can only serve it after
#     winning if the entry genuinely replicated to it, so failover is the replication
#     evidence. That is stronger than a test-only stale read, and needs no extra API.
#   * Replication and durability are separate claims and get separate assertions:
#     the new leader proves replication; restarting the killed node from its own
#     data-dir proves the WAL persisted it.
#   * Reading an old value on the new leader does not prove the *write* path survived
#     failover, so the script also writes again afterwards.
set -euo pipefail

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bin="$repo_dir/target/debug/kv9"
artifact_dir="${KV9_RAW_E2E_DIR:-$(mktemp -d /tmp/kv9-raw-e2e.XXXXXX)}"
base_port="${KV9_BASE_PORT:-$((23000 + ($$ % 1000)))}"
cluster_token="raw-e2e-cluster-token"
client_token="raw-e2e-client-token"
declare -A pids=()

# Same guardrail as the phase-1 gate: a listener precheck is racy, an outbound
# connection can claim an ephemeral source port between check and bind.
if [[ ! "$base_port" =~ ^[0-9]+$ ]] || (( base_port < 1024 || base_port + 3 > 65535 )); then
  echo "FAIL: KV9_BASE_PORT must leave three valid non-privileged ports" >&2
  exit 2
fi
if [[ -r /proc/sys/net/ipv4/ip_local_port_range ]]; then
  read -r ephemeral_low ephemeral_high </proc/sys/net/ipv4/ip_local_port_range
  if (( base_port + 3 >= ephemeral_low && base_port + 1 <= ephemeral_high )); then
    echo "FAIL: ports $((base_port + 1))-$((base_port + 3)) overlap the host ephemeral range ${ephemeral_low}-${ephemeral_high}" >&2
    exit 2
  fi
fi

status_value() {
  local node="$1" key="$2"
  local file="$artifact_dir/n${node}/status"
  test -f "$file" || return 1
  awk -F= -v wanted="$key" '$1 == wanted { print substr($0, length($1) + 2); found=1 } END { if (!found) exit 1 }' "$file"
}

start_node() {
  local node="$1"
  mkdir -p "$artifact_dir/n${node}"
  KV9_CLUSTER_TOKEN="$cluster_token" KV9_CLIENT_TOKENS="acceptance=$client_token" "$bin" \
    --node-id "$node" \
    --addr "127.0.0.1:$((base_port + node))" \
    --data-dir "$artifact_dir/n${node}" \
    --join "1@127.0.0.1:$((base_port + 1)),2@127.0.0.1:$((base_port + 2)),3@127.0.0.1:$((base_port + 3))" \
    >>"$artifact_dir/n${node}.log" 2>&1 &
  pids[$node]=$!
}

stop_all() {
  local node pid
  for node in "${!pids[@]}"; do
    pid="${pids[$node]}"
    kill -0 "$pid" 2>/dev/null && kill "$pid" 2>/dev/null || true
  done
  wait 2>/dev/null || true
}
trap stop_all EXIT

wait_until() {
  local label="$1" timeout="$2"; shift 2
  local deadline=$((SECONDS + timeout))
  while (( SECONDS < deadline )); do
    if "$@"; then return 0; fi
    sleep 0.05
  done
  echo "FAIL: timed out waiting for $label" >&2
  exit 1
}

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
  test "$leaders" -eq 1
}

# `exclude` matters: a killed node's status file is frozen at its last write, so it
# still claims `role=leader` forever. Reading it as current state is how a dead node
# gets picked as the new leader -- the same "remembered state read as live state" that
# cost this team a day. The caller must name any node it knows to be dead.
leader_node() {
  local exclude="${1:-0}" node
  for node in 1 2 3; do
    test "$node" -eq "$exclude" && continue
    if test "$(status_value "$node" role 2>/dev/null || true)" = leader; then echo "$node"; return 0; fi
  done
  return 1
}

# `timeout` on every client call: a deadlocked server would otherwise hang the whole
# gate forever, and "still running" is indistinguishable from "still running" on a CI
# dashboard. An unbounded experiment does not fail, it disappears.
client() {
  local node="$1"; shift
  KV9_CLIENT_TOKEN="$client_token" timeout 30 "$bin" client "$@" \
    --addr "127.0.0.1:$((base_port + node))"
}

# The status file is a snapshot republished every tick, not a live query. Reading it
# immediately after a write can catch the previous tick and report a position that has
# already advanced in the process. Poll for the value to appear rather than sampling
# once -- the same "snapshot is not live state" mistake this script exists to catch.
applied_reached() {
  local node="$1" want="$2"
  test "$(status_value "$node" applied_index 2>/dev/null || true)" -ge "$want" 2>/dev/null
}

hex() { printf '%s' "$1" | od -An -tx1 -v | tr -d ' \n'; }

echo "Building kv9..."
cargo build --quiet --manifest-path "$repo_dir/Cargo.toml"

echo "Starting three nodes on ports $((base_port + 1))-$((base_port + 3))..."
for node in 1 2 3; do start_node "$node"; done
wait_until "three members Serving behind one leader" 20 all_serving

leader="$(leader_node)"
echo "Leader is n${leader}."

# ---------------------------------------------------------------- keyspace
create_output="$(KV9_CLIENT_TOKEN="$client_token" timeout 30 "$bin" client create-keyspace \
  --addr "127.0.0.1:$((base_port + leader))" --name raw-e2e --api-type raw)"
keyspace="$(awk -F= '$1 == "keyspace_id" { print $2 }' <<<"$create_output")"
test -n "$keyspace" || { echo "FAIL: no keyspace_id in create output" >&2; exit 1; }
echo "Created raw keyspace ${keyspace}."

key_hex="$(hex alpha)"
value_hex="$(hex first-value)"

# ---------------------------------------------------------------- write + read back
client "$leader" raw-put --keyspace "$keyspace" --key-hex "$key_hex" --value-hex "$value_hex" >/dev/null
got="$(client "$leader" raw-get --keyspace "$keyspace" --key-hex "$key_hex")"
test "$got" = "value_hex=${value_hex}" || { echo "FAIL: read-back got '$got'" >&2; exit 1; }

# The write blocks until its own (term,index) applies, and nothing else is writing, so
# the leader's applied position now *is* that write's position.
#
# NOTE: this is derived rather than returned. RawPut's response is `Empty`, so unlike
# CreateKeyspace the client cannot report the exact pair it committed. Deriving it from
# a quiescent status is weaker — it assumes no concurrent writer — and returning the
# position in the response would remove the assumption. Flagged as a follow-up; it needs
# a proto change, which is outside this task's lease.
wait_until "the leader's status to publish the write" 10 applied_reached "$leader" 1
write_term="$(status_value "$leader" applied_term)"
write_index="$(status_value "$leader" applied_index)"
test "$write_index" -gt 0 || { echo "FAIL: applied_index still 0 after write" >&2; exit 1; }
echo "Write applied at (term=${write_term}, index=${write_index})."

scan_output="$(client "$leader" raw-scan --keyspace "$keyspace")"
grep -q "key_hex=${key_hex} value_hex=${value_hex}" <<<"$scan_output" || { echo "FAIL: scan missing the row: $scan_output" >&2; exit 1; }
grep -q "^count=1$" <<<"$scan_output" || { echo "FAIL: scan count wrong: $scan_output" >&2; exit 1; }

# A follower must refuse, and say so in a form a script can branch on.
for node in 1 2 3; do
  test "$node" -eq "$leader" && continue
  set +e
  follower_output="$(client "$node" raw-get --keyspace "$keyspace" --key-hex "$key_hex" 2>&1)"
  follower_rc=$?
  set -e
  test "$follower_rc" -ne 0 || { echo "FAIL: follower n${node} served a read" >&2; exit 1; }
  grep -q "not_leader=true" <<<"$follower_output" || {
    echo "FAIL: follower n${node} refused for the wrong reason: $follower_output" >&2; exit 1; }
done
echo "Followers correctly refuse reads with a redirect hint."

# ---------------------------------------------------------------- failover
echo "Killing leader n${leader}..."
kill -9 "${pids[$leader]}"; wait "${pids[$leader]}" 2>/dev/null || true; unset "pids[$leader]"
old_leader="$leader"

survivors_have_new_leader() {
  local node
  for node in 1 2 3; do
    test "$node" -eq "$old_leader" && continue
    test "$(status_value "$node" role 2>/dev/null || true)" = leader && return 0
  done
  return 1
}
wait_until "a survivor to win the election" 20 survivors_have_new_leader
new_leader="$(leader_node "$old_leader")"
echo "New leader is n${new_leader}."

# Before any new write applies, the new leader's last applied command must be *exactly*
# the pre-failover write. `>=` would be wrong: (term,index) is not a single ordered
# watermark, so a larger term with a smaller index would satisfy `>=` while meaning
# something else entirely.
test "$(status_value "$new_leader" applied_index)" -eq "$write_index" || {
  echo "FAIL: new leader applied_index $(status_value "$new_leader" applied_index) != $write_index" >&2; exit 1; }
test "$(status_value "$new_leader" applied_term)" -eq "$write_term" || {
  echo "FAIL: new leader applied_term $(status_value "$new_leader" applied_term) != $write_term" >&2; exit 1; }

# THE replication evidence: this node could only answer if the entry reached it.
got="$(client "$new_leader" raw-get --keyspace "$keyspace" --key-hex "$key_hex")"
test "$got" = "value_hex=${value_hex}" || { echo "FAIL: post-failover read got '$got'" >&2; exit 1; }
echo "Pre-failover value survived on the new leader."

# Reading an old value proves replication but says nothing about whether the *write*
# path still works after failover.
second_key_hex="$(hex beta)"
second_value_hex="$(hex second-value)"
client "$new_leader" raw-put --keyspace "$keyspace" --key-hex "$second_key_hex" --value-hex "$second_value_hex" >/dev/null
got="$(client "$new_leader" raw-get --keyspace "$keyspace" --key-hex "$second_key_hex")"
test "$got" = "value_hex=${second_value_hex}" || { echo "FAIL: post-failover write not readable, got '$got'" >&2; exit 1; }
wait_until "the new leader's status to publish the post-failover write" 10 \
  applied_reached "$new_leader" $((write_index + 1))
post_failover_term="$(status_value "$new_leader" applied_term)"
post_failover_index="$(status_value "$new_leader" applied_index)"
test "$post_failover_index" -gt "$write_index" || { echo "FAIL: post-failover applied_index $post_failover_index did not advance past $write_index" >&2; exit 1; }
echo "Proposal path still live after failover: (term=${post_failover_term}, index=${post_failover_index})."

# ---------------------------------------------------------------- delete + delete-range
client "$new_leader" raw-delete --keyspace "$keyspace" --key-hex "$key_hex" >/dev/null
got="$(client "$new_leader" raw-get --keyspace "$keyspace" --key-hex "$key_hex")"
test "$got" = "found=false" || { echo "FAIL: delete left '$got'" >&2; exit 1; }

for suffix in a b c; do
  client "$new_leader" raw-put --keyspace "$keyspace" \
    --key-hex "$(hex "range-${suffix}")" --value-hex "$(hex v)" >/dev/null
done
client "$new_leader" raw-delete-range --keyspace "$keyspace" \
  --start-hex "$(hex range-)" --end-hex "$(hex range.)" >/dev/null
scan_output="$(client "$new_leader" raw-scan --keyspace "$keyspace")"
grep -q "^count=1$" <<<"$scan_output" || {
  echo "FAIL: delete-range left the wrong rows: $scan_output" >&2; exit 1; }
grep -q "key_hex=${second_key_hex}" <<<"$scan_output" || { echo "FAIL: surviving row missing: $scan_output" >&2; exit 1; }
# Capture the final position only once the leader has *quiesced*. Polling for "some
# advance" would latch a mid-sequence value: the earlier attempt caught index 10 while
# the delete-range was still applying, then demanded the restarted node match exactly
# 10 -- and failed it for correctly reaching 11. No writes are issued after this point,
# so two identical samples mean the sequence is done.
quiesced() {
  local first second
  first="$(status_value "$new_leader" applied_index 2>/dev/null || true)"
  sleep 0.2
  second="$(status_value "$new_leader" applied_index 2>/dev/null || true)"
  test -n "$first" && test "$first" = "$second" && test "$first" -gt "$post_failover_index"
}
wait_until "the new leader to finish applying the deletes" 15 quiesced
final_term="$(status_value "$new_leader" applied_term)"
final_index="$(status_value "$new_leader" applied_index)"
echo "Delete and delete-range replicated; final position (term=${final_term}, index=${final_index})."

# ---------------------------------------------------------------- restart = durability
echo "Restarting n${old_leader} from its original data-dir..."
start_node "$old_leader"
restarted_caught_up() {
  test "$(status_value "$old_leader" applied_index 2>/dev/null || true)" = "$final_index" \
    && test "$(status_value "$old_leader" applied_term 2>/dev/null || true)" = "$final_term"
}
wait_until "restarted node to reach the exact final (term,index)" 30 restarted_caught_up

test -s "$artifact_dir/n${old_leader}/catalog.wal" || { echo "FAIL: restarted node has no catalog.wal" >&2; exit 1; }
echo "PASS: raw KV replicated through failover, deletes applied, restart caught up to (${final_term},${final_index})"
