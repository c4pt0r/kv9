#!/usr/bin/env bash
# Automatic voter replacement while quorum holds (docs/VOTER-REPLACEMENT.md, phase 1).
#
# Three metadata voters and two credentialed learners. Kill a voter: the leader must
# promote a caught-up learner FIRST and remove the lost voter SECOND, by itself, with
# writes and linearizable reads continuing throughout. Then kill another ORIGINAL
# voter: the group must still serve — the property three fixed voters cannot give.
# Also checks the manual guard: `remove-node` refuses to shrink a three-voter group.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/root_provision.sh"

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
artifact_dir="${KV9_VOTER_E2E_DIR:-$(mktemp -d /tmp/kv9-voter-replacement.XXXXXX)}"
base_port="${KV9_BASE_PORT:-$((27000 + ($$ % 1000)))}"
cluster_token="voter-e2e-cluster-token"
client_token="voter-e2e-client-token"
bootstrap_token="voter-e2e-bootstrap-token"
root_path="$artifact_dir/root.bin"
root_voters="1@127.0.0.1:$((base_port + 1)),2@127.0.0.1:$((base_port + 2)),3@127.0.0.1:$((base_port + 3))"
# The verdict thresholds are configuration; the gate makes them short so that a
# minute-scale run exercises the whole state machine. Production defaults are 300 s
# and 600 s (see runtime/voter_replacement.rs).
lost_secs="${KV9_E2E_LOST_SECS:-4}"
declare -A pids=()

status_value() {
  local node="$1" key="$2" file="$artifact_dir/n${1}/status"
  test -f "$file" || return 1
  awk -F= -v wanted="$key" '$1 == wanted { print substr($0, length($1) + 2); found=1 } END { if (!found) exit 1 }' "$file"
}

# Passed with `env` directly in front of the binary (never through a shell function run
# in the background: `$!` would then be the function's subshell, and killing it would leave
# the node running — exactly the "dead" voter this gate needs to be really dead).
node_environment=(KV9_CLUSTER_TOKEN="$cluster_token" KV9_CLIENT_TOKENS="e2e=$client_token"
  KV9_AUTO_REPLACE_VOTER=1 KV9_VOTER_LOST_SECS="$lost_secs" KV9_VOTER_REPLACE_COOLDOWN_SECS=1)

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
  for f in "$artifact_dir"/n*/status; do echo "--- $f"; grep -E "^(node_id|role|leader_id|meta_voters|meta_learners|voter_replacement)=" "$f"; done >&2 || true
  exit 1
}

live_nodes() { local n; for n in "${!pids[@]}"; do kill -0 "${pids[$n]}" 2>/dev/null && echo "$n"; done | sort -n; }

# All live nodes Serving and agreeing on one leader; prints the leader id.
agreed_leader=""
all_agree() {
  local node leader_id=0 seen
  for node in $(live_nodes); do
    test "$(status_value "$node" bootstrap_state 2>/dev/null || true)" = "Serving" || return 1
    test -z "$(status_value "$node" fatal 2>/dev/null || true)" || return 1
    seen="$(status_value "$node" leader_id 2>/dev/null || true)"
    [[ "$seen" =~ ^[0-9]+$ ]] && (( seen > 0 )) || return 1
    (( leader_id == 0 )) && leader_id="$seen"
    test "$seen" -eq "$leader_id" || return 1
  done
  kill -0 "${pids[$leader_id]}" 2>/dev/null || return 1
  agreed_leader="$leader_id"
}

client() { KV9_CLIENT_TOKEN="$client_token" timeout 30 "$bin" client "$@"; }
leader_client() {
  local out attempt
  for attempt in 1 2 3 4 5; do
    all_agree || { sleep 0.5; continue; }
    if out="$(client "$@" --addr "127.0.0.1:$((base_port + agreed_leader))" 2>&1)"; then
      printf '%s\n' "$out"; return 0
    fi
    printf '%s\n' "$out" >&2
    sleep 0.5
  done
  return 1
}

