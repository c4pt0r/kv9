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
cleanup() { for p in $pids; do kill "$p" 2>/dev/null || true; done; rm -rf "$dir"; }
trap cleanup EXIT

export KV9_CLUSTER_TOKEN=quickstart-cluster KV9_CLIENT_TOKENS=admin=quickstart-client
join=""
for i in 1 2 3; do join="${join}${join:+,}$i@127.0.0.1:$((base+i))"; done
for i in 1 2 3; do
  mkdir -p "$dir/n$i"
  "$bin" --node-id "$i" --addr "127.0.0.1:$((base+i))" --data-dir "$dir/n$i" --join "$join" \
    >"$dir/n$i.log" 2>&1 &
  pids="$pids $!"
done

# Find the leader across ALL nodes: node 1 is not guaranteed to be it.
leader=""
for _ in $(seq 1 60); do
  for i in 1 2 3; do
    if grep -q '^role=leader' "$dir/n$i/status" 2>/dev/null; then leader="$i"; break; fi
  done
  [ -n "$leader" ] && break
  sleep 0.5
done
[ -n "$leader" ] || { echo "FAIL: no leader within 30s" >&2; exit 1; }

export KV9_CLIENT_TOKEN=quickstart-client
addr="127.0.0.1:$((base+leader))"
ks=$("$bin" client create-keyspace --addr "$addr" --name quickstart --api-type raw \
      | awk -F= '$1=="keyspace_id"{print $2}')
[ -n "$ks" ] || { echo "FAIL: no keyspace id returned" >&2; exit 1; }

key=$(printf 'hello' | xxd -p); val=$(printf 'world' | xxd -p)
"$bin" client raw-put --addr "$addr" --keyspace "$ks" --key-hex "$key" --value-hex "$val" >/dev/null
got=$("$bin" client raw-get --addr "$addr" --keyspace "$ks" --key-hex "$key")
[ "$got" = "value_hex=$val" ] || { echo "FAIL: raw-get returned '$got', wanted value_hex=$val" >&2; exit 1; }
"$bin" client raw-delete --addr "$addr" --keyspace "$ks" --key-hex "$key" >/dev/null
gone=$("$bin" client raw-get --addr "$addr" --keyspace "$ks" --key-hex "$key")
[ "$gone" = "found=false" ] || { echo "FAIL: after delete got '$gone', wanted found=false" >&2; exit 1; }

echo "PASS: quickstart path works end to end (leader n$leader, keyspace $ks)"
