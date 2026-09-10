#!/usr/bin/env bash
# Sourced by chaos-mesh-e2e.sh; enabled only by an explicit native plan. A separate client preserves every invocation
# across the complete existing fault matrix; it is never a database dependency.

native_active=0
native_helpers="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
native_enabled() { [ -n "${KV9_NATIVE_BATCH_CHAOS_PLAN:-}" ]; }

native_start() {
  native_enabled || return 0
  python3 "$native_helpers/point-client-identity.py" --plan "$KV9_NATIVE_BATCH_CHAOS_PLAN" --artifact "$artifact" --namespace "$namespace"
  local serving created
  native_name="native-batch-$run_id"
  serving="$(wait_agreed_leader 'leader for native workload setup' 30)"
  created="$(client "$serving" create-keyspace --addr "$(service_ip "$serving"):20160" \
    --name "$native_name" --api-type raw)"
  printf '%s\n' "$created" >"$artifact/native-create.out"
  native_keyspace="$(awk -F= '$1 == "keyspace_id" {print $2}' <<<"$created")"
  [[ "$native_keyspace" =~ ^[1-9][0-9]*$ ]] || { echo 'FAIL: native keyspace lacks a positive ID' >&2; return 1; }
  python3 - "$native_name" "$native_keyspace" \
    "$(service_ip 1):20160" "$(service_ip 2):20160" "$(service_ip 3):20160" \
    >"$artifact/native-config.json" <<'PY'
import json, sys
name, keyspace, *addresses = sys.argv[1:]
print(json.dumps(dict(version=1, rpc_transport='tonic_stream', client=dict(version=1,
    peers=[dict(node_id=i, address=address) for i,address in enumerate(addresses,1)],
    keyspace_id=int(keyspace), epoch_conf_ver=1, epoch_version=1,
    max_in_flight=4, max_attempts=6, deadline_ms=1500, retry_backoff_ms=10),
    run_id=name, keyspace_name=name, seed=40, workers=4,
    keys=4, batch_size=8, value_bytes=128, mix=[10,10,10,35,35],
    max_calls=6000, measure_ms=1800000, interval_ms=500, history_bytes=268435456)))
PY
  k create configmap kv9-native-batch-config -n "$namespace" \
    --from-file="config.json=$artifact/native-config.json" >/dev/null
  k apply -f - >/dev/null <<YAML
apiVersion: v1
kind: Pod
metadata:
  name: kv9-native-batch-client
  namespace: $namespace
  labels:
    app: kv9-native-batch-client
spec:
  restartPolicy: Never
  containers:
    - name: workload
      image: $image
      imagePullPolicy: IfNotPresent
      command: ["/bin/bash", "-c"]
      args:
        - |
          umask 077
          getconf CLK_TCK > /tmp/workload-clock-ticks.txt
          printf 'baseline\n' > /tmp/workload.phase
          /usr/local/bin/kv9-batch-workload --config /config/config.json \
            --build-manifest /opt/kv9-batch-workload/build.json --output /tmp/workload \
            --stop-file /tmp/workload.stop --phase-file /tmp/workload.phase \
            > /tmp/workload.log 2>&1
          echo \$? > /tmp/workload.exit
          sleep 3600
      env:
        - name: KV9_CLIENT_TOKEN
          valueFrom:
            secretKeyRef:
              name: kv9-auth
              key: client-token
      resources:
        requests:
          cpu: 100m
          memory: 64Mi
        limits:
          cpu: "2"
          memory: 256Mi
      readinessProbe:
        exec:
          command: ["/bin/bash", "-c", "test -s /tmp/workload/ready.json && test ! -e /tmp/workload.exit"]
        periodSeconds: 1
        timeoutSeconds: 1
        failureThreshold: 60
      volumeMounts:
        - name: config
          mountPath: /config
          readOnly: true
  volumes:
    - name: config
      configMap:
        name: kv9-native-batch-config
YAML
  native_active=1
  k wait -n "$namespace" --for=condition=Ready pod/kv9-native-batch-client --timeout=60s >/dev/null
  k get pod -n "$namespace" kv9-native-batch-client -o json >"$artifact/native-client-pod.json"
  k exec -n "$namespace" kv9-native-batch-client -- cat /tmp/workload-clock-ticks.txt >"$artifact/native-clock-ticks.txt"
  python3 "$native_helpers"/native-gate.py --plan "$KV9_NATIVE_BATCH_CHAOS_PLAN" \
    --artifact "$artifact" --namespace "$namespace" --phase baseline
}

