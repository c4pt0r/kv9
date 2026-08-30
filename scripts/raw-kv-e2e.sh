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
bootstrap_token="raw-e2e-bootstrap-token"
root_path="$artifact_dir/root.bin"
root_voters="1@127.0.0.1:$((base_port + 1)),2@127.0.0.1:$((base_port + 2)),3@127.0.0.1:$((base_port + 3))"
declare -A pids=()

# Same guardrail as the phase-1 gate: a listener precheck is racy, an outbound
# connection can claim an ephemeral source port between check and bind.
if [[ ! "$base_port" =~ ^[0-9]+$ ]] || (( base_port < 1024 || base_port + 3 > 65535 )); then
  echo "FAIL: KV9_BASE_PORT must leave three valid non-privileged ports" >&2
  exit 2
fi

KV9_BOOTSTRAP_TOKEN="$bootstrap_token" "$bin" root-create --output "$root_path" \
  --voters "$root_voters" >"$artifact_dir/root-create.log"
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
  if [[ ! -e "$artifact_dir/n${node}/kv9-store-identity" ]]; then
    KV9_BOOTSTRAP_TOKEN="$bootstrap_token" "$bin" init --root "$root_path" --node-id "$node" \
      --data-dir "$artifact_dir/n${node}" >>"$artifact_dir/n${node}.log" 2>&1
  fi
  KV9_CLUSTER_TOKEN="$cluster_token" KV9_CLIENT_TOKENS="acceptance=$client_token" "$bin" \
    start \
    --node-id "$node" \
    --addr "127.0.0.1:$((base_port + node))" \
    --data-dir "$artifact_dir/n${node}" \
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

# Publishes the agreed leader in `agreed_leader` as a side effect. That value is the
# whole point of this check and it is already computed here; re-deriving it afterwards
# by scanning for a self-reported `role=leader` discards the cross-node agreement one
# line after establishing it, and can pick a deposed-but-not-yet-stepped-down node --
# which still says `role=leader` about itself while every follower already names
# someone else. Source and freshness are separate defects needing separate fixes: this
# is the source half; `leader_client` below is the freshness half.
agreed_leader=""
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

# `leader_node()` used to live here: it returned the first node whose own status said
# `role=leader`, with an `exclude` argument so a caller could name a node it knew to be
# dead. It is deleted rather than left unused, because a leader chosen from one node's
# self-report is exactly what this script was red on -- a deposed node that has not yet
# stepped down still claims leadership, and a killed node's status file is frozen
# claiming it forever. Both callers now derive the leader from cross-node agreement on
# `leader_id` (`all_serving` before the kill, `survivors_agree_on_new_leader` after it),
# which needs no exclude list: a dead node cannot be agreed upon by the survivors.
# Leaving the helper in place would leave the weak source one call site away.

# `timeout` on every client call: a deadlocked server would otherwise hang the whole
# gate forever, and "still running" is indistinguishable from "still running" on a CI
# dashboard. An unbounded experiment does not fail, it disappears.
client() {
  local node="$1"; shift
  KV9_CLIENT_TOKEN="$client_token" timeout 30 "$bin" client "$@" \
    --addr "127.0.0.1:$((base_port + node))"
}

