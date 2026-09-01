#!/usr/bin/env bash
# Partition typed-exclusivity E2E (task #28 layer b; the process half of the
# #27 item-4 raw-read closure). Layer allocation is deliberate and NARROW:
#
#   * This script asserts TYPED EXCLUSIVITY ONLY: during isolation, a public
#     raw read against the isolated self-believed leader fails as ONE OF the
#     named typed families — `read_unconfirmed=true phase=(quorum|apply)` or
#     `not_leader=true` — and NEVER as a transport-shaped failure. "The read
#     failed" is an aggregate three paths satisfy; the third (connection
#     errors) is green even with ReadIndex unimplemented, which is exactly
#     why it does not count here.
#   * NO phase assertions (pre/post-deposition). Phase precision belongs to
#     the in-proc layer, which pins ticks; asserting phases here would flake
#     exactly when CI is slow and masquerade as a ReadIndex bug.
#   * The matcher itself carries a NEGATIVE CONTROL: a pure transport failure
#     (dial to a dead port — no server, so no server-sent fields) must NOT
#     match. Without this, the matcher could silently widen until "any gRPC
#     error" passes — the aggregate coming back in disguise.
#
# The isolation mechanism is the PartitionMask file protocol (kv9-raft
# testing feature; the binary must be built with --features partition-testing):
# per-node `testing-partition` files name the peer ids that node is cut from,
# written atomically (tmp + mv); healing is ONLY the spelled token
# `connected`. The mask cuts RAFT transport only — client gRPC still reaches
# the isolated leader, which is the point: the leader must REFUSE typed, not
# vanish.
set -euo pipefail

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bin="$repo_dir/target/debug/kv9"
artifact_dir="${KV9_PARTITION_E2E_DIR:-$(mktemp -d /tmp/kv9-partition-read-e2e.XXXXXX)}"
base_port="${KV9_BASE_PORT:-$((27000 + ($$ % 1000)))}"
cluster_token="partition-e2e-cluster-token"
client_token="partition-e2e-client-token"
bootstrap_token="partition-e2e-bootstrap-token"
root_path="$artifact_dir/root.bin"
root_voters="1@127.0.0.1:$((base_port + 1)),2@127.0.0.1:$((base_port + 2)),3@127.0.0.1:$((base_port + 3))"
declare -A pids=()

if [[ ! "$base_port" =~ ^[0-9]+$ ]] || (( base_port < 1024 || base_port + 3 > 65535 )); then
  echo "FAIL: KV9_BASE_PORT must leave three valid non-privileged ports" >&2
  exit 2
fi
if [[ -r /proc/sys/net/ipv4/ip_local_port_range ]]; then
  read -r ephemeral_low ephemeral_high </proc/sys/net/ipv4/ip_local_port_range
  if (( base_port + 3 >= ephemeral_low && base_port + 1 <= ephemeral_high )); then
    echo "FAIL: ports overlap the host ephemeral range" >&2
    exit 2
  fi
fi

# The mask surface must actually be compiled in, or every "cut" below is a
# silent no-op and the whole gate green-washes: the binary is required to
# have been built with --features partition-testing. There is no runtime
# probe for a compiled-out feature, so the harness builds it explicitly.
( cd "$repo_dir" && cargo build --features partition-testing ) >"$artifact_dir/build.log" 2>&1 \
  || { cat "$artifact_dir/build.log" >&2; echo "FAIL: partition-testing build failed" >&2; exit 1; }

KV9_BOOTSTRAP_TOKEN="$bootstrap_token" "$bin" root-create --output "$root_path" \
  --voters "$root_voters" >"$artifact_dir/root-create.log"

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
  # Every node reads its OWN data dir for the mask file: per-node masks are
  # what makes the cut symmetric-by-construction below.
  KV9_TESTING_PARTITION_DIR="$artifact_dir/n${node}" \
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

# Atomic mask write: tmp + mv, per the protocol's layer-1 discipline. A raw
# `>` can be read torn, and a torn body parses to "keep the last mask" —
# fail-closed for the node, fail-confusing for the harness.
write_mask() {
  local node="$1" body="$2"
  printf '%s\n' "$body" >"$artifact_dir/n${node}/testing-partition.tmp"
  mv "$artifact_dir/n${node}/testing-partition.tmp" "$artifact_dir/n${node}/testing-partition"
}