native_set_phase() {
  if [ "$native_active" != 1 ]; then return 0; fi
  k exec -n "$namespace" kv9-native-batch-client -- /bin/bash -c \
    'test ! -e /tmp/workload.exit && printf "%s\n" "$1" > /tmp/workload.phase.tmp && mv /tmp/workload.phase.tmp /tmp/workload.phase' \
    phase "$1"
}

native_phase() {
  native_enabled || return 0
  python3 "$native_helpers"/native-gate.py --plan "$KV9_NATIVE_BATCH_CHAOS_PLAN" \
    --artifact "$artifact" --namespace "$namespace" --phase "$1"
}

native_collect() {
  mkdir -p "$artifact/native-run"
  local file
  for file in config.json build.json history.jsonl report.json ready.json progress.json; do
    k exec -n "$namespace" kv9-native-batch-client -- cat "/tmp/workload/$file" \
      >"$artifact/native-run/$file" 2>"$artifact/native-run/$file.copy-error" || true
  done
  k exec -n "$namespace" kv9-native-batch-client -- cat /tmp/workload.log >"$artifact/native-workload.log" 2>&1 || true
  k exec -n "$namespace" kv9-native-batch-client -- cat /tmp/workload.exit >"$artifact/native-exit.txt" 2>&1 || true
}

native_cleanup() {
  if [ "$native_active" != 1 ]; then return 0; fi
  k exec -n "$namespace" kv9-native-batch-client -- touch /tmp/workload.stop >/dev/null 2>&1 || true
  # Failure cleanup preserves the live namespace. It retains the current
  # artifacts even if no final report exists; it cannot turn failure into success.
  native_collect
}

native_finished() {
  k exec -n "$namespace" kv9-native-batch-client -- test -s /tmp/workload.exit 2>/dev/null
}

native_finish() {
  native_enabled || return 0
  k exec -n "$namespace" kv9-native-batch-client -- touch /tmp/workload.stop
  wait_until 'native workload drains and verifies its final dataset' 60 native_finished
  native_collect
  [ "$(cat "$artifact/native-exit.txt")" = 0 ] || { echo 'FAIL: native workload did not exit successfully' >&2; return 1; }
  k exec -n "$namespace" kv9-native-batch-client -- /bin/bash -c '
    set -euo pipefail
    pid=$(sed -n "s/.*\"process_id\": *\([0-9]*\).*/\1/p" /tmp/workload/ready.json)
    [[ "$pid" =~ ^[1-9][0-9]*$ ]]
    printf "process_pid=%s\nprocess_boot_id=" "$pid"
    cat /proc/sys/kernel/random/boot_id
    if test -e "/proc/$pid"; then cat "/proc/$pid/stat"; exit 1; fi
    printf "process_absent=true\n"
  ' >"$artifact/native-process-exit.txt"
  native_active=0
  k get pod -n "$namespace" kv9-native-batch-client -o json >"$artifact/native-client-final-pod.json"
  k delete pod kv9-native-batch-client -n "$namespace" --wait=true >/dev/null
  k get pods -n "$namespace" -o json >"$artifact/native-after-collector-pods.json"
  echo 'PASS: native batch workload stopped, retained full history and exited before collector removal'
}

# Prebuilt mode never calls persistent_build or stages into the source tree.
native_prebuilt() {
  python3 "$native_helpers/verify-prebuilt.py" --plan "$KV9_NATIVE_BATCH_CHAOS_PLAN" \
    --copy-workload "$artifact/persistent-build" --copy-native "$artifact/native-build"
}
native_catalog() {
  python3 - "$keyspace" "$persistent_name" "$persistent_keyspace" "$native_name" "$native_keyspace" >"$artifact/history-initial.json" <<'PYTHON'
import json, sys
print(json.dumps({'keyspaces': [{'name': 'chaos', 'id': int(sys.argv[1])},
                               {'name': sys.argv[2], 'id': int(sys.argv[3])},
                               {'name': sys.argv[4], 'id': int(sys.argv[5])}]}))
PYTHON
}
native_delay_gate() {
  native_enabled || return 0
  python3 "$native_helpers/delay-gate.py" --plan "$KV9_NATIVE_BATCH_CHAOS_PLAN" \
    --artifact "$artifact" --namespace "$namespace" --timeout-seconds 20
}
native_check() {
  native_enabled || return 0
  python3 "$native_helpers/native-final-drain.py" --plan "$KV9_NATIVE_BATCH_CHAOS_PLAN" --artifact "$artifact" --namespace "$namespace"
  python3 "$native_helpers/check-native-windows.py" --plan "$KV9_NATIVE_BATCH_CHAOS_PLAN" --artifact "$artifact"
}
