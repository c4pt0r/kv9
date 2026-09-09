#!/usr/bin/env bash
# Actual PodChaos owner death followed by fixture-controlled loss of the log.
# The original file is retained outside the Raft directory for recovery checks.
# This is not a power-loss simulation or a claim that PodChaos deletes files.

loss_serving() {
  local pid
  pid="$(k exec -n "$namespace" "$loss_pod" -- cat /tmp/kv9-loss.pid 2>/dev/null)" || return 1
  [ "$(status_value "$victim" pid)" = "$pid" ] && node_serving "$victim"
}

loss_stopped() {
  k exec -n "$namespace" "$loss_pod" -- /bin/bash -c \
    'test ! -f /tmp/kv9-loss.pid && test -f /tmp/kv9-loss.exit' 2>/dev/null
}

loss_replaced() {
  local uid pod
  uid="$(pod_uid "$victim")" || return 1
  [ -n "$uid" ] && [ "$uid" != "$loss_old_uid" ] || return 1
  pod="$(pod_for "$victim")" || return 1
  k exec -n "$namespace" "$pod" -- /bin/bash -c \
    'test -f /data/kv9-loss.hold && test ! -f /tmp/kv9-loss.pid &&
     ! timeout 1 /bin/bash -c "exec 3<>/dev/tcp/127.0.0.1/20160"' >/dev/null 2>&1
}

loss_capture() {
  local phase="$1" folder="$loss_scene/$1"
  mkdir -p "$folder"
  date --iso-8601=ns >"$folder/at.txt"
  k get pod -n "$namespace" "$loss_pod" -o json >"$folder/pod.json"
  k get pvc -n "$namespace" "kv9-data-n$victim" -o json >"$folder/pvc.json"
  k exec -n "$namespace" "$loss_pod" -- cat /data/kv9-store-lifecycle >"$folder/lifecycle"
  k exec -n "$namespace" "$loss_pod" -- cat /data/status >"$folder/status.txt"
  if [[ "$phase" == before || "$phase" == healed ]]; then
    loss_serving
    k exec -n "$namespace" "$loss_pod" -- cat /tmp/kv9-loss.pid >"$folder/pid.txt"
    k exec -n "$namespace" "$loss_pod" -- timeout 2 /bin/bash -c \
      'exec 3<>/dev/tcp/127.0.0.1/20160'
    echo reachable >"$folder/local-probe.txt"
  else
    k exec -n "$namespace" "$loss_pod" -- /bin/bash -c \
      'test ! -f /tmp/kv9-loss.pid && ! timeout 1 /bin/bash -c "exec 3<>/dev/tcp/127.0.0.1/20160"' \
      >"$folder/local-probe.log" 2>&1
    echo stopped-without-listener >"$folder/local-probe.txt"
    if [[ "$phase" == rejected-* ]]; then
      k exec -n "$namespace" "$loss_pod" -- cat /tmp/kv9-loss.last-pid >"$folder/pid.txt"
      k exec -n "$namespace" "$loss_pod" -- cat /tmp/kv9-loss.exit >"$folder/exit.txt"
      k exec -n "$namespace" "$loss_pod" -- cat /tmp/kv9-loss.log >"$folder/process.log"
      k exec -n "$namespace" "$loss_pod" -- /bin/bash -c \
        'test ! -e /data/raft/raft.log && sha256sum /data/kv9-loss.saved-log' >"$folder/saved-log.sha256"
      echo absent >"$folder/raft-log.txt"
      [ "$(cat "$folder/exit.txt")" = 1 ] &&
        grep -Fq 'raft error: open /data/raft/raft.log:' "$folder/process.log" &&
        grep -Fq 'os error 2' "$folder/process.log" &&
        ! grep -Fq panicked "$folder/process.log"
    fi
  fi
}

loss_caught_up() {
  local applied
  applied="$(status_value "$victim" driver_applied_index)" || return 1
  [[ "$applied" =~ ^[0-9]+$ ]] && (( applied >= loss_required_index ))
}

