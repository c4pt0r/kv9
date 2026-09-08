#!/usr/bin/env bash
# Sourced by chaos-mesh-e2e.sh, after its Pod/Network matrix. All resources
# inherit that runner's isolated Kind namespace and evidence directory.

io_process_stopped() {
  k exec -n "$namespace" "$io_pod" -- /bin/bash -c \
    'test ! -f /tmp/kv9-io.pid && test -f /tmp/kv9-io.exit' 2>/dev/null
}

io_process_serving() {
  local pid
  pid="$(k exec -n "$namespace" "$io_pod" -- cat /tmp/kv9-io.pid 2>/dev/null)" || return 1
  [ "$(status_value "$victim" pid)" = "$pid" ] && node_serving "$victim"
}

io_start_process() {
  k exec -n "$namespace" "$io_pod" -- /bin/bash -c \
    'rm -f /tmp/kv9-io.exit; touch /tmp/kv9-io.start'
  wait_until "fresh process serves on voter $victim" 40 io_process_serving
}

io_stop_process() {
  k exec -n "$namespace" "$io_pod" -- /bin/bash -c \
    'kill -TERM "$(cat /tmp/kv9-io.pid)"'
  wait_until "process stops before mounting IOChaos" 15 io_process_stopped
}

io_caught_up() {
  local applied
  applied="$(status_value "$victim" driver_applied_index)" || return 1
  [[ "$applied" =~ ^[0-9]+$ ]] && (( applied >= io_required_index ))
}

io_write_fault() {
  local action="$1" name="$2" errno="${3:-}"
  {
    cat <<YAML
apiVersion: chaos-mesh.org/v1alpha1
kind: IOChaos
metadata:
  name: $name
  namespace: $namespace
spec:
  action: $action
  mode: one
  selector:
    namespaces: ["$namespace"]
    pods:
      $namespace: ["$io_pod"]
  containerNames: ["kv9"]
  volumePath: /data
  path: /data/raft/raft.log
  methods: ["WRITE"]
  percent: 100
YAML
    if [ "$action" = latency ]; then
      echo '  delay: 1ms'
    else
      printf '  errno: %s\n' "$errno"
    fi
  } >"$artifact/$name.request.yaml"
  k apply -f "$artifact/$name.request.yaml" >/dev/null
  wait_injected iochaos "$name"
}

