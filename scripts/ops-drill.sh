#!/usr/bin/env bash
# Operations drill against a local kv9 cluster on MinIO: every verb in docs/OPERATIONS.md,
# in the order an operator meets them, plus two self-repair scenarios. Logs everything.
set -uo pipefail
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
D=${DRILL_DIR:-/tmp/kv9-drill-$(date +%H%M%S)}   # NVMe: the RAID at /mnt/data has 60-100 ms fsync tails that churn the 200 ms Raft election timeout
KEEP=${DRILL_KEEP:-$D}   # where the log and status files are copied at the end
mkdir -p "$D"; exec > >(tee "$D/drill.log") 2>&1
base=${DRILL_BASE_PORT:-27500}; minio_port=27499
KV9=$repo/target/debug/kv9; ADMIN=$repo/target/debug/kv9-admin; REPL=$repo/target/debug/kv9-repl
export KV9_BOOTSTRAP_TOKEN=drill-bootstrap KV9_CLUSTER_TOKEN=drill-cluster
export KV9_CLIENT_TOKENS="admin=drill-admin-token,app=drill-app-token"
export KV9_ADMIN_TOKEN=drill-admin-token KV9_CLIENT_TOKEN=drill-app-token
declare -A addr pid
# DRILL_SCOPED=1: nodes 7, 9, 10 and 11 join in the scoped metadata mode (docs/META-REPLICA-MODES.md);
# 4 and 5 stay full and are promoted, 6 stays a full learner (voter replacement's spare).
SCOPED=${DRILL_SCOPED:-0}
for n in 1 2 3 4 5 6 7 8 9 10 11; do addr[$n]=127.0.0.1:$((base + n)); done
peers=$(for n in 1 2 3 4 5 6 7 9 10 11; do printf '%s=%s,' $n ${addr[$n]}; done); peers=${peers%,}
EP="${addr[1]},${addr[2]},${addr[3]}"

ts() { date +%H:%M:%S; }
step() { printf '\n\n==================== [%s] %s   (metadata term %s)\n' "$(ts)" "$*" "$(timeout 10 "$ADMIN" --endpoints "$EP" --output json --timeout 5 cluster info 2>/dev/null | py 'd["result"]["term"]' 2>/dev/null || echo '?')"; }
note() { printf '\n# %s\n' "$*"; }
run() { printf '\n[%s] $ %s\n' "$(ts)" "$*"; "$@"; local rc=$?; (( rc )) && printf '[exit=%d]\n' $rc; return $rc; }
adm() { run "$ADMIN" --endpoints "$EP" --yes --timeout 30 "$@"; }
# retry a verb whose typed refusal is transient (a transferee one entry behind)
adm_retry() { local i; for i in 1 2 3 4 5; do adm "$@" && return 0; sleep 1; note "retry $i"; done; return 1; }
# wait until a verb succeeds (used right after a leader restart, while the directory still points at it)
adm_until_ok() { local i; for i in $(seq 1 30); do if admj "$@" >/dev/null 2>&1; then adm "$@"; return 0; fi; sleep 1; done; adm "$@"; }
diag() { note "diagnostics: $1"; admj group describe "$2" | py 'json.dumps({"plans": d["result"]["plans"], "splits": d["result"]["splits"], "merges": d["result"]["merges"]})'; for k in 103 104 106 108 117; do printf 'kind %s: ' $k; admj task list --kind $k --limit 100 | py '[t["operation"][:8] for t in d["result"]["tasks"]]'; done; }
admj() { "$ADMIN" --endpoints "$EP" --yes --timeout 30 --output json "$@" 2>/dev/null; }
py() { python3 -c "import json,sys; d=json.load(sys.stdin); print($1)"; }
repl() { local lines="$1"; printf '\n[%s] $ kv9-repl <<< %s\n' "$(ts)" "$(echo "$lines" | tr '\n' ';')"; printf '%s\n' "$lines" | "$REPL" --addr "${addr[1]}" --peers "$peers" --timeout-ms 10000; }
wait_for() { local label="$1" secs="$2"; shift 2; local end=$((SECONDS + secs)) t0=$SECONDS
  while (( SECONDS < end )); do if "$@" >/dev/null 2>&1; then printf '[%s] ok: %s (%ds)\n' "$(ts)" "$label" $((SECONDS - t0)); return 0; fi; sleep 1; done
  printf '[%s] TIMEOUT after %ds: %s\n' "$(ts)" $secs "$label"; return 1; }
