#!/usr/bin/env bash
# Phase-1 creation-authority and durable-root acceptance.
set -euo pipefail

bin="${KV9_BIN:-./target/debug/kv9}"
[ -x "$bin" ] || { echo "FAIL: $bin is not executable" >&2; exit 1; }
base="${KV9_BASE_PORT:-28300}"
(( base > 1024 && base + 10 < 32768 )) || {
  echo "FAIL: ports $((base + 1))-$((base + 10)) must be non-privileged and below the ephemeral range" >&2
  exit 1
}

artifact="$(mktemp -d /tmp/kv9-root-e2e.XXXXXX)"
pids=""
cleanup() {
  local rc=$?
  for pid in $pids; do kill "$pid" 2>/dev/null || true; done
  if (( rc == 0 )); then
    rm -rf "$artifact"
  else
    echo "FAIL: preserving root-trust evidence at $artifact" >&2
  fi
}
trap cleanup EXIT

export KV9_CLUSTER_TOKEN=root-e2e-cluster
export KV9_CLIENT_TOKENS=admin=root-e2e-client
export KV9_CLIENT_TOKEN=root-e2e-client
bootstrap_token=root-e2e-bootstrap
root="$artifact/root.bin"
voters="1@127.0.0.1:$((base+1)),2@127.0.0.1:$((base+2)),3@127.0.0.1:$((base+3))"

status_value() {
  local node="$1" key="$2"
  local file="$artifact/n${node}/status"
  test -f "$file" || return 1
  awk -F= -v wanted="$key" '$1 == wanted { print substr($0, length($1) + 2); found=1 } END { if (!found) exit 1 }' "$file"
}

wait_until() {
  local label="$1"; shift
  local attempt
  for attempt in $(seq 1 120); do
    if "$@"; then return 0; fi
    sleep 0.25
  done
  echo "FAIL: timed out waiting for $label" >&2
  find "$artifact" -name status -type f -print -exec cat {} \; >&2 || true
  return 1
}

all_serving() {
  local node leader=0 seen digest="" generation=""
  for node in 1 2 3; do
    [ "$(status_value "$node" bootstrap_state 2>/dev/null || true)" = Serving ] || return 1
    [ -z "$(status_value "$node" fatal 2>/dev/null || true)" ] || return 1
    seen="$(status_value "$node" leader_id 2>/dev/null || true)"
    [ -n "$seen" ] && (( seen > 0 )) || return 1
    if (( leader == 0 )); then leader="$seen"; fi
    [ "$seen" = "$leader" ] || return 1
    seen="$(status_value "$node" root_digest 2>/dev/null || true)"
    [ -n "$seen" ] || return 1
    if [ -z "$digest" ]; then digest="$seen"; fi
    [ "$seen" = "$digest" ] || return 1
    seen="$(status_value "$node" bootstrap_generation 2>/dev/null || true)"
    [ -n "$seen" ] || return 1
    if [ -z "$generation" ]; then generation="$seen"; fi
    [ "$seen" = "$generation" ] || return 1
  done
  printf '%s\n' "$leader"
}

node4_learner() {
  [ "$(status_value 4 bootstrap_state 2>/dev/null || true)" = Serving ] || return 1
  [ -z "$(status_value 4 fatal 2>/dev/null || true)" ] || return 1
  [[ ",$(status_value 4 meta_learners 2>/dev/null || true)," == *,4,* ]]
}

node_serving() {
  [ "$(status_value "$1" bootstrap_state 2>/dev/null || true)" = Serving ] &&
    [ -z "$(status_value "$1" fatal 2>/dev/null || true)" ]
}

node4_voter_everywhere() {
  local node
  for node in 1 2 3 4; do
    [ "$(status_value "$node" bootstrap_state 2>/dev/null || true)" = Serving ] || return 1
    [[ ",$(status_value "$node" meta_voters 2>/dev/null || true)," == *,4,* ]] || return 1
  done
}

wrong_root_was_rejected() {
  local state observation
  kill -0 "$n9_pid" 2>/dev/null || return 1
  state="$(status_value 9 bootstrap_state 2>/dev/null)" || return 1
  observation="$(status_value 9 discovery_seed_1 2>/dev/null)" || return 1
  [ "$state" != Serving ] &&
    [[ "$observation" =~ ,attempts=[1-9][0-9]* ]] &&
    [[ "$observation" =~ ,rejected_root_identity=[1-9][0-9]* ]] &&
    [[ "$observation" == *"last=rejected_root_identity"* ]]
}

