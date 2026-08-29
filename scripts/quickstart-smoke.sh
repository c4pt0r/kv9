#!/usr/bin/env bash
# The README/--help quickstart, as something that runs.
#
# It exists because the prose version was wrong twice in one day: `--help` listed no raw
# commands at all while five worked, and the walkthrough said `kv9 client ...` when a clean
# clone only produces `./target/debug/kv9`, and read the leader from n1's status file when
# n1 may be a follower. Both mistakes are invisible to a reader and fatal to a copier.
#
# So the quickstart is executable: if the documented path stops working, this fails, instead
# of a newcomer discovering it.
#
# Deliberately uses the SAME form a reader is told to use -- ./target/debug/kv9, no PATH
# assumption -- because the point is to test the instructions, not the binary.
set -euo pipefail

bin=./target/debug/kv9
[ -x "$bin" ] || { echo "FAIL: $bin missing; run 'cargo build --workspace' first" >&2; exit 1; }

base="${KV9_BASE_PORT:-27700}"
[ "$base" -lt 32768 ] || { echo "FAIL: base port must stay below the ephemeral range" >&2; exit 1; }
dir="$(mktemp -d /tmp/kv9-quickstart.XXXXXX)"
pids=""
# Kill only our own pids -- never a global pkill -- and keep the data dir unless the
# run actually succeeded.
#
# An earlier version deleted the dir unconditionally here, so every CI failure erased
# its own scene before the workflow's if:failure() collection could run. The collector
# then truthfully reported "no directory to collect", which reads as a collector bug
# and sends the investigation the wrong way. The other three E2E scripts never delete
# their artifact dir; this one had simply not inherited that.
#
# Keyed off the real exit status rather than a "we reached the end" flag: such a flag
# stops protecting the scene the moment anyone appends code after it, and it fails
# OPEN -- deleting -- in exactly the case where the evidence matters.
cleanup() {
  local rc=$?
  for p in $pids; do kill "$p" 2>/dev/null || true; done
  if (( rc == 0 )); then rm -rf "$dir"; else echo "FAIL: preserving quickstart evidence at $dir" >&2; fi
}
trap cleanup EXIT

status_value() {
  # Two `local` statements, not one. Bash expands the whole `local` command line
  # before assigning any of it, so `local node="$1" file="...${node}..."` expands
  # ${node} while it is still unset and silently yields ".../n/status" -- a path that
  # never exists, so every lookup returns empty and every gate reads as "not ready".
  # The other three scripts already split this; folding it into one line reintroduced
  # the bug they had avoided.
  local node="$1" key="$2"
  local file="$dir/n${node}/status"
  test -f "$file" || return 1
  awk -F= -v wanted="$key" '$1 == wanted { print substr($0, length($1) + 2); found=1 } END { if (!found) exit 1 }' "$file"
}

export KV9_CLUSTER_TOKEN=quickstart-cluster KV9_CLIENT_TOKENS=admin=quickstart-client
join=""
for i in 1 2 3; do join="${join}${join:+,}$i@127.0.0.1:$((base+i))"; done
for i in 1 2 3; do
  mkdir -p "$dir/n$i"
  "$bin" --node-id "$i" --addr "127.0.0.1:$((base+i))" --data-dir "$dir/n$i" --join "$join" \
    >"$dir/n$i.log" 2>&1 &
  pids="$pids $!"
done