voters_of() { status_value "$1" meta_voters 2>/dev/null || true; }
has_member() { [[ ",$1," == *",$2,"* ]]; }

echo "Building kv9..."
bin="$(build_fixture_binary "$repo_dir" "$artifact_dir/build.jsonl")"
KV9_BOOTSTRAP_TOKEN="$bootstrap_token" "$bin" root-create --output "$root_path" \
  --voters "$root_voters" --store-incarnations "$(prepare_root_stores "$bin" "$artifact_dir" "$root_voters")" \
  >"$artifact_dir/root-create.log"

echo "Starting voters 1-3 on ports $((base_port + 1))-$((base_port + 3))..."
for node in 1 2 3; do start_voter "$node"; done
wait_until "three voters Serving behind one leader" 30 all_agree
echo "Leader is n${agreed_leader}."

# ---------------------------------------------------------------- learners 4 and 5
for node in 4 5; do
  admit_out="$(leader_client admit-node --node-id "$node" --node-addr "127.0.0.1:$((base_port + node))" --ttl-seconds 120)"
  ticket="$(awk -F= '$1=="join_ticket"{print $2}' <<<"$admit_out")"
  [[ "$ticket" =~ ^[0-9a-f]{64}$ ]] || { echo "FAIL: admit-node $node returned no ticket: $admit_out" >&2; exit 1; }
  mkdir -p "$artifact_dir/n${node}"
  KV9_JOIN_TICKET="$ticket" "$bin" join --root "$root_path" --node-id "$node" \
    --addr "127.0.0.1:$((base_port + node))" --data-dir "$artifact_dir/n${node}" >"$artifact_dir/n${node}.join" 2>&1
  env "${node_environment[@]}" KV9_JOIN_TICKET="$ticket" "$bin" start --node-id "$node" \
    --addr "127.0.0.1:$((base_port + node))" --data-dir "$artifact_dir/n${node}" >>"$artifact_dir/n${node}.log" 2>&1 &
  pids[$node]=$!
done
learners_joined() {
  all_agree || return 1
  local l; l="$(status_value "$agreed_leader" meta_learners 2>/dev/null || true)"
  has_member "$l" 4 && has_member "$l" 5
}
wait_until "learners 4 and 5 joined the metadata group" 60 learners_joined
echo "Learners 4 and 5 are replicating; voters: $(voters_of "$agreed_leader")."

# ---------------------------------------------------------------- data before any fault
create_out="$(leader_client create-keyspace --name voter-e2e --api-type raw)"
keyspace="$(awk -F= '$1=="keyspace_id"{print $2}' <<<"$create_out")"
test -n "$keyspace" || { echo "FAIL: no keyspace_id: $create_out" >&2; exit 1; }
hex() { printf '%s' "$1" | od -An -tx1 -v | tr -d ' \n'; }
leader_client raw-put --keyspace "$keyspace" --key-hex "$(hex before)" --value-hex "$(hex v0)" >/dev/null
echo "Wrote a key with all three original voters alive."

# ---------------------------------------------------------------- manual guard
set +e
guard_out="$(client remove-node --node-id 3 --addr "127.0.0.1:$((base_port + agreed_leader))" 2>&1)"
guard_rc=$?
set -e
if (( guard_rc == 0 )) || [[ "$guard_out" != *"fewer than three voters"* ]]; then
  echo "FAIL: remove-node must refuse to shrink a three-voter group, got rc=$guard_rc: $guard_out" >&2; exit 1
fi
echo "Manual remove-node correctly refused to leave fewer than three voters."

# ---------------------------------------------------------------- first voter loss
pick_victim() { # an ORIGINAL voter (1..3) that is live and not the leader
  local n
  for n in 1 2 3; do
    (( n != agreed_leader )) || continue
    [[ -n "${pids[$n]:-}" ]] && kill -0 "${pids[$n]}" 2>/dev/null && { echo "$n"; return; }
  done
  return 1
}
victim="$(pick_victim)"
echo "Killing original voter n${victim} (leader is n${agreed_leader})..."
kill "${pids[$victim]}"; wait "${pids[$victim]}" 2>/dev/null || true
pgrep -f -- "--node-id $victim --addr 127.0.0.1:$((base_port + victim))" >/dev/null && { echo "FAIL: n${victim} still running after kill" >&2; exit 1; }
unset "pids[$victim]"