sv() { awk -F= -v k="$2" '$1==k{print substr($0,length(k)+2)}' "$D/n$1/status" 2>/dev/null; }
node_env=(KV9_STORAGE=minio KV9_ADMIN_PRINCIPALS=admin KV9_RECLAIM_RETIRED=1
  KV9_AUTO_REPAIR_REPLICA=1 KV9_REPLICA_LOST_SECS=20 KV9_REPLICA_REPAIR_COOLDOWN_SECS=5
  KV9_AUTO_REPLACE_VOTER=1 KV9_VOTER_LOST_SECS=20 KV9_VOTER_REPLACE_COOLDOWN_SECS=5)
start_node() { local n=$1; shift
  printf '[%s] $ kv9 start --node-id %s --addr %s --data-dir n%s  (env: %s)\n' "$(ts)" $n ${addr[$n]} $n "${node_env[*]} $*"
  env "${node_env[@]}" "$@" "$KV9" start --node-id $n --addr ${addr[$n]} --data-dir "$D/n$n" >>"$D/n$n.log" 2>&1 & pid[$n]=$!; }
serving() { [[ $(sv $1 pid) == "${pid[$1]}" && $(sv $1 bootstrap_state) == Serving && $(sv $1 endpoint_ready) == true ]]; }
cleanup() { step "teardown"; for n in "${!pid[@]}"; do kill ${pid[$n]} 2>/dev/null; done; docker rm -f kv9-drill-minio >/dev/null 2>&1; sleep 1; cp "$D/drill.log" "$KEEP/drill-$(basename $D).log" 2>/dev/null; for n in 1 2 3 4 5 6 7 9 10 11; do cp "$D/n$n/status" "$KEEP/status-$(basename $D)-n$n" 2>/dev/null; done; echo "artifacts: $D (log copied to $KEEP)"; }
trap cleanup EXIT

# ---------------------------------------------------------------- group helpers (JSON)
gview() { admj group describe "$1"; }
gleader() { gview "$1" | py '[v["observed"] for v in d["result"]["replica_views"] if (v.get("observed") or {}).get("role")=="Leader" and v["observed"].get("state")=="active"][0]'; }
gvoters() { gview "$1" | py 'sorted([v["observed"] for v in d["result"]["replica_views"] if (v.get("observed") or {}).get("role")=="Leader" and v["observed"].get("state")=="active"][0]["voters"])'; }
voters_are() { [[ $(gvoters "$1") == "$2" ]]; }
stage_of() { gview "$1" | py "[p['stage'] for p in d['result'].get('$2',[]) if p['task']==$3][0]"; }
watch_stages() { # watch_stages <group-task> <family> <task> <final-regex> <secs>
  local last="" s end=$((SECONDS + $5)); while (( SECONDS < end )); do s=$(stage_of "$1" "$2" "$3" 2>/dev/null)
    if [[ -n $s && $s != "$last" ]]; then printf '[%s]   %s task %s: stage=%s\n' "$(ts)" "$2" "$3" "$s"; last=$s; fi
    [[ $s =~ $4 ]] && return 0; sleep 1; done; echo "TIMEOUT watching $2 $3 (last=$last)"; return 1; }
join_node() { local n=$1 meta=${2:-auto}; local t
  t=$(admj node admit $n --addr ${addr[$n]} --ttl 600 --meta $meta | py 'd["result"]["join_ticket"]')
  printf '[%s] $ kv9-admin node admit %s --addr %s --ttl 600   -> join_ticket=%s...\n' "$(ts)" $n ${addr[$n]} "${t:0:12}"
  printf '[%s] $ KV9_JOIN_TICKET=... kv9 join --root root.bin --node-id %s --data-dir n%s\n' "$(ts)" $n $n
  KV9_JOIN_TICKET=$t "$KV9" join --root "$D/root.bin" --node-id $n --data-dir "$D/n$n" || return 1
  start_node $n KV9_JOIN_TICKET=$t; wait_for "node $n Serving" 60 serving $n; }

