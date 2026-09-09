#!/usr/bin/env bash
# Sourced by the owned Chaos run. A separate client preserves every invocation
# across the complete existing fault matrix; it is never a database dependency.

persistent_active=0

persistent_build() {
  python3 scripts/build-workload.py --output "$artifact/persistent-build"
  # Docker must copy real files from its context, including when Cargo's target
  # directory is shared outside this worktree. Do not use external symlinks.
  local cargo_target
  cargo_target="$(cargo metadata --locked --no-deps --format-version=1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
  python3 - "$cargo_target" "$artifact/persistent-build" <<'PY'
from pathlib import Path
import shutil, sys
target, build = map(Path, sys.argv[1:])
for source, destination in [
    (target/'debug/kv9', Path('target/debug/kv9')),
    (target/'debug/examples/admission-pressure', Path('target/debug/examples/admission-pressure')),
    (build/'kv9-workload', Path('target/chaos-persistent/kv9-workload')),
    (build/'build.json', Path('target/chaos-persistent/build.json')),
]:
    destination.parent.mkdir(parents=True, exist_ok=True)
    if source.resolve() != destination.resolve(): shutil.copy2(source, destination)
PY
}

persistent_start() {
  local serving created
  persistent_name="persistent-$run_id"
  serving="$(wait_agreed_leader 'leader for persistent workload setup' 30)"
  created="$(client "$serving" create-keyspace --addr "$(service_ip "$serving"):20160" \
    --name "$persistent_name" --api-type raw)"
  printf '%s\n' "$created" >"$artifact/persistent-create.out"
  persistent_keyspace="$(awk -F= '$1 == "keyspace_id" {print $2}' <<<"$created")"
  [[ "$persistent_keyspace" =~ ^[1-9][0-9]*$ ]] || { echo 'FAIL: persistent keyspace lacks a positive ID' >&2; return 1; }
  python3 - "$persistent_name" "$persistent_keyspace" \
    "$(service_ip 1):20160" "$(service_ip 2):20160" "$(service_ip 3):20160" \
    >"$artifact/persistent-config.json" <<'PY'
import json, sys
name, keyspace, *addresses = sys.argv[1:]
print(json.dumps(dict(version=1, client=dict(version=1,
    peers=[dict(node_id=i, address=address) for i,address in enumerate(addresses,1)],
    keyspace_id=int(keyspace), epoch_conf_ver=1, epoch_version=1,
    max_in_flight=2, max_attempts=6, deadline_ms=1500, retry_backoff_ms=10),
    mode='correctness', run_id=name, keyspace_name=name, seed=40, workers=2,
    keys=4, value_bytes=128, mix=dict(get=50, put=40, delete=10),
    warmup_operations=16, max_operations=6000, measure_ms=1800000,
    interval_ms=500, history_bytes=67108864)))
PY
  k create configmap kv9-persistent-config -n "$namespace" \
    --from-file="config.json=$artifact/persistent-config.json" >/dev/null
  k apply -f - >/dev/null <<YAML
apiVersion: v1
kind: Pod
metadata:
  name: kv9-persistent-client
  namespace: $namespace
  labels:
    app: kv9-persistent-client
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
          /usr/local/bin/kv9-workload --config /config/config.json \
            --build-manifest /opt/kv9-workload/build.json --output /tmp/workload \
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
        name: kv9-persistent-config
YAML
  persistent_active=1
  k wait -n "$namespace" --for=condition=Ready pod/kv9-persistent-client --timeout=60s >/dev/null
  k get pod -n "$namespace" kv9-persistent-client -o json >"$artifact/persistent-client-pod.json"
  k exec -n "$namespace" kv9-persistent-client -- cat /tmp/workload-clock-ticks.txt >"$artifact/persistent-clock-ticks.txt"
  wait_until 'persistent baseline acknowledged put and get' 25 persistent_phase_observed baseline
  cp "$artifact/persistent-progress.json" "$artifact/baseline-persistent-progress.json"
  echo 'PASS: persistent workload initialized through public gRPC and retained baseline progress'
}

persistent_set_phase() {
  if [ "$persistent_active" != 1 ]; then return 0; fi
  k exec -n "$namespace" kv9-persistent-client -- /bin/bash -c \
    'test ! -e /tmp/workload.exit && printf "%s\n" "$1" > /tmp/workload.phase.tmp && mv /tmp/workload.phase.tmp /tmp/workload.phase' \
    phase "$1"
}