# Creation is an explicit command. Legacy empty-directory startup must fail
# before it can open Raft or mint identity.
if "$bin" --node-id 8 --addr "127.0.0.1:$((base+8))" --data-dir "$artifact/legacy" \
  >"$artifact/legacy.out" 2>&1; then
  echo "FAIL: legacy implicit bootstrap unexpectedly succeeded" >&2
  exit 1
fi
grep -q 'explicit root-create, init, join, start, or client command required' "$artifact/legacy.out"
[ ! -e "$artifact/legacy/raft" ] || { echo "FAIL: rejected legacy startup opened Raft" >&2; exit 1; }

KV9_BOOTSTRAP_TOKEN="$bootstrap_token" "$bin" root-create --output "$root" --voters "$voters" \
  >"$artifact/root-create.out"
expected_digest="$(awk -F'[ =]' '{for(i=1;i<=NF;i++) if($i=="root_digest") print $(i+1)}' "$artifact/root-create.out")"
expected_generation="$(awk -F'[ =]' '{for(i=1;i<=NF;i++) if($i=="bootstrap_generation") print $(i+1)}' "$artifact/root-create.out")"
[ -n "$expected_digest" ] && [ -n "$expected_generation" ]

# A wrong creation credential must leave no durable identity behind.
if KV9_BOOTSTRAP_TOKEN=wrong "$bin" init --root "$root" --node-id 1 --data-dir "$artifact/wrong-token" \
  >"$artifact/wrong-token.out" 2>&1; then
  echo "FAIL: wrong bootstrap credential initialized a store" >&2
  exit 1
fi
grep -q 'bootstrap credential does not match' "$artifact/wrong-token.out"
[ ! -e "$artifact/wrong-token/kv9-root-descriptor" ]
[ ! -e "$artifact/wrong-token/kv9-store-identity" ]

for node in 1 2 3; do
  KV9_BOOTSTRAP_TOKEN="$bootstrap_token" "$bin" init --root "$root" --node-id "$node" \
    --data-dir "$artifact/n${node}" >"$artifact/n${node}.init"
done

# A durable store identity cannot be rebound to another node id.
if "$bin" start --node-id 2 --addr "127.0.0.1:$((base+1))" --data-dir "$artifact/n1" \
  >"$artifact/rebind.out" 2>&1; then
  echo "FAIL: durable node identity was rebound" >&2
  exit 1
fi
grep -q 'does not match durable store identity' "$artifact/rebind.out"

for node in 1 2 3; do
  "$bin" start --node-id "$node" --addr "127.0.0.1:$((base+node))" \
    --data-dir "$artifact/n${node}" >"$artifact/n${node}.log" 2>&1 &
  pids="$pids $!"
done
leader=""
for _ in $(seq 1 120); do if leader="$(all_serving)"; then break; fi; leader=""; sleep 0.25; done
[ -n "$leader" ] || { echo "FAIL: root-certified voters did not reach Serving" >&2; exit 1; }
[ "$(status_value 1 root_digest)" = "$expected_digest" ]
[ "$(status_value 1 bootstrap_generation)" = "$expected_generation" ]

# Exact-root restart keeps the same identity and rejoins without re-init.
victim=$(( leader == 1 ? 2 : 1 ))
victim_pid="$(awk -v n="$victim" '{print $(n)}' <<<"$pids")"
kill -9 "$victim_pid" 2>/dev/null || true
wait "$victim_pid" 2>/dev/null || true
"$bin" start --node-id "$victim" --addr "127.0.0.1:$((base+victim))" \
  --data-dir "$artifact/n${victim}" >"$artifact/n${victim}.restart.log" 2>&1 &
pids="$pids $!"
wait_until 'exact-root restart Serving' node_serving "$victim"
[ "$(status_value "$victim" root_digest)" = "$expected_digest" ]

# Admission returns a one-time ticket. A correctly shaped but wrong ticket
# cannot consume the committed admission; the same durable store then joins
# with the issued ticket and is promoted only after learner catch-up.
admit_out="$artifact/admit.out"
"$bin" client admit-node --addr "127.0.0.1:$((base+leader))" --node-id 4 \
  --node-addr "127.0.0.1:$((base+4))" --ttl-seconds 120 >"$admit_out"