# ================================================================ 0. object storage + build
step "0. MinIO (object storage is required by replica plans and split/merge)"
export KV9_OBJECT_STORE_ENDPOINT=http://127.0.0.1:$minio_port KV9_OBJECT_STORE_BUCKET=kv9-drill
export KV9_OBJECT_STORE_ACCESS_KEY=kv9drill KV9_OBJECT_STORE_SECRET_KEY=kv9drill-secret-key
docker rm -f kv9-drill-minio >/dev/null 2>&1
run docker run -d --name kv9-drill-minio -p 127.0.0.1:$minio_port:9000 -e MINIO_ROOT_USER=$KV9_OBJECT_STORE_ACCESS_KEY -e MINIO_ROOT_PASSWORD=$KV9_OBJECT_STORE_SECRET_KEY quay.io/minio/minio:latest server /data
for _ in $(seq 60); do curl -sf $KV9_OBJECT_STORE_ENDPOINT/minio/health/live >/dev/null && break; sleep 0.5; done
run docker exec -e MC_HOST_d=http://$KV9_OBJECT_STORE_ACCESS_KEY:$KV9_OBJECT_STORE_SECRET_KEY@127.0.0.1:9000 kv9-drill-minio mc mb --ignore-existing d/$KV9_OBJECT_STORE_BUCKET
(cd $repo && cargo build -q -p kv9 -p kv9-admin -p kv9-repl --bin kv9 --bin kv9-admin --bin kv9-repl) || exit 1
run "$KV9" --version; run "$ADMIN" version 2>&1 | head -3; run git -C $repo rev-parse --short HEAD

# ================================================================ 1. create the cluster
step "1. Create a three-voter cluster (store-prepare -> root-create -> init -> start)"
inc=""
for n in 1 2 3; do out=$(run "$KV9" store-prepare --node-id $n --data-dir "$D/n$n"); echo "$out"; inc+="$n=$(echo "$out" | grep -o 'store_incarnation=[^ ]*' | cut -d= -f2),"; done
run "$KV9" root-create --output "$D/root.bin" --voters "1@${addr[1]},2@${addr[2]},3@${addr[3]}" --store-incarnations "${inc%,}"
for n in 1 2 3; do run "$KV9" init --root "$D/root.bin" --node-id $n --data-dir "$D/n$n"; done
for n in 1 2 3; do start_node $n; done
for n in 1 2 3; do wait_for "node $n Serving" 60 serving $n; done

step "2. First look: cluster info / node list / cluster health / status document"
adm cluster info; adm node list; adm cluster health
note "the status document of node 1 (the fields an operator reads first)"
grep -E '^(node_id|role|leader_id|meta_voters|meta_learners|applied_index|fatal|replica_plans|replica_repair|voter_replacement|node_drain|range_split|data_groups)=' "$D/n1/status"

# ================================================================ 3. grow the cluster
step "3. Add nodes 4-7 (admit -> join -> start), promote 4 and 5 to metadata voters"
if [[ $SCOPED == 1 ]]; then
  for n in 4 5 6; do join_node $n full || exit 1; done
  for n in 7 9 10 11; do join_node $n scoped || exit 1; done
  note "scoped nodes: no metadata Raft owner, a streamed catalog slice"
  for n in 7 9 10 11; do grep -E '^(meta_mode|catalog_scope|catalog_scope_server|raft_owner_started)=' "$D/n$n/status" | tr '\n' ' ' | sed "s/^/n$n /"; echo; done
else
  for n in 4 5 6 7; do join_node $n || exit 1; done
fi
adm node list
adm node promote 4 --wait; adm node promote 5 --wait
note "repeating a membership verb whose state already holds answers 'confirmed'"
adm node promote 4 --wait
note "an unused admission is cancelled with revoke"
adm node admit 8 --addr ${addr[8]} --ttl 60 | grep -v join_ticket; adm node revoke 8
adm cluster info