run_store_log_loss_matrix() {
  local victim label loss_pod loss_old_uid loss_scene loss_required_index attempt key_hex
  for victim in 1 2 3; do
    label="store-loss-voter-$victim-log-missing"
    loss_scene="$artifact/$label"
    mkdir -p "$loss_scene"
    echo "Stage: activated Raft log loss after PodChaos kills voter $victim"
    # A shell supervisor keeps the replacement Pod inspectable after startup
    # refuses. Only this fixture observes control files; kv9 is unmodified.
    python3 - "$victim" >"$loss_scene/supervisor.json" <<'PY'
import json, sys
script = '''set -uo pipefail
while [ -f /data/kv9-loss.hold ]; do sleep 0.1; done
touch /tmp/kv9-loss.start
while true; do
  while [ ! -f /tmp/kv9-loss.start ]; do sleep 0.1; done
  rm -f /tmp/kv9-loss.start /tmp/kv9-loss.exit
  /usr/local/bin/kv9 start --node-id NODE --addr 0.0.0.0:20160 --data-dir /data > /tmp/kv9-loss.log 2>&1 &
  child=$!
  echo "$child" > /tmp/kv9-loss.pid
  echo "$child" > /tmp/kv9-loss.last-pid
  wait "$child"
  result=$?
  rm -f /tmp/kv9-loss.pid
  echo "$result" > /tmp/kv9-loss.exit
done
'''.replace('NODE', sys.argv[1])
print(json.dumps([
    dict(op='replace', path='/spec/template/spec/containers/0/command', value=['/bin/bash', '-c']),
    dict(op='replace', path='/spec/template/spec/containers/0/args', value=[script]),
]))
PY
    k patch deployment "kv9-n$victim" -n "$namespace" --type=json \
      --patch-file "$loss_scene/supervisor.json" >/dev/null
    k rollout status deployment/"kv9-n$victim" -n "$namespace" --timeout=45s >/dev/null
    loss_pod="$(pod_for "$victim")"
    wait_until "store-loss supervisor runs the original voter" 40 loss_serving
    wait_agreed_leader "agreement before log loss" 30 >/dev/null
    key_hex="6c6f73732d3$victim"
    chaos_probe "$label-before-put" 0 raw-put --keyspace "$keyspace" \
      --key-hex "$key_hex" --value-hex 6265666f7265 >"$artifact/$label-before-put.out"
    loss_capture before
    loss_old_uid="$(pod_uid "$victim")"
    k exec -n "$namespace" "$loss_pod" -- touch /data/kv9-loss.hold
    k apply -f - >/dev/null <<YAML
apiVersion: chaos-mesh.org/v1alpha1
kind: PodChaos
metadata:
  name: store-loss-kill
  namespace: $namespace
spec:
  action: pod-kill
  mode: one
  selector:
    namespaces: ["$namespace"]
    pods:
      $namespace: ["$loss_pod"]
YAML
    wait_until "PodChaos kills the original store owner" 60 loss_replaced
    wait_injected podchaos store-loss-kill
    k get podchaos store-loss-kill -n "$namespace" -o json >"$loss_scene/podchaos.json"
    loss_pod="$(pod_for "$victim")"
    loss_capture held
    # The old owner is gone and the replacement cannot open the data. Save
    # the exact stopped log; never rename a live file behind an open handle.
    k exec -n "$namespace" "$loss_pod" -- /bin/bash -c \
      'test -s /data/raft/raft.log && test ! -e /data/kv9-loss.saved-log &&
       sha256sum /data/raft/raft.log && mv /data/raft/raft.log /data/kv9-loss.saved-log' \
      >"$loss_scene/original-log.sha256"
    k exec -n "$namespace" "$loss_pod" -- cat /data/kv9-loss.saved-log >"$loss_scene/original-raft.log"
    # Release only the shell's initial hold. Each failed process is followed
    # by an explicit fresh start; a stale PVC status is not refusal evidence.
    k exec -n "$namespace" "$loss_pod" -- rm /data/kv9-loss.hold
    for attempt in 1 2; do
      if (( attempt == 2 )); then
        k exec -n "$namespace" "$loss_pod" -- /bin/bash -c \
          'rm -f /tmp/kv9-loss.exit; touch /tmp/kv9-loss.start'
      fi
      wait_until "missing Active log refuses fresh startup attempt $attempt" 20 loss_stopped
      loss_capture "rejected-$attempt"
    done
    wait_majority_leader "majority serves while original voter cannot recover" 30 "$victim" >/dev/null
    history_phase "$label"
    loss_capture rejected-during-history
    chaos_probe "$label-majority-put" "$victim" raw-put --keyspace "$keyspace" \
      --key-hex "$key_hex" --value-hex 6166746572 >"$artifact/$label-majority-put.out"
    loss_required_index="$(awk -F= '$1 == "applied_index" {print $2}' "$artifact/$label-majority-put.out")"
    [[ "$loss_required_index" =~ ^[1-9][0-9]*$ ]]
    chaos_probe "$label-majority-get" "$victim" raw-get --keyspace "$keyspace" \
      --key-hex "$key_hex" >"$artifact/$label-majority-get.out"
    grep -Fxq value_hex=6166746572 "$artifact/$label-majority-get.out"
    history_set_phase healing
    k delete podchaos store-loss-kill -n "$namespace" --wait=true >/dev/null
    k exec -n "$namespace" "$loss_pod" -- /bin/bash -c \
      'test ! -e /data/raft/raft.log && mv /data/kv9-loss.saved-log /data/raft/raft.log &&
       sha256sum /data/raft/raft.log' >"$loss_scene/restored-log.sha256"
    k exec -n "$namespace" "$loss_pod" -- /bin/bash -c \
      'rm -f /tmp/kv9-loss.exit; touch /tmp/kv9-loss.start'
    wait_until "original log recovers the original voter" 40 loss_serving
    wait_agreed_leader "agreement after original log restoration" 40 >/dev/null
    wait_until "restored original voter applies the majority receipt" 30 loss_caught_up
    loss_capture healed
    chaos_probe "$label-recovered-get" 0 raw-get --keyspace "$keyspace" \
      --key-hex "$key_hex" >"$artifact/$label-recovered-get.out"
    grep -Fxq value_hex=6166746572 "$artifact/$label-recovered-get.out"
    echo "PASS: PodChaos voter $victim refused two missing-log starts and recovered the original store with majority service"
  done
}