hex() { printf '%s' "$1" | od -An -tx1 | tr -d ' \n'; }
k_hex="$(hex k)"
v1_hex="$(hex v1)"
v2_hex="$(hex v2)"

client() {
  local node="$1"; shift
  KV9_CLIENT_TOKEN="$client_token" timeout 30 "$bin" client "$@" \
    --addr "127.0.0.1:$((base_port + node))" --keyspace "$keyspace_id"
}

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

leader_reply=""
leader_client() {
  local output rc hinted attempt leader="$agreed_leader"
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
    leader_reply="$output"
    printf '%s\n' "$output" >&2
    return "$rc"
  done
  echo "FAIL: leader changed during five consecutive client attempts" >&2
  exit 1
}

# THE MATCHER. One function, used for the positive assertion AND probed by
# the negative control: an outcome passes only if it carries a SERVER-SENT
# machine field of the two families. Transport failures print prose from the
# local client ("client request failed: ... connect ... / status: ..."), and
# carry neither field — presence of a field proves the SERVER answered with
# a verdict.
is_typed_refusal() {
  local output="$1"
  # WHOLE-LINE exact machine records only (grep -x): a prose line that merely
  # CONTAINS a marker substring, or a phase word extended past the two
  # protocol values (phase=quorumjunk), must not pass — otherwise future
  # generic output could impersonate typed by inclusion. The near-miss
  # controls below red if these anchors are ever loosened.
  grep -Eqx 'read_unconfirmed=true phase=(quorum|apply)' <<<"$output" && return 0
  grep -Eqx 'not_leader=true leader_node_id=([0-9]+|unknown)' <<<"$output" && return 0
  return 1
}

echo "== matcher self-test: near-miss negatives and exact positives (no server involved)"
# Near-misses: protocol-shaped but not protocol. Each must be REJECTED —
# loosening the whole-line anchors reds here, before any cluster exists.
for near_miss in \
  'read_unconfirmed=true phase=quorumjunk' \
  'client request failed: not_leader=true-ish' \
  'prefix not_leader=true leader_node_id=2 suffix'; do
  if is_typed_refusal "$near_miss"; then
    echo "FAIL: matcher accepted a near-miss non-protocol line: $near_miss" >&2
    exit 1
  fi
done
# Exact records must be ACCEPTED — without this half, a matcher that rejects
# everything would sail through every negative control above.
for exact_record in \
  'read_unconfirmed=true phase=quorum' \
  'read_unconfirmed=true phase=apply' \
  'not_leader=true leader_node_id=2' \
  'not_leader=true leader_node_id=unknown'; do
  if ! is_typed_refusal "$exact_record"; then
    echo "FAIL: matcher rejected an exact protocol record: $exact_record" >&2
    exit 1
  fi
done

echo "== forming 3-voter cluster"
for n in 1 2 3; do start_node "$n"; done
wait_until "all three Serving with one agreed leader" 60 all_serving
leader="$agreed_leader"

echo "== baseline: raw keyspace + a committed value readable via the leader"
keyspace_id=""
KV9_CLIENT_TOKEN="$client_token" timeout 30 "$bin" client create-keyspace --name part-e2e \
  --api-type raw --addr "127.0.0.1:$((base_port + leader))" >"$artifact_dir/create.log" 2>&1 \
  || { cat "$artifact_dir/create.log" >&2; echo "FAIL: create-keyspace" >&2; exit 1; }
keyspace_id="$(awk -F= '$1 == "keyspace_id" {print $2}' "$artifact_dir/create.log")"
test -n "$keyspace_id" || { echo "FAIL: no keyspace id in create output" >&2; exit 1; }

leader_client raw-put --key-hex "$k_hex" --value-hex "$v1_hex"
leader_client raw-get --key-hex "$k_hex"
[[ "$leader_reply" == *"$v1_hex"* ]] || { echo "FAIL: baseline read did not see v1" >&2; exit 1; }