# Wait for bootstrap_state=Serving, NOT merely for someone to claim role=leader.
#
# This is a SEMANTIC barrier, not stylistic agreement with the other three scripts.
# `Serving` implies the seed catalog is applied locally, and `tenants(0)` is written
# in the same seed CatalogTxn as the cluster_id: the winner only announces
# MetadataInitialized after wait_applied succeeds, and a non-winner requires a local
# cluster_id first. So Serving => tenants(0) exists.
# Winning an election does NOT imply any of that. Between "elected" and "catalog
# written" there is a window in which create-keyspace resolves tenant_id=0 against a
# tenants table that has no row 0 yet, and the request fails with
#   FK violation: keyspaces.col3 = 0 has no row in tenants
# Do not "simplify" this back to waiting for a leader: leadership and catalog
# readiness are different facts, and only the second one is the precondition here.
#
# NOTE (task #37): waiting here makes THIS script correct, but a human following the
# prose quickstart still has no such barrier and can still hit that FK error. This
# script was the only thing exercising that window, so this fix removes the alarm
# without fixing the cause. The cause is a product defect -- pre-Serving client ops
# must return a retryable MetaNotReady rather than leaking a plan-phase integrity
# error -- tracked separately and landing alongside this change, not after it.
all_serving() {
  local i leader_id=0 seen
  for i in 1 2 3; do
    [ "$(status_value "$i" bootstrap_state 2>/dev/null || true)" = "Serving" ] || return 1
    [ -z "$(status_value "$i" fatal 2>/dev/null || true)" ] || return 1
    seen="$(status_value "$i" leader_id 2>/dev/null || true)"
    [ -n "$seen" ] && [ "$seen" -gt 0 ] 2>/dev/null || return 1
    # if/then, not `[ ] && x`: under `set -e` a bare failing && list as the last
    # statement of a block can terminate the shell, and this one is false on every
    # iteration after the first.
    if [ "$leader_id" -eq 0 ]; then leader_id="$seen"; fi
    [ "$seen" -eq "$leader_id" ] || return 1
  done
  echo "$leader_id"
}

# Resolve the leader from the value all three nodes AGREE on, not from one node's
# self-reported role. A node that has been deposed but has not yet stepped down still
# says `role=leader` about itself; the followers' `leader_id` is the cross-checked
# fact. The old code took the first self-claim and cached it, which is how a stale or
# never-quite-true leader ends up as the target for every later request.
leader=""
for _ in $(seq 1 60); do
  if leader="$(all_serving)"; then break; fi
  leader=""
  sleep 0.5
done
[ -n "$leader" ] || { echo "FAIL: three nodes did not reach Serving behind one agreed leader within 30s" >&2; exit 1; }

export KV9_CLIENT_TOKEN=quickstart-client

# Source and freshness are two different defects and need two different fixes
# (@Cindy's distinction):
#   source     one node's self-reported role  ->  the leader_id all three agree on
#   freshness  resolved once, then cached     ->  follow a typed hint on every request
# Fixing only the source still caches a value that was merely better-founded when read;
# fixing only freshness still re-reads a single node's self-claim. The barrier above is
# the source half; this wrapper is the freshness half.
#
# Retrying a typed `not_leader=true` is safe precisely because that answer states the
# command was NOT applied. Timeouts and every other failure stay loud and unretried:
# their write outcome is unknown, and retrying an unknown outcome is how a script turns
# one write into two. (Same three-state reasoning the propose path uses for
# NotLeader / Unconfirmed / Failed -- known-not-applied is the only safely retryable one.)
client_reply=""
quickstart_client() {
  local output rc hinted attempt
  for attempt in 1 2 3 4 5; do
    if output="$("$bin" client "$@" --addr "127.0.0.1:$((base + leader))" 2>&1)"; then
      client_reply="$output"
      return 0
    else
      rc=$?
    fi
    if [[ "$output" =~ not_leader=true[[:space:]]+leader_node_id=([1-3]) ]]; then
      hinted="${BASH_REMATCH[1]}"
      leader="$hinted"
      continue
    fi
    printf '%s\n' "$output" >&2
    return "$rc"
  done
  echo "FAIL: leader changed during five consecutive quickstart requests" >&2
  return 1
}

quickstart_client create-keyspace --name quickstart --api-type raw
ks=$(awk -F= '$1=="keyspace_id"{print $2}' <<<"$client_reply")
[ -n "$ks" ] || { echo "FAIL: no keyspace id returned" >&2; exit 1; }

key=$(printf 'hello' | xxd -p); val=$(printf 'world' | xxd -p)
quickstart_client raw-put --keyspace "$ks" --key-hex "$key" --value-hex "$val"
quickstart_client raw-get --keyspace "$ks" --key-hex "$key"
got="$client_reply"
[ "$got" = "value_hex=$val" ] || { echo "FAIL: raw-get returned '$got', wanted value_hex=$val" >&2; exit 1; }
quickstart_client raw-delete --keyspace "$ks" --key-hex "$key"
quickstart_client raw-get --keyspace "$ks" --key-hex "$key"
gone="$client_reply"
[ "$gone" = "found=false" ] || { echo "FAIL: after delete got '$gone', wanted found=false" >&2; exit 1; }

echo "PASS: quickstart path works end to end (leader n$leader, keyspace $ks)"
