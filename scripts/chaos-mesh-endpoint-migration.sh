#!/usr/bin/env bash
# Sourced after the original voter fault matrix. The admitted learner has a
# Pod-IP endpoint, so its owner death exercises a real retained-PVC move while
# both independent clients continue through the surviving metadata voters.

migration_capture() {
  local stage="$1" folder="$migration_scene/$1" file pid
  mkdir -p "$folder"
  date --iso-8601=ns >"$folder/at.txt"
  k get pod -n "$namespace" "$migration_pod" -o json >"$folder/pod.json"
  k get pvc -n "$namespace" kv9-data-n4 -o json >"$folder/pvc.json"
  for file in kv9-root-descriptor kv9-store-identity kv9-store-lifecycle kv9-serving-endpoint; do
    k exec -n "$namespace" "$migration_pod" -- cat "/data/$file" >"$folder/$file"
  done
  k exec -n "$namespace" "$migration_pod" -- cat /data/status >"$folder/status.txt"
  if [[ "$stage" != held ]]; then
    pid="$(awk -F= '$1 == "pid" {print $2}' "$folder/status.txt")"
    [[ "$pid" =~ ^[1-9][0-9]*$ ]] || return 1
    k exec -n "$namespace" "$migration_pod" -- cat "/proc/$pid/cmdline" >"$folder/cmdline"
    k exec -n "$namespace" "$migration_pod" -- cat "/proc/$pid/stat" >"$folder/proc-stat.txt"
  fi
}

migration_replaced_held() {
  local pod uid
  pod="$(pod_for 4)" || return 1
  uid="$(pod_uid 4)" || return 1
  [ -n "$uid" ] && [ "$uid" != "$migration_old_uid" ] || return 1
  k exec -n "$namespace" "$pod" -- /bin/bash -c \
    'test ! -e /tmp/join-ticket && ! timeout 1 /bin/bash -c "exec 3<>/dev/tcp/127.0.0.1/20160"' \
    >/dev/null 2>&1
}

migration_owner_waiting() {
  local pid pod
  pod="$(pod_for 4)" || return 1
  [ "$(status_value 4 endpoint_ready)" = false ] &&
    [ "$(status_value 4 bootstrap_state)" = Joining ] &&
    [ "$(status_value 4 raft_receive_authorized)" = true ] &&
    [ "$(status_value 4 raft_owner_started)" = true ] || return 1
  pid="$(status_value 4 pid)" || return 1
  k exec -n "$namespace" "$pod" -- /bin/bash -c \
    'tr "\0" " " < "/proc/$1/cmdline" | grep -F -- "--advertise-addr $2"' \
    command "$pid" "$migration_address" >/dev/null 2>&1
}

migration_owner_confirmed() {
  local index applied
  [ "$(status_value 4 bootstrap_state)" = Serving ] &&
    [ "$(status_value 4 endpoint_ready)" = true ] || return 1
  index="$(status_value 4 endpoint_confirmation_index)" || return 1
  applied="$(status_value 4 driver_applied_index)" || return 1
  [[ "$index" =~ ^[1-9][0-9]*$ && "$applied" =~ ^[1-9][0-9]*$ ]] &&
    (( index > migration_mutation_index && applied >= index ))
}

migration_probe_endpoints() {
  local label="$1" rc=0 ip="${migration_old_address%:*}"
  k exec -n "$namespace" kv9-history-client -- timeout 2 /bin/bash -c \
    'exec 3<>/dev/tcp/'"$ip"'/20160' \
    >"$migration_scene/$label-old-probe.out" 2>"$migration_scene/$label-old-probe.err" || rc=$?
  [[ "$rc" == 1 || "$rc" == 124 ]] || {
    echo 'FAIL: obsolete endpoint is still reachable or probe failed outside its deadline/refusal contract' >&2
    return 1
  }
  printf '%s\n' "$rc" >"$migration_scene/$label-old-probe.exit"
  k exec -n "$namespace" kv9-history-client -- timeout 2 /bin/bash -c \
    'exec 3<>/dev/tcp/'"${migration_address%:*}"'/20160' \
    >"$migration_scene/$label-new-probe.out" 2>"$migration_scene/$label-new-probe.err"
  echo 0 >"$migration_scene/$label-new-probe.exit"
  date --iso-8601=ns >"$migration_scene/$label-probed-at.txt"
}