echo "== negative control: the matcher must REJECT a pure transport failure"
# A dead port: no server, therefore no server-sent field of either family.
dead_port=$((base_port + 9))
if output="$(KV9_CLIENT_TOKEN="$client_token" timeout 10 "$bin" client raw-get --key-hex "$k_hex" \
  --addr "127.0.0.1:${dead_port}" --keyspace "$keyspace_id" 2>&1)"; then
  echo "FAIL: control read against a dead port unexpectedly succeeded" >&2
  exit 1
fi
if is_typed_refusal "$output"; then
  echo "FAIL: the matcher accepted a pure transport failure — the typed-exclusivity" >&2
  echo "      assertion below would be satisfiable with ReadIndex unimplemented" >&2
  printf '%s\n' "$output" >&2
  exit 1
fi

echo "== cutting the leader from both followers (symmetric, per-node masks)"
isolated="$leader"
followers=()
for n in 1 2 3; do [[ "$n" != "$isolated" ]] && followers+=("$n"); done
write_mask "$isolated" "${followers[0]},${followers[1]}"
write_mask "${followers[0]}" "$isolated"
write_mask "${followers[1]}" "$isolated"

echo "== isolated leader must refuse reads TYPED, never serve, never transport-error"
# Until the mask engages (next tick) a read may still succeed; the FIRST
# non-success outcome must already be typed, and once one is observed the
# refusal must be STABLE — five consecutive typed refusals, no success, no
# transport shape. "Fails throughout isolation" without phase-pinning.
typed_seen=0
observe_deadline=$((SECONDS + 60))
while (( typed_seen < 5 )); do
  if (( SECONDS >= observe_deadline )); then
    echo "FAIL: timed out collecting five consecutive typed refusals from the isolated leader" >&2
    exit 1
  fi
  if output="$(client "$isolated" raw-get --key-hex "$k_hex" 2>&1)"; then
    if (( typed_seen > 0 )); then
      echo "FAIL: isolated leader SERVED a read after already refusing typed — the" >&2
      echo "      refusal must hold for the entire isolation" >&2
      exit 1
    fi
    sleep 0.2
    continue
  fi
  if is_typed_refusal "$output"; then
    typed_seen=$((typed_seen + 1))
    # Scene evidence only — which family fired is TIMING (pre/post
    # deposition) and is deliberately NOT asserted here; phase precision
    # belongs to the in-proc layer. The log answers "what did we actually
    # observe" without turning it into a flaky requirement.
    printf '%s
' "$output" >>"$artifact_dir/typed-refusals.log"
    continue
  fi
  echo "FAIL: isolated leader failed with a NON-typed outcome — transport errors and" >&2
  echo "      prose do not satisfy the read promise; the typed family is the contract" >&2
  printf '%s\n' "$output" >&2
  exit 1
done

echo "== majority side elects and serves the committed value"
new_leader=""
majority_agrees() {
  local a b la lb
  a="${followers[0]}"; b="${followers[1]}"
  la="$(status_value "$a" leader_id 2>/dev/null || echo 0)"
  lb="$(status_value "$b" leader_id 2>/dev/null || echo 0)"
  [[ "$la" == "$lb" ]] || return 1
  [[ "$la" == "${followers[0]}" || "$la" == "${followers[1]}" ]] || return 1
  new_leader="$la"
}
wait_until "majority agrees on a new leader among the connected pair" 60 majority_agrees
if output="$(client "$new_leader" raw-get --key-hex "$k_hex" 2>&1)"; then
  [[ "$output" == *"$v1_hex"* ]] || { echo "FAIL: majority leader served wrong value: $output" >&2; exit 1; }
else
  echo "FAIL: majority leader refused the read: $output" >&2
  exit 1
fi

echo "== healing: the spelled token, and only it"
for n in 1 2 3; do write_mask "$n" "connected"; done
wait_until "cluster reconverges on one leader after heal" 60 all_serving
leader_client raw-put --key-hex "$k_hex" --value-hex "$v2_hex"
leader_client raw-get --key-hex "$k_hex"
[[ "$leader_reply" == *"$v2_hex"* ]] || { echo "FAIL: post-heal write/read did not roundtrip v2" >&2; exit 1; }

echo "observed refusal families (informational, not asserted):"
sort "$artifact_dir/typed-refusals.log" | uniq -c | sed 's/^/  /'
echo "PASS: isolated leader refuses reads typed (read_unconfirmed/not_leader), transport errors excluded, majority serves, heal restores"