# ================================================================ 4. data group + keyspace + data
step "4. A data group on stores 1,2,3 and a keyspace bound to it; write data through the REPL"
created=$(admj group create --voters 1,2,3 --label orders); echo "$created" | py 'json.dumps(d["result"])'
task=$(echo "$created" | py 'd["result"]["creation_task"]'); region=$(echo "$created" | py 'd["result"]["region_id"]')
echo "creation_task=$task region=$region"
adm keyspace create --name orders --group "$task"
wait_for "data group active behind a leader" 120 gleader "$task"
wait_for "keyspace routable" 120 bash -c "printf 'use orders\nput probe 1\n' | $REPL --addr ${addr[1]} --peers $peers --timeout-ms 5000 | grep -q applied"
fill=$(printf 'use orders\n'; for i in $(seq 0 399); do printf 'put r/%05d value-%d\n' $i $i; done)
printf '[%s] $ kv9-repl: use orders; put r/00000..r/00399\n' "$(ts)"; printf '%s\n' "$fill" | "$REPL" --addr "${addr[1]}" --peers "$peers" --timeout-ms 10000 | sort | uniq -c | sort -rn | head -3
repl $'use orders\nget r/00007\nscan r/0010 r/0011 3\nregion r/00300\ninfo'
adm keyspace list; adm group list; adm group describe "$task" | head -40

# ================================================================ 5. leadership moves
step "5. Planned maintenance: move metadata leadership, then the data group's leadership"
leader=$(admj cluster info | py 'd["resolved_leader"]["node_id"]'); to=$(( leader == 4 ? 5 : 4 ))
echo "metadata leader is n$leader; transferring to n$to"
adm_retry node transfer-leader --to $to --wait; adm cluster info | grep -i leader
note "transfer to the current leader confirms instead of failing"
adm node transfer-leader --to $to --wait
glid=$(gview "$task" | py '[v["node_id"] for v in d["result"]["replica_views"] if (v.get("observed") or {}).get("role")=="Leader"][0]')
gto=$(( glid == 1 ? 2 : 1 )); echo "group leader is n$glid; transferring to n$gto"
adm_retry group transfer-leader "$task" --to $gto --wait

# ================================================================ 6. add + move replica
step "6. Grow the data group: add a replica on node 4 (one committed plan, stages watched)"
plan=$(admj group add-replica "$task" --to 4 --label add-4); echo "$plan" | py 'json.dumps(d["result"])'
ptask=$(echo "$plan" | py 'd["result"]["plan_task"]')
note "an Add plan's last catalog stage is 'evidenced' (REPLICA-ADDITION.md §3); completion = the destination votes"
watch_stages "$task" plans "$ptask" 'evidenced|aborted' 300; wait_for "voters 1,2,3,4" 120 voters_are "$task" "[1, 2, 3, 4]"
repl $'use orders\nget r/00007\nput during-add ok\nget during-add'
step "6b. Move the replica on node 1 to node 5 (the group never shrinks first)"
plan=$(admj group move-replica "$task" --from 1 --to 5 --label move-1-5); ptask=$(echo "$plan" | py 'd["result"]["plan_task"]')
watch_stages "$task" plans "$ptask" 'decided|aborted' 300; wait_for "voters 2,3,4,5" 180 voters_are "$task" "[2, 3, 4, 5]"
adm task list --kind 111; adm group describe "$task" | sed -n '1,30p'

# ================================================================ 7. refusals an operator will meet
step "7. Refusals an operator will meet: non-admin token; removing the metadata leader itself"
leader=$(admj cluster info | py 'd["resolved_leader"]["node_id"]')
run env KV9_ADMIN_TOKEN=drill-app-token "$ADMIN" --endpoints "$EP" --yes node list
adm node remove $leader

# ================================================================ 8. abort a plan
step "8. Abort: start a replica addition to node 6 and abort it while it runs"
plan=$(admj group add-replica "$task" --to 6 --label add-6-abort); ptask=$(echo "$plan" | py 'd["result"]["plan_task"]')
echo "plan task $ptask; aborting"; adm task abort "$ptask"
watch_stages "$task" plans "$ptask" 'decided|aborted' 300; echo "voters now: $(gvoters "$task")"
adm task describe "$ptask" | head -30

# ================================================================ 9. split + exclusivity + merge
step "9. Split the range at r/00200; a membership plan is refused while the split is live"
split=""; for i in 1 2 3 4 5; do split=$(admj group split "$task" --at r/00200 --label split-200); echo "$split" | py 'json.dumps(d.get("result") or d["error"])'
  if echo "$split" | py 'd["result"]["intent_task"]' >/dev/null 2>&1; then break; fi; diag "split refused (attempt $i)" "$task"; sleep 2; done