# A cached leader is only a point-in-time observation. During startup the cluster may
# legitimately elect a newer leader between two client calls. Follow a typed NotLeader
# hint and retry only that known-not-applied outcome; deadlines and all other failures
# remain loud because their write outcome may be unknown.
leader_reply=""
leader_client() {
  local output rc hinted attempt
  for attempt in 1 2 3 4 5; do
    if output="$(client "$leader" "$@" 2>&1)"; then
      leader_reply="$output"
      return 0
    else
      rc=$?
    fi
    if [[ "$output" =~ not_leader=true[[:space:]]+leader_node_id=([1-3]) ]]; then
      hinted="${BASH_REMATCH[1]}"
      if [[ "$hinted" != "$leader" ]]; then
        leader="$hinted"
        continue
      fi
    fi
    # Publish the reply on a FINAL failure too. The negative cases below need to read
    # the real refusal text: if they only see a non-zero status they will accept a
    # NotLeader raised by an unrelated election as proof that the context gate refused
    # them -- a false green that asserts nothing about the gate under test.
    leader_reply="$output"
    printf '%s\n' "$output" >&2
    return "$rc"
  done
  echo "FAIL: leader changed during five consecutive client attempts" >&2
  return 1
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

# Take the value `all_serving` just proved all three nodes agree on, not a fresh scan
# for whoever calls themselves leader. This is what the previous run got wrong: it
# resolved n3 from a single self-report and then addressed every later request there,
# while all three status files ended up naming n2.
leader="$agreed_leader"
test -n "$leader" || { echo "FAIL: all_serving passed without publishing an agreed leader" >&2; exit 1; }
echo "Leader is n${leader}."

# ---------------------------------------------------------------- keyspace
leader_client create-keyspace --name raw-e2e --api-type raw
create_output="$leader_reply"
keyspace="$(awk -F= '$1 == "keyspace_id" { print $2 }' <<<"$create_output")"
test -n "$keyspace" || { echo "FAIL: no keyspace_id in create output" >&2; exit 1; }
echo "Created raw keyspace ${keyspace}."

key_hex="$(hex alpha)"
value_hex="$(hex first-value)"

# ------------------------------------------------- context gate (real path)
# The gate is only worth having if it refuses the wrong context, so check that it does
# through the same gRPC path a client uses -- not by unit-testing the predicate alone.

# An unknown keyspace must be refused, not silently written into a keyspace that will
# exist later.
# Routed through `leader_client` so an election cannot answer for the gate: a bare
# non-zero status is satisfied just as well by a NotLeader redirect as by the context
# gate refusing, and that would be a false green -- the assertion would pass while
# proving nothing about the thing under test. Following the redirect first, then
# checking the refusal text, keeps the failure attributable.
# The keyspace id is a variable so the request and the expected message cannot drift
# apart: pinning the id means an error *about a different keyspace* cannot satisfy this.
unknown_keyspace=999
set +e
leader_client raw-get --keyspace "$unknown_keyspace" --key-hex "$key_hex"
unknown_rc=$?
set -e
unknown_output="$leader_reply"
test "$unknown_rc" -ne 0 || { echo "FAIL: unknown keyspace was served: $unknown_output" >&2; exit 1; }
# Assert the POSITIVE text, not merely the absence of one wrong reason. `rc != 0` plus
# "not a redirect" is still satisfied by a connection failure, a deadline, an auth
# failure, or MetaNotReady -- every one of which would pass while proving nothing about
# the context gate. Substring taken verbatim from 5/5 real runs, narrowed to the part
# that identifies this gate and this keyspace.
grep -q "keyspace KeyspaceId(${unknown_keyspace}) not found" <<<"$unknown_output" || {
  echo "FAIL: unknown-keyspace refusal did not come from the context gate." >&2
  echo "      wanted substring: keyspace KeyspaceId(${unknown_keyspace}) not found" >&2
  echo "      full reply: $unknown_output" >&2
  exit 1; }

# A txn keyspace must refuse raw writes: Percolator expects its own lock/write structure
# there, and raw bytes would corrupt that silently.
leader_client create-keyspace --name raw-e2e-txn --api-type txn
txn_out="$leader_reply"
txn_keyspace="$(awk -F= '$1 == "keyspace_id" { print $2 }' <<<"$txn_out")"
test -n "$txn_keyspace" || { echo "FAIL: could not create the txn keyspace" >&2; exit 1; }
set +e
leader_client raw-put --keyspace "$txn_keyspace" --key-hex "$key_hex" --value-hex "$value_hex"
mismatch_rc=$?
set -e
mismatch_output="$leader_reply"
test "$mismatch_rc" -ne 0 || {
  echo "FAIL: raw write accepted into a txn keyspace: $mismatch_output" >&2; exit 1; }
# Same rule as above, and pinned to the txn keyspace we actually created.
grep -q "api type mismatch for keyspace KeyspaceId(${txn_keyspace})" <<<"$mismatch_output" || {
  echo "FAIL: txn-mismatch refusal did not come from the api-type gate." >&2
  echo "      wanted substring: api type mismatch for keyspace KeyspaceId(${txn_keyspace})" >&2
  echo "      full reply: $mismatch_output" >&2
  exit 1; }

# Control: a valid context on the same path answers normally (the key is not written
# yet, so `found=false` IS the successful answer). Without this the two refusals above
# would be consistent with "everything fails".
leader_client raw-get --keyspace "$keyspace" --key-hex "$key_hex"
control="$leader_reply"
test "$control" = "found=false" || {
  echo "FAIL: control read broke, the refusals prove nothing: $control" >&2; exit 1; }
echo "Context gate refuses unknown and txn keyspaces; raw keyspace still served."


# ---------------------------------------------------------------- write + read back
leader_client raw-put --keyspace "$keyspace" --key-hex "$key_hex" --value-hex "$value_hex"
put_output="$leader_reply"
leader_client raw-get --keyspace "$keyspace" --key-hex "$key_hex"
got="$leader_reply"
test "$got" = "value_hex=${value_hex}" || { echo "FAIL: read-back got '$got'" >&2; exit 1; }

# The position comes from the write's own response. Inferring it from a quiescent status
# was not merely weaker, it was not evidence: `applied_reached leader 1` returns instantly
# because CreateKeyspace already pushed the index past 1, and any concurrent command moves
# the same number — so the script could assert against somebody else's write.
write_term="$(awk -F= '$1 == "applied_term" { print $2 }' <<<"$put_output")"
write_index="$(awk -F= '$1 == "applied_index" { print $2 }' <<<"$put_output")"
test "$write_index" -gt 0 || { echo "FAIL: applied_index still 0 after write" >&2; exit 1; }
echo "Write applied at (term=${write_term}, index=${write_index})."

leader_client raw-scan --keyspace "$keyspace"
scan_output="$leader_reply"
grep -q "key_hex=${key_hex} value_hex=${value_hex}" <<<"$scan_output" || { echo "FAIL: scan missing the row: $scan_output" >&2; exit 1; }
grep -q "^count=1$" <<<"$scan_output" || { echo "FAIL: scan count wrong: $scan_output" >&2; exit 1; }

# A follower must refuse, and say so in a form a script can branch on. Role is sampled
# both before and after the RPC: if an election overlaps the request, a successful read
# from the new leader must not be misclassified as a follower serving data.
declare -A follower_refused=()
stable_followers_refuse() {
  local node before after output rc count=0
  for node in 1 2 3; do
    before="$(status_value "$node" role 2>/dev/null || true)"
    [[ "$before" == follower ]] || continue
    if output="$(client "$node" raw-get --keyspace "$keyspace" --key-hex "$key_hex" 2>&1)"; then
      rc=0
    else
      rc=$?
    fi
    after="$(status_value "$node" role 2>/dev/null || true)"
    [[ "$after" == follower ]] || continue
    if (( rc == 0 )); then
      echo "FAIL: node n${node} served a read while it remained follower" >&2
      exit 1
    fi
    if ! grep -Eq 'not_leader=true[[:space:]]+leader_node_id=[1-3]' <<<"$output"; then
      echo "FAIL: follower n${node} refused for the wrong reason: $output" >&2
      exit 1
    fi
    follower_refused[$node]=1
  done
  for node in "${!follower_refused[@]}"; do count=$((count + 1)); done
  (( count >= 2 ))
}
wait_until "two stable followers to refuse reads with redirect hints" 20 stable_followers_refuse
echo "Followers correctly refuse reads with a redirect hint."

# The refusal checks deliberately contact followers and may overlap another election.
# Re-establish an authoritative successful leader read immediately before choosing the
# process whose death is meant to exercise failover.
leader_client raw-get --keyspace "$keyspace" --key-hex "$key_hex"
test "$leader_reply" = "value_hex=${value_hex}" || {
  echo "FAIL: pre-kill leader read got '$leader_reply'" >&2; exit 1; }

# ---------------------------------------------------------------- failover
echo "Killing leader n${leader}..."
kill -9 "${pids[$leader]}"; wait "${pids[$leader]}" 2>/dev/null || true; unset "pids[$leader]"
old_leader="$leader"

# Same source rule as before the kill: the survivors must AGREE on leader_id, and
# exactly one of them may call itself leader. Picking the first survivor that claims
# `role=leader` is the weak signal this script was red on -- and after an intentional
# kill it is weaker still, because the dead node's status file is frozen mid-term and
# the two survivors can briefly disagree while the election settles.
agreed_new_leader=""
survivors_agree_on_new_leader() {
  local node leaders=0 leader_id=0 seen
  for node in 1 2 3; do
    test "$node" -eq "$old_leader" && continue
    test -z "$(status_value "$node" fatal 2>/dev/null || true)" || return 1
    seen="$(status_value "$node" leader_id 2>/dev/null || true)"
    test -n "$seen" && test "$seen" -gt 0 2>/dev/null || return 1
    # The survivors must not still be naming the node we just killed.
    test "$seen" -ne "$old_leader" || return 1
    (( leader_id == 0 )) && leader_id="$seen"
    test "$seen" -eq "$leader_id" || return 1
    test "$(status_value "$node" role 2>/dev/null || true)" = leader && leaders=$((leaders + 1))
  done
  test "$leaders" -eq 1 || return 1
  agreed_new_leader="$leader_id"
}
wait_until "both survivors to agree on one new leader" 20 survivors_agree_on_new_leader
new_leader="$agreed_new_leader"
test -n "$new_leader" || { echo "FAIL: survivor agreement passed without publishing a leader" >&2; exit 1; }
# Everything after the failover addresses the leader through `leader_client`, which
# keeps `$leader` current by following typed redirects. Hand the agreed value over
# rather than continuing to address the frozen `$new_leader`: an election during the
# post-failover writes would otherwise reproduce exactly the failure this card fixes,
# one stage later.
leader="$new_leader"
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
#
# This is the load-bearing assertion of the whole script, and it is the one that is
# cheapest to delete by accident, because the lines around it look like they already
# cover the same ground. They do not:
#
#   the (term,index) checks above    guard "did the node report an honest position"
#   this line                        guards "did the data actually reach another node"
#
# A write path that skipped Raft and wrote its own local engine passes every earlier
# check in this script -- write, read-back, scan, follower refusal -- and fails only
# here. That is not hypothetical: it was verified by mutation. Replace the
# propose -> wait_applied in NodeRuntime::commit_batch with a direct local engine
# write and this exact line reddens with `post-failover read got 'found=false'`,
# while everything before it stays green. Restoring the propose restores the pass.
#
# So: delete this and the script still looks thorough, still exits 0 on a build that
# has silently stopped replicating. If you are changing it, change it into something
# that still requires the value to be served by a node that was never the writer.
leader_client raw-get --keyspace "$keyspace" --key-hex "$key_hex"
got="$leader_reply"
test "$got" = "value_hex=${value_hex}" || { echo "FAIL: post-failover read got '$got'" >&2; exit 1; }
echo "Pre-failover value survived on the new leader."

# Reading an old value proves replication but says nothing about whether the *write*
# path still works after failover.
second_key_hex="$(hex beta)"
second_value_hex="$(hex second-value)"
leader_client raw-put --keyspace "$keyspace" --key-hex "$second_key_hex" --value-hex "$second_value_hex"
second_put_output="$leader_reply"
leader_client raw-get --keyspace "$keyspace" --key-hex "$second_key_hex"
got="$leader_reply"
test "$got" = "value_hex=${second_value_hex}" || { echo "FAIL: post-failover write not readable, got '$got'" >&2; exit 1; }
post_failover_term="$(awk -F= '$1 == "applied_term" { print $2 }' <<<"$second_put_output")"
post_failover_index="$(awk -F= '$1 == "applied_index" { print $2 }' <<<"$second_put_output")"
test "$post_failover_index" -gt "$write_index" || { echo "FAIL: post-failover applied_index $post_failover_index did not advance past $write_index" >&2; exit 1; }
echo "Proposal path still live after failover: (term=${post_failover_term}, index=${post_failover_index})."

# ---------------------------------------------------------------- delete + delete-range
leader_client raw-delete --keyspace "$keyspace" --key-hex "$key_hex"
leader_client raw-get --keyspace "$keyspace" --key-hex "$key_hex"
got="$leader_reply"
test "$got" = "found=false" || { echo "FAIL: delete left '$got'" >&2; exit 1; }

for suffix in a b c; do
  leader_client raw-put --keyspace "$keyspace" \
    --key-hex "$(hex "range-${suffix}")" --value-hex "$(hex v)"
done
leader_client raw-delete-range --keyspace "$keyspace" \
  --start-hex "$(hex range-)" --end-hex "$(hex range.)"
leader_client raw-scan --keyspace "$keyspace"
scan_output="$leader_reply"
grep -q "^count=1$" <<<"$scan_output" || {
  echo "FAIL: delete-range left the wrong rows: $scan_output" >&2; exit 1; }
grep -q "key_hex=${second_key_hex}" <<<"$scan_output" || { echo "FAIL: surviving row missing: $scan_output" >&2; exit 1; }
# Capture the final position only once the leader has *quiesced*. Polling for "some
# advance" would latch a mid-sequence value: the earlier attempt caught index 10 while
# the delete-range was still applying, then demanded the restarted node match exactly
# 10 -- and failed it for correctly reaching 11. No writes are issued after this point,
# so two identical samples mean the sequence is done.
# Sample the leader `leader_client` actually ended up talking to, not the node that won
# the election several operations ago. If a redirect moved us, `$new_leader` is a stale
# name and its applied_index answers a question about the wrong process.
quiesced() {
  local first second
  first="$(status_value "$leader" applied_index 2>/dev/null || true)"
  sleep 0.2
  second="$(status_value "$leader" applied_index 2>/dev/null || true)"
  test -n "$first" && test "$first" = "$second" && test "$first" -gt "$post_failover_index"
}
wait_until "the new leader to finish applying the deletes" 15 quiesced
final_term="$(status_value "$leader" applied_term)"
final_index="$(status_value "$leader" applied_index)"
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
