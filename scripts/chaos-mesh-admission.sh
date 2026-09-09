#!/usr/bin/env bash
# Sourced by chaos-mesh-e2e.sh; uses only that run's explicitly selected cluster.

admission_metrics_during() {
  k exec -n "$namespace" "$admission_pressure_pod" -- cat /data/metrics.json >"$artifact/admission-during.metrics.json"
  python3 scripts/check-latency-metrics.py "$artifact" --pressure during 2>/dev/null
}

admission_metrics_after() {
  k exec -n "$namespace" "$admission_pressure_pod" -- cat /data/metrics.json >"$artifact/admission-after.metrics.json"
  python3 scripts/check-latency-metrics.py "$artifact" --pressure after 2>/dev/null
}

admission_pressure_ready() {
  kill -0 "$admission_pressure_pid" || return 1
  k exec -n "$namespace" kv9-history-client -- test -s /tmp/admission.ready 2>/dev/null
}

admission_pressure_drained() {
  k exec -n "$namespace" "$admission_pressure_pod" -- cat /data/status >"$artifact/admission-after.status"
  python3 - "$artifact" <<'PYTHON'
import json, pathlib, sys
root = pathlib.Path(sys.argv[1])
def fields(name): return dict(line.split('=', 1) for line in (root / name).read_text().splitlines())
a, b = fields('admission-after.status'), fields('admission-before.status')
done = json.loads((root / 'admission-pressure.jsonl').read_text().splitlines()[-1])
def counter(state, name): return int(dict(p.split('=') for p in state['public_rpc_raw_read'].split(','))[name])
ready = (a['public_rpc_in_flight'] == '0' and a['public_rpc_encoded_bytes'] == '0'
         and counter(a, 'completed') - counter(b, 'completed') >= done['missing']
         and counter(a, 'refused_count') - counter(b, 'refused_count') >= done['count_refusals'])
sys.exit(0 if ready else 1)
PYTHON
}

admission_partition_effect() {
  local phase="$1" survivor="$2" isolated="$3"
  if tcp_probe "$survivor" "$isolated"; then
    echo 'FAIL: admission pressure requires an observed live partition' >&2; return 1
  fi
  printf 'observed_at=%s\nsurvivor=%s\nisolated=%s\nconnection=blocked\n' \
    "$(date --iso-8601=ns)" "$survivor" "$isolated" >"$artifact/admission-$phase-effect.txt"
  k get -n "$namespace" networkchaos/isolate-leader -o json >"$artifact/admission-$phase-fault.json"
}

run_public_admission_pressure() {
  local serving="$1" isolated="$2" survivor="$3"
  echo 'Stage: bounded public admission under a live leader partition'
  admission_pressure_pod="$(pod_for "$serving")"
  admission_partition_effect before "$survivor" "$isolated"
  k exec -n "$namespace" "$admission_pressure_pod" -- cat /data/status >"$artifact/admission-before.status"
  k exec -n "$namespace" "$admission_pressure_pod" -- cat /data/metrics.json >"$artifact/admission-before.metrics.json"
  k exec -n "$namespace" kv9-history-client -- env KV9_CLIENT_TOKEN="$client_token" \
    timeout 50 /usr/local/bin/admission-pressure "$(service_ip "$serving"):20160" "$keyspace" \
    /tmp/admission.ready /tmp/admission.stop >"$artifact/admission-pressure.jsonl" 2>"$artifact/admission-pressure.err" &
  admission_pressure_pid=$!
  wait_until 'actual public count refusals and successful reads' 15 admission_pressure_ready
  k exec -n "$namespace" kv9-history-client -- cat /tmp/admission.ready >"$artifact/admission-ready.txt"
  k exec -n "$namespace" "$admission_pressure_pod" -- cat /data/status >"$artifact/admission-during.status"
  wait_until 'pressure reaches queue/backend/read observers' 10 admission_metrics_during
  history_phase public-admission-overload
  admission_partition_effect after "$survivor" "$isolated"
  k exec -n "$namespace" kv9-history-client -- touch /tmp/admission.stop
  wait "$admission_pressure_pid"
  wait_until 'public reservations drain after pressure' 10 admission_pressure_drained
  wait_until 'post-pressure metrics include completed jobs' 10 admission_metrics_after
  python3 scripts/check-admission-pressure.py "$artifact" --self-test
  echo 'PASS: bounded public admission refused pressure while the partitioned quorum served history'
}