stask=$(echo "$split" | py 'd["result"]["intent_task"]') || exit 1
note "a membership plan is refused while the split is live"
adm group add-replica "$task" --to 6 --label refused-during-split
watch_stages "$task" splits "$stask" 'published|aborted' 600
adm group list; adm task list --kind 113; adm task list --kind 114
note "reads and scans cross the new boundary; the REPL follows the resume cursor"
repl $'use orders\nget r/00007\nget r/00399\nregion r/00100\nregion r/00300\nscan r/00198 r/00203 10'
live_groups() { admj group list | python3 -c '
import json,sys; gs=json.load(sys.stdin)["result"]
live=[(min(k["start"] for k in g["keyspaces"]), g["creation_task"]) for g in gs if any(not k["sealed"] for k in g["keyspaces"])]
print(" ".join(str(t) for _, t in sorted(live)))'; }
echo "live groups carrying the keyspace (low to high): $(live_groups)"
step "9b. Merge the two children back into one range"
set -- $(live_groups); lo=$1; hi=$2
merge=$(admj group merge $lo $hi --label merge-back); echo "$merge" | py 'json.dumps(d["result"])'
mtask=$(echo "$merge" | py 'd["result"]["intent_task"]')
watch_stages "$lo" merges "$mtask" 'published|aborted' 600 || watch_stages "$hi" merges "$mtask" 'published|aborted' 60
adm group list; repl $'use orders\nget r/00007\nget r/00399\nscan r/00198 r/00203 10'
echo "live group(s) carrying the keyspace now: $(live_groups)"
set -- $(live_groups); task=$1

# ================================================================ 10. drain a node
step "10. Retire node 2: node drain --wait (replicas move off, metadata membership released)"
adm node drain 2 --wait --wait-secs 600
adm node list; echo "voters now: $(gvoters "$task")"; adm task list --kind 112
grep -E '^node_drain=' "$D/n2/status" | cut -c1-400
note "the drained node is stopped"; kill ${pid[2]}; unset 'pid[2]'; EP="${addr[1]},${addr[4]},${addr[5]}"
adm cluster health

# ================================================================ 11. unplanned loss, self-repair
step "11. Unplanned loss: kill -9 node 3 (metadata voter + data-group voter); the cluster repairs itself"
echo "voters before: $(gvoters "$task")"; kill -9 ${pid[3]}; unset 'pid[3]'; echo "[$(ts)] n3 killed"
sleep 5; adm cluster health
wait_for "data group voters no longer include 3" 300 bash -c "[[ \$($ADMIN --endpoints $EP --yes --output json group describe $task | python3 -c 'import json,sys; d=json.load(sys.stdin); print(3 not in [v[\"observed\"] for v in d[\"result\"][\"replica_views\"] if (v.get(\"observed\") or {}).get(\"role\")==\"Leader\" and v[\"observed\"].get(\"state\")==\"active\"][0][\"voters\"])') == True ]]"
echo "voters after repair: $(gvoters "$task")"
wait_for "metadata voters no longer include 3" 300 bash -c "! $ADMIN --endpoints $EP --yes --output json cluster info | python3 -c 'import json,sys; d=json.load(sys.stdin); sys.exit(0 if 3 in d[\"result\"][\"meta_voters\"] else 1)'"
adm cluster info; adm node list; adm task list --kind 111 | tail -5
for n in 1 4 5; do grep -E '^(replica_repair|voter_replacement)=' "$D/n$n/status" | cut -c1-300 | sed "s/^/n$n /"; done
adm cluster health

# ================================================================ 12. restart a node
step "12. Restart node 4 with the same start command; it rejoins with its data directory"
note "node 4 is the metadata leader at this point, so this is also a leader restart"
kill ${pid[4]}; wait ${pid[4]} 2>/dev/null; echo "[$(ts)] n4 stopped"; start_node 4; wait_for "node 4 Serving again" 90 serving 4
adm_until_ok cluster info | grep -E 'leader_node_id|"term"'; adm node describe 4 | head -25; adm cluster health

# ================================================================ 13. final state
step "13. Final state and the committed history"
adm group list; adm keyspace describe orders; adm node list
repl $'use orders\nget r/00000\nget r/00199\nget r/00200\nget r/00399\nget during-add\nscan r/ s 1000' | tail -3
for k in 100 102 103 106 108 111 112 113 114 115 116 117 118; do c=$(admj task list --kind $k --limit 200 | py 'len(d["result"]["tasks"])' 2>/dev/null); printf 'kind %s: %s rows\n' $k "${c:-?}"; done
echo "DRILL_DONE"