ticket="$(awk -F= '$1=="join_ticket"{print $2}' "$admit_out")"
[[ "$ticket" =~ ^[0-9a-f]{64}$ ]] || { echo "FAIL: admit-node returned no 64-hex ticket" >&2; exit 1; }
KV9_JOIN_TICKET="$ticket" "$bin" join --root "$root" --node-id 4 \
  --addr "127.0.0.1:$((base+4))" --data-dir "$artifact/n4" >"$artifact/n4.join"

KV9_JOIN_TICKET="$(printf '0%.0s' $(seq 1 64))" "$bin" start --node-id 4 \
  --addr "127.0.0.1:$((base+4))" --data-dir "$artifact/n4" >"$artifact/n4.wrong-ticket.log" 2>&1 &
wrong_pid=$!; pids="$pids $wrong_pid"
sleep 2
[ "$(status_value 4 bootstrap_state 2>/dev/null || true)" != Serving ] || {
  echo "FAIL: wrong join ticket reached Serving" >&2; exit 1;
}
kill "$wrong_pid" 2>/dev/null || true
wait "$wrong_pid" 2>/dev/null || true

KV9_JOIN_TICKET="$ticket" "$bin" start --node-id 4 --addr "127.0.0.1:$((base+4))" \
  --data-dir "$artifact/n4" >"$artifact/n4.log" 2>&1 &
n4_pid=$!; pids="$pids $n4_pid"
wait_until 'credentialed node joined as learner' node4_learner

leader="$(status_value 1 leader_id)"
"$bin" client promote-node --addr "127.0.0.1:$((base+leader))" --node-id 4 \
  >"$artifact/promote.out"
wait_until 'joined learner promoted on all members' node4_voter_everywhere

# Once the admission is consumed, a fresh disk may not reuse node 4 even if
# it steals the old ticket and canonical address: the first registration
# bound the committed node row to the original StoreIncarnation.
kill "$n4_pid" 2>/dev/null || true
wait "$n4_pid" 2>/dev/null || true
mv "$artifact/n4" "$artifact/n4-original"
KV9_JOIN_TICKET="$ticket" "$bin" join --root "$root" --node-id 4 \
  --addr "127.0.0.1:$((base+4))" --data-dir "$artifact/n4" >"$artifact/n4-replacement.join"
KV9_JOIN_TICKET="$ticket" "$bin" start --node-id 4 --addr "127.0.0.1:$((base+4))" \
  --data-dir "$artifact/n4" >"$artifact/n4-replacement.log" 2>&1 &
replacement_pid=$!; pids="$pids $replacement_pid"
sleep 2
[ "$(status_value 4 bootstrap_state 2>/dev/null || true)" != Serving ] || {
  echo "FAIL: replacement store reused a consumed node identity" >&2; exit 1;
}
kill "$replacement_pid" 2>/dev/null || true
wait "$replacement_pid" 2>/dev/null || true
mv "$artifact/n4" "$artifact/n4-replacement"
mv "$artifact/n4-original" "$artifact/n4"
"$bin" start --node-id 4 --addr "127.0.0.1:$((base+4))" --data-dir "$artifact/n4" \
  >"$artifact/n4.original-restart.log" 2>&1 &
n4_pid=$!; pids="$pids $n4_pid"
wait_until 'original store incarnation restarted' node_serving 4

# Admit node 9 first so membership authentication opens; only then can this
# fixture prove the discovery root-identity gate itself rejects a different
# root with an overlapping seed.
leader="$(status_value 1 leader_id)"
"$bin" client admit-node --addr "127.0.0.1:$((base+leader))" --node-id 9 \
  --node-addr "127.0.0.1:$((base+9))" --ttl-seconds 120 >"$artifact/admit-n9.out"
wrong_root="$artifact/wrong-root.bin"
KV9_BOOTSTRAP_TOKEN=other-root "$bin" root-create --output "$wrong_root" \
  --voters "1@127.0.0.1:$((base+1)),9@127.0.0.1:$((base+9))" >"$artifact/wrong-root-create.out"
KV9_BOOTSTRAP_TOKEN=other-root "$bin" init --root "$wrong_root" --node-id 9 \
  --data-dir "$artifact/n9" >"$artifact/n9.init"
"$bin" start --node-id 9 --addr "127.0.0.1:$((base+9))" --data-dir "$artifact/n9" \
  >"$artifact/n9.log" 2>&1 &
n9_pid=$!; pids="$pids $n9_pid"
wait_until 'admitted node rejected by the root-identity gate' wrong_root_was_rejected

echo "PASS: explicit root, durable incarnation, fenced discovery, credentialed learner, and promotion"