persistent_phase_observed() {
  k exec -n "$namespace" kv9-persistent-client -- /bin/bash -c \
    'test ! -e /tmp/workload.exit && cat /tmp/workload/progress.json' \
    >"$artifact/persistent-progress.tmp" 2>/dev/null || return 1
  python3 - "$artifact/persistent-progress.tmp" "$1" <<'PY'
import json, sys
record=json.load(open(sys.argv[1]))
rows=[r for r in record['progress']['phase_successes'] if r['phase']==sys.argv[2]]
sys.exit(0 if record['stage']=='measure' and len(rows)==1 and rows[0]['put']>0 and rows[0]['get']>0 else 1)
PY
  local rc=$?
  if (( rc == 0 )); then mv "$artifact/persistent-progress.tmp" "$artifact/persistent-progress.json"; fi
  return "$rc"
}

persistent_phase() {
  local phase="$1"
  wait_until "persistent put and read complete during $phase" 25 persistent_phase_observed "$phase"
  cp "$artifact/persistent-progress.json" "$artifact/$phase-persistent-progress.json"
  k get podchaos,networkchaos,iochaos -n "$namespace" -o json >"$artifact/$phase-persistent-faults.json"
  if [[ "$phase" == io-voter-* ]]; then
    k get pod -n "$namespace" "$io_pod" -o json >"$artifact/$phase-persistent-victim.json"
  fi
  date --iso-8601=ns >"$artifact/$phase-persistent-observed-at.txt"
  echo "PASS: persistent put and read completed during $phase"
}

persistent_collect() {
  mkdir -p "$artifact/persistent-run"
  local file
  for file in config.json build.json history.jsonl report.json ready.json progress.json; do
    k exec -n "$namespace" kv9-persistent-client -- cat "/tmp/workload/$file" \
      >"$artifact/persistent-run/$file" 2>"$artifact/persistent-run/$file.copy-error" || true
  done
  k exec -n "$namespace" kv9-persistent-client -- cat /tmp/workload.log >"$artifact/persistent-workload.log" 2>&1 || true
  k exec -n "$namespace" kv9-persistent-client -- cat /tmp/workload.exit >"$artifact/persistent-exit.txt" 2>&1 || true
}

persistent_cleanup() {
  if [ "$persistent_active" != 1 ]; then return 0; fi
  k exec -n "$namespace" kv9-persistent-client -- touch /tmp/workload.stop >/dev/null 2>&1 || true
  # Failure cleanup preserves the live namespace. It retains the current
  # artifacts even if no final report exists; it cannot turn failure into success.
  persistent_collect
}

persistent_finished() {
  k exec -n "$namespace" kv9-persistent-client -- test -s /tmp/workload.exit 2>/dev/null
}

persistent_finish() {
  k exec -n "$namespace" kv9-persistent-client -- touch /tmp/workload.stop
  wait_until 'persistent workload drains and verifies its final dataset' 60 persistent_finished
  persistent_collect
  [ "$(cat "$artifact/persistent-exit.txt")" = 0 ] || { echo 'FAIL: persistent workload did not exit successfully' >&2; return 1; }
  local revision_args=()
  if [ -n "${GITHUB_SHA:-}" ]; then revision_args=(--expected-revision "$GITHUB_SHA"); fi
  python3 scripts/check-persistent-chaos.py "$artifact" "${revision_args[@]}"
  persistent_active=0
  k delete pod kv9-persistent-client -n "$namespace" --wait=true >/dev/null
  local serving
  serving="$(wait_agreed_leader 'database service after persistent collector removal' 30)"
  client "$serving" raw-put --addr "$(service_ip "$serving"):20160" --keyspace "$persistent_keyspace" \
    --key-hex 636f6c6c6563746f722d676f6e65 --value-hex 7375727669766564 >"$artifact/persistent-after-collector-put.out"
  client "$serving" raw-get --addr "$(service_ip "$serving"):20160" --keyspace "$persistent_keyspace" \
    --key-hex 636f6c6c6563746f722d676f6e65 >"$artifact/persistent-after-collector-get.out"
  grep -Fxq 'value_hex=7375727669766564' "$artifact/persistent-after-collector-get.out"
  echo 'PASS: database read and write succeeded after persistent collector removal'
}