run_io_matrix() {
  local victim errno io_pod io_uid label rc fatal io_leader key_hex io_required_index
  for victim in 1 2 3; do
    # Keep the container/mount namespace alive across process stops. This
    # supervisor is an E2E fixture; the shipped binary has no control-file seam.
    # Starting kv9 AFTER the FUSE mount is installed ensures the real WAL
    # descriptor participates in the fault. This cell covers restart/catch-up
    # writes; live-process descriptor replacement is a separate obligation.
    python3 - "$victim" >"$artifact/io-supervisor-$victim.json" <<'PY'
import json, sys
node = int(sys.argv[1])
script = '''set -uo pipefail
touch /tmp/kv9-io.start
while true; do
  while [ ! -f /tmp/kv9-io.start ]; do sleep 0.1; done
  rm -f /tmp/kv9-io.start /tmp/kv9-io.exit
  /usr/local/bin/kv9 start --node-id NODE --addr 0.0.0.0:20160 --data-dir /data > /tmp/kv9-io.log 2>&1 &
  child=$!
  echo "$child" > /tmp/kv9-io.pid
  wait "$child"
  result=$?
  rm -f /tmp/kv9-io.pid
  echo "$result" > /tmp/kv9-io.exit
done
'''.replace('NODE', str(node))
print(json.dumps([
    {'op': 'replace', 'path': '/spec/template/spec/containers/0/command',
     'value': ['/usr/local/bin/no-statx', '/bin/bash', '-c']},
    {'op': 'replace', 'path': '/spec/template/spec/containers/0/args', 'value': [script]},
]))
PY
    k patch deployment "kv9-n$victim" -n "$namespace" --type=json \
      --patch-file "$artifact/io-supervisor-$victim.json" >/dev/null
    k rollout status deployment/"kv9-n$victim" -n "$namespace" --timeout=40s >/dev/null
    io_pod="$(pod_for "$victim")"
    io_uid="$(pod_uid "$victim")"
    k exec -n "$namespace" "$io_pod" -- /usr/local/bin/no-statx --check \
      >"$artifact/io-statx-compatibility-$victim.txt"
    wait_until "IO supervisor starts the real database" 40 io_process_serving
    for errno in 5 28; do
      label="io-voter-$victim-errno-$errno"
      echo "Stage: IOChaos write failure for voter $victim errno $errno"
      io_leader="$(wait_agreed_leader "pre-fault agreement" 30)"
      key_hex="696f2d3${victim}2d$(printf '%s' "$errno" | od -An -tx1 | tr -d ' \n')"
      client "$io_leader" raw-put --addr "$(service_ip "$io_leader"):20160" --keyspace "$keyspace" \
        --key-hex "$key_hex" --value-hex 6265666f7265 >"$artifact/$label-before-put.out"
      io_stop_process
      io_leader="$(wait_majority_leader "surviving majority before restart with I/O fault" 25 "$victim")"
      io_write_fault fault raft-io-fault "$errno"
      k exec -n "$namespace" "$io_pod" -- cat /proc/mounts >"$artifact/$label-mounts.txt"
      grep -Eq ' /data fuse(\.[^ ]+)? ' "$artifact/$label-mounts.txt" || {
        echo 'FAIL: the database restart is not under the IOChaos FUSE mount' >&2; return 1;
      }
      k exec -n "$namespace" "$io_pod" -- /bin/bash -c \
        'rm -f /tmp/kv9-io.exit; touch /tmp/kv9-io.start'
      # Force catch-up traffic against the real reopened Raft log.
      client "$io_leader" raw-put --addr "$(service_ip "$io_leader"):20160" --keyspace "$keyspace" \
        --key-hex "$key_hex" --value-hex 647572696e67 >"$artifact/$label-trigger.out" 2>&1 || true
      wait_until "actual Raft I/O failure terminates voter $victim" 30 io_process_stopped
      k exec -n "$namespace" "$io_pod" -- cat /tmp/kv9-io.log >"$artifact/$label-process.log"
      k exec -n "$namespace" "$io_pod" -- cat /data/status >"$artifact/$label-status.txt"
      rc="$(k exec -n "$namespace" "$io_pod" -- cat /tmp/kv9-io.exit)"
      printf 'pod=%s\nuid=%s\nexit_code=%s\nobserved_at=%s\n' \
        "$io_pod" "$io_uid" "$rc" "$(date --iso-8601=ns)" >"$artifact/$label-exit.txt"
      fatal="$(status_value "$victim" fatal)"
      [ "$rc" = 1 ] && [[ "$fatal" == *'fatal Raft persistence failure during '* ]] &&
        [[ "$fatal" == *"os error $errno"* ]] &&
        grep -Fq 'node runtime failed:' "$artifact/$label-process.log" &&
        ! grep -Fq panicked "$artifact/$label-process.log" || {
        echo "FAIL: voter $victim did not exit through the observed Raft errno $errno path" >&2; return 1;
      }
      [ "$(pod_uid "$victim")" = "$io_uid" ] || {
        echo 'FAIL: Pod replacement invalidated the I/O experiment' >&2; return 1;
      }
      record_fault iochaos raft-io-fault
      io_leader="$(wait_majority_leader "surviving majority after I/O failure" 25 "$victim")"
      client "$io_leader" raw-put --addr "$(service_ip "$io_leader"):20160" --keyspace "$keyspace" \
        --key-hex "$key_hex" --value-hex 6166746572 >"$artifact/$label-majority-put.out"
      io_required_index="$(awk -F= '$1 == "applied_index" {print $2}' "$artifact/$label-majority-put.out")"
      [[ "$io_required_index" =~ ^[0-9]+$ ]] || {
        echo 'FAIL: majority write returned no applied receipt' >&2; return 1;
      }
      client "$io_leader" raw-get --addr "$(service_ip "$io_leader"):20160" --keyspace "$keyspace" \
        --key-hex "$key_hex" >"$artifact/$label-majority-get.out"
      grep -Fxq value_hex=6166746572 "$artifact/$label-majority-get.out"
      k delete iochaos raft-io-fault -n "$namespace" --wait=true >/dev/null
      io_start_process
      io_leader="$(wait_agreed_leader "voter catches up after I/O healing" 40)"
      wait_until "recovered voter applies the majority's acknowledged prefix" 30 io_caught_up
      k exec -n "$namespace" "$io_pod" -- cat /data/status >"$artifact/$label-recovered-status.txt"
      client "$io_leader" raw-get --addr "$(service_ip "$io_leader"):20160" --keyspace "$keyspace" \
        --key-hex "$key_hex" >"$artifact/$label-recovered-get.out"
      grep -Fxq value_hex=6166746572 "$artifact/$label-recovered-get.out"
      echo "PASS: IOChaos voter $victim errno $errno reached Raft, exited, and recovered with majority service"
    done
  done
}