replaced() {
  all_agree || return 1
  local v; v="$(voters_of "$agreed_leader")"
  [[ -n "$v" ]] || return 1
  ! has_member "$v" "$1" || return 1                       # the lost voter is gone
  { has_member "$v" 4 || has_member "$v" 5; } || return 1  # a learner took its place
  [[ "$(tr ',' '\n' <<<"$v" | wc -l)" -eq 3 ]]               # and the count is back to three
}
wait_until "n${victim} replaced by a promoted learner (promote first, remove second)" $((lost_secs * 4 + 60)) replaced "$victim"
echo "Voters after the first loss: $(voters_of "$agreed_leader") (events: $(status_value "$agreed_leader" voter_replacement | head -c 400))"

# The promotion must have been applied BEFORE the removal: the leader's event log
# records the steps in order, and no event may name a removal before a promotion.
# The configuration shows the result a pass earlier than the event log settles it.
applied_steps() { # prints the applied steps of the leader's event log, in order
  status_value "$agreed_leader" voter_replacement 2>/dev/null | python3 -c '
import json, sys
s = json.loads(sys.stdin.read() or "{}")
print(" ".join(e["step"] for e in s.get("events", []) if e["outcome"].startswith("applied")))'
}
events_settled() { [[ " $(applied_steps) " == *" remove "* ]]; }
wait_until "the leader settled its promote and remove events" 30 events_settled
steps="$(applied_steps)"
[[ "$steps" == promote*remove* ]] || { echo "FAIL: steps must be promote then remove, got: $steps" >&2; exit 1; }
echo "Replacement steps in order: $steps"

leader_client raw-put --keyspace "$keyspace" --key-hex "$(hex after1)" --value-hex "$(hex v1)" >/dev/null
got="$(leader_client raw-get --keyspace "$keyspace" --key-hex "$(hex before)")"
[[ "$got" == "value_hex=$(hex v0)" ]] || { echo "FAIL: pre-loss value lost after the first replacement: $got" >&2; exit 1; }
echo "Writes and reads continue after the first replacement."

# ---------------------------------------------------------------- second ORIGINAL voter loss
victim2="$(pick_victim)"
echo "Killing another original voter n${victim2} (leader is n${agreed_leader})..."
kill "${pids[$victim2]}"; wait "${pids[$victim2]}" 2>/dev/null || true
pgrep -f -- "--node-id $victim2 --addr 127.0.0.1:$((base_port + victim2))" >/dev/null && { echo "FAIL: n${victim2} still running after kill" >&2; exit 1; }
unset "pids[$victim2]"
second_replaced() {
  all_agree || return 1
  local v; v="$(voters_of "$agreed_leader")"
  [[ -n "$v" ]] || return 1
  ! has_member "$v" "$1" && ! has_member "$v" "$2" || return 1
  has_member "$v" 4 && has_member "$v" 5 || return 1
  [[ "$(tr ',' '\n' <<<"$v" | wc -l)" -eq 3 ]]
}
wait_until "n${victim2} replaced too: both learners now vote" $((lost_secs * 4 + 60)) second_replaced "$victim" "$victim2"
leader_client raw-put --keyspace "$keyspace" --key-hex "$(hex after2)" --value-hex "$(hex v2)" >/dev/null
for k in before after1 after2; do
  got="$(leader_client raw-get --keyspace "$keyspace" --key-hex "$(hex $k)")"
  [[ "$got" == value_hex=* ]] || { echo "FAIL: $k unreadable after two voter losses: $got" >&2; exit 1; }
done
echo "Voters after the second loss: $(voters_of "$agreed_leader"); all three keys readable."
echo "PASS: voter replacement — two of three original voters lost one after another, learners promoted first and lost voters removed second, service never lost (artifacts in $artifact_dir)"