run_endpoint_migration() {
  local migration_scene="$artifact/endpoint-migration" migration_pod migration_old_uid
  local migration_address migration_old_address migration_mutation_index leader pod_ip receipt
  mkdir -p "$migration_scene"
  migration_pod="$(pod_for 4)"
  migration_old_uid="$(pod_uid 4)"
  [ "$(status_value 4 bootstrap_state)" = Serving ] || {
    echo 'FAIL: migration requires the original registered learner' >&2; return 1;
  }
  migration_capture before
  leader="$(wait_agreed_leader 'leader before learner endpoint migration' 30)"
  client "$leader" get-node-endpoint --addr "$(service_ip "$leader"):20160" --node-id 4 \
    >"$migration_scene/endpoint-before.out"
  migration_old_address="$(awk -F= '$1 == "address" {print $2}' "$migration_scene/endpoint-before.out")"
  cat >"$migration_scene/kill.request.yaml" <<YAML
apiVersion: chaos-mesh.org/v1alpha1
kind: PodChaos
metadata:
  name: endpoint-migration-kill
  namespace: $namespace
spec:
  action: pod-kill
  mode: all
  selector:
    namespaces: ["$namespace"]
    pods:
      "$namespace": ["$migration_pod"]
YAML
  k apply -f "$migration_scene/kill.request.yaml" >/dev/null
  wait_injected podchaos endpoint-migration-kill
  wait_until 'PodChaos kills the admitted owner and its successor waits with the original PVC' 60 migration_replaced_held
  migration_pod="$(pod_for 4)"
  k exec -n "$namespace" "$migration_pod" -- /usr/local/bin/kv9 store-prepare \
    --node-id 4 --data-dir /data >"$migration_scene/owner-release.prepare"
  migration_capture held
  k get podchaos -n "$namespace" endpoint-migration-kill -o json >"$migration_scene/kill.injected.json"
  # Use a distinct Service address for the migrated canonical route. It is a
  # distributed Kubernetes routing facility, not a database coordinator.
  k apply -f - >/dev/null <<YAML
apiVersion: v1
kind: Service
metadata:
  name: kv9-migrated-n4
  namespace: $namespace
spec:
  selector:
    app: kv9
    kv9-node: "4"
  ports:
    - name: grpc
      port: 20160
      targetPort: 20160
YAML
  migration_address="$(k get service -n "$namespace" kv9-migrated-n4 -o jsonpath='{.spec.clusterIP}'):20160"
  [ "$migration_address" != "$migration_old_address" ] || return 1
  k get service -n "$namespace" kv9-migrated-n4 -o json >"$migration_scene/migrated-service.json"
  python3 - "$migration_address" >"$migration_scene/configure.patch.json" <<'PY'
import ipaddress, json, sys
address=sys.argv[1]
ipaddress.ip_address(address.split(':')[0])
script = 'exec /usr/local/bin/kv9 start --node-id 4 --addr 0.0.0.0:20160 --advertise-addr ' + address + ' --data-dir /data'
print(json.dumps({'spec': {'template': {'spec': {'containers': [{'name': 'kv9', 'args': [script]}]}}}}))
PY
  k patch deployment kv9-n4 -n "$namespace" --type=strategic --patch-file "$migration_scene/configure.patch.json" >/dev/null
  k rollout status deployment/kv9-n4 -n "$namespace" --timeout=60s >/dev/null
  wait_until 'retained learner opens Raft but refuses Serving at its unauthorized endpoint' 45 migration_owner_waiting
  migration_pod="$(pod_for 4)"
  pod_ip="$(k get pod -n "$namespace" "$migration_pod" -o jsonpath='{.status.podIP}')"
  [ "$pod_ip:20160" != "$migration_old_address" ] || {
    echo 'FAIL: replacement Pod reused the obsolete canonical address' >&2; return 1;
  }
  migration_capture unconfirmed
  migration_probe_endpoints pending-before-history
  cp "$migration_scene/before/pod.json" "$artifact/endpoint-migration-pending-persistent-victim.json"
  history_phase endpoint-migration-pending
  migration_probe_endpoints pending-after-history
  migration_owner_waiting
  migration_capture pending
  leader="$(wait_agreed_leader 'surviving quorum before migration authorization' 30)"
  local cluster incarnation generation
  cluster="$(awk -F= '$1 == "cluster_id" {print $2}' "$migration_scene/endpoint-before.out")"
  incarnation="$(awk -F= '$1 == "store_incarnation" {print $2}' "$migration_scene/endpoint-before.out")"
  generation="$(awk -F= '$1 == "generation" {print $2}' "$migration_scene/endpoint-before.out")"
  client "$leader" change-node-endpoint --addr "$(service_ip "$leader"):20160" --cluster-id "$cluster" \
    --node-id 4 --store-incarnation "$incarnation" --expected-address "$migration_old_address" \
    --expected-generation "$generation" --new-address "$migration_address" >"$migration_scene/mutation.out"
  migration_mutation_index="$(awk -F= '$1 == "mutation_index" {print $2}' "$migration_scene/mutation.out")"
  [[ "$migration_mutation_index" =~ ^[1-9][0-9]*$ ]] || return 1
  client "$leader" change-node-endpoint --addr "$(service_ip "$leader"):20160" --cluster-id "$cluster" \
    --node-id 4 --store-incarnation "$incarnation" --expected-address "$migration_old_address" \
    --expected-generation "$generation" --new-address "$migration_address" >"$migration_scene/duplicate.out"
  wait_until 'migrated learner applies its exact fresh confirmation' 45 migration_owner_confirmed
  migration_capture confirmed
  migration_probe_endpoints recovered-before-history
  cp "$migration_scene/before/pod.json" "$artifact/endpoint-migration-recovered-persistent-victim.json"
  history_phase endpoint-migration-recovered
  migration_probe_endpoints recovered-after-history
  # A read at the new public socket must reach the actual learner and return
  # its typed NotLeader verdict; it must not acquire leader read authority.
  local rc=0
  client 4 raw-get --addr "$migration_address" --keyspace "$keyspace" --key-hex 62617365 \
    >"$migration_scene/new-endpoint-get.out" 2>"$migration_scene/new-endpoint-get.err" || rc=$?
  echo "$rc" >"$migration_scene/new-endpoint-get.exit"
  PYTHONPATH=scripts python3 - "$migration_scene" <<'PY'
from pathlib import Path
import subprocess,sys
from chaos_client import refused
root=Path(sys.argv[1])
assert refused(subprocess.CompletedProcess([], int((root/'new-endpoint-get.exit').read_text()),
    (root/'new-endpoint-get.out').read_text(), (root/'new-endpoint-get.err').read_text())), 'new endpoint did not return exclusive learner refusal'
PY
  k get podchaos -n "$namespace" endpoint-migration-kill -o json >"$migration_scene/kill.after-history.json"
  history_set_phase healing
  k delete podchaos endpoint-migration-kill -n "$namespace" --wait=true >/dev/null
  echo 'PASS: actual PodChaos retained-PVC endpoint migration with exact confirmation and both complete histories'
}
