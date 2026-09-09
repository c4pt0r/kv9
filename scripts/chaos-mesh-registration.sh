#!/usr/bin/env bash
# Sourced by the isolated Chaos Mesh runner while its history client is active.

registration_joiner_serving() {
  [ "$(status_value 4 bootstrap_state)" = Serving ] &&
    [ -z "$(status_value 4 fatal)" ] &&
    [ "$(status_value 4 registration_last)" = registered ]
}

registration_fault_effect() {
  local label="$1"
  if tcp_probe 4 1; then
    echo 'FAIL: the first registration seed is still reachable from the joiner' >&2
    return 1
  fi
  tcp_probe 4 2 && tcp_probe 4 3 || {
    echo 'FAIL: the joiner cannot reach the surviving quorum' >&2; return 1;
  }
  printf 'observed_at=%s\nn4_to_n1=blocked\nn4_to_n2=reachable\nn4_to_n3=reachable\n' \
    "$(date --iso-8601=ns)" >"$artifact/registration-$label-effect.txt"
  record_fault networkchaos registration-seed-blackhole
}

run_registration_seed_fault() {
  local join_pod join_ip join_ticket join_leader
  echo 'Stage: joining through healthy seeds while the first seed is partitioned'
  write_service 4
  write_pvc 4
  # Wait for a ticket inside a fixture shell so we can learn the Pod's canonical
  # IP and install the fault before the production join/start path begins.
  python3 - "$namespace" "$image" >"$artifact/registration-joiner.json" <<'PY'
import json, sys
namespace, image = sys.argv[1:]
script = '''set -euo pipefail
while [ ! -s /tmp/join-ticket ]; do sleep 0.1; done
export KV9_JOIN_TICKET="$(cat /tmp/join-ticket)"
if [ ! -f /data/kv9-store-identity ]; then
  /usr/local/bin/kv9 join --root /root/root.bin --node-id 4 --addr "$POD_IP:20160" --data-dir /data
fi
exec /usr/local/bin/kv9 start --node-id 4 --addr "$POD_IP:20160" --data-dir /data
'''
labels = {'app': 'kv9', 'kv9-node': '4'}
container = dict(name='kv9', image=image, imagePullPolicy='IfNotPresent',
                 command=['/bin/bash', '-c'], args=[script], env=[
                     {'name': 'POD_IP', 'valueFrom': {'fieldRef': {'fieldPath': 'status.podIP'}}},
                     *[{'name': name, 'valueFrom': {'secretKeyRef': {'name': 'kv9-auth', 'key': key}}}
                       for name, key in [('KV9_CLUSTER_TOKEN', 'cluster-token'),
                                         ('KV9_CLIENT_TOKENS', 'client-tokens')]]],
                 volumeMounts=[{'name': 'data', 'mountPath': '/data'},
                               {'name': 'root', 'mountPath': '/root', 'readOnly': True}])
print(json.dumps(dict(apiVersion='apps/v1', kind='Deployment',
    metadata=dict(name='kv9-n4', namespace=namespace),
    spec=dict(replicas=1, strategy={'type': 'Recreate'}, selector={'matchLabels': labels},
              template=dict(metadata={'labels': labels}, spec=dict(
                  terminationGracePeriodSeconds=2, containers=[container], volumes=[
                      {'name': 'data', 'persistentVolumeClaim': {'claimName': 'kv9-data-n4'}},
                      {'name': 'root', 'configMap': {'name': 'kv9-root'}}]))))))
PY
  k apply -f "$artifact/registration-joiner.json" >/dev/null
  k rollout status deployment/kv9-n4 -n "$namespace" --timeout=30s >/dev/null
  join_pod="$(pod_for 4)"
  join_ip="$(k get pod -n "$namespace" "$join_pod" -o jsonpath='{.status.podIP}')"
  [[ "$join_ip" =~ ^[0-9.]+$ ]] || { echo 'FAIL: joiner has no canonical IPv4 Pod address' >&2; return 1; }
  # Isolate seed 1 from the other voters too, guaranteeing a reachable leader
  # in the surviving 2/3 quorum. Isolating ONLY the joiner-to-leader edge would
  # not satisfy the conditional progress premise of a reachable leader.
  cat >"$artifact/registration-seed-blackhole.request.yaml" <<YAML
apiVersion: chaos-mesh.org/v1alpha1
kind: NetworkChaos
metadata:
  name: registration-seed-blackhole
  namespace: $namespace
spec:
  action: partition
  mode: all
  selector:
    namespaces: ["$namespace"]
    labelSelectors:
      app: kv9
      kv9-node: "1"
  direction: both
  target:
    mode: all
    selector:
      namespaces: ["$namespace"]
      labelSelectors:
        app: kv9
      expressionSelectors:
        - key: kv9-node
          operator: NotIn
          values: ["1"]
YAML
  k apply -f "$artifact/registration-seed-blackhole.request.yaml" >/dev/null
  wait_injected networkchaos registration-seed-blackhole
  registration_fault_effect before
  join_leader="$(wait_majority_leader 'reachable leader for seed-blackhole registration' 25 1)"
  client "$join_leader" admit-node --addr "$(service_ip "$join_leader"):20160" \
    --node-id 4 --node-addr "$join_ip:20160" --ttl-seconds 120 >"$artifact/registration-admit.out"
  join_ticket="$(awk -F= '$1 == "join_ticket" {print $2}' "$artifact/registration-admit.out")"
  [[ "$join_ticket" =~ ^[0-9a-f]{64}$ ]] || { echo 'FAIL: no admission ticket' >&2; return 1; }
  printf '%s\n' "$join_ticket" | k exec -i -n "$namespace" "$join_pod" -- \
    /bin/bash -c 'umask 077; cat > /tmp/join-ticket'
  wait_until 'joiner registers and applies its exact receipt despite the first-seed blackhole' 45 registration_joiner_serving
  history_phase registration-seed-blackhole
  k exec -n "$namespace" "$join_pod" -- cat /data/status >"$artifact/registration-serving-status.txt"
  python3 - "$artifact/registration-serving-status.txt" <<'PY'
from pathlib import Path
import sys
s = dict(line.split('=', 1) for line in Path(sys.argv[1]).read_text().splitlines())
assert s['bootstrap_state'] == 'Serving' and s['fatal'] == ''
assert s['meta_voters'] == '1,2,3' and s['meta_learners'] == '4'
assert int(s['registration_attempts']) >= 2 and int(s['registration_errors']) >= 1
assert s['registration_last'] == 'registered' and s['registration_last_walk'] == 'registered'
assert int(s['registration_receipt_term']) > 0 and int(s['registration_receipt_index']) > 0
assert int(s['driver_applied_index']) >= int(s['registration_receipt_index'])
PY
  # Raw reads currently require a leader. The caught-up learner must still
  # refuse with a typed leader hint; Serving alone grants no read authority.
  local read_rc=0
  client 4 raw-get --addr "$join_ip:20160" --keyspace "$keyspace" --key-hex 62617365 \
    >"$artifact/registration-learner-get.out" 2>"$artifact/registration-learner-get.err" || read_rc=$?
  PYTHONPATH=scripts python3 - "$artifact" "$read_rc" <<'PYTHON'
from pathlib import Path
import subprocess, sys
from chaos_client import refused
root = Path(sys.argv[1])
result = subprocess.CompletedProcess([], int(sys.argv[2]),
    (root / 'registration-learner-get.out').read_text(),
    (root / 'registration-learner-get.err').read_text())
assert refused(result), 'learner read did not exclusively refuse with NotLeader'
assert result.stderr.splitlines()[0] in {
    'not_leader=true leader_node_id=2', 'not_leader=true leader_node_id=3'}
PYTHON
  KV9_CLIENT_TOKEN="$client_token" python3 scripts/chaos_client.py \
    --kubectl "$kubectl_bin" --kubeconfig "$kubeconfig" --namespace "$namespace" \
    --addresses "2=$(service_ip 2):20160,3=$(service_ip 3):20160" \
    --evidence "$artifact/registration-survivor-get-attempts.jsonl" -- \
    raw-get --keyspace "$keyspace" --key-hex 62617365 >"$artifact/registration-survivor-get.out"
  grep -Fxq 'value_hex=7631' "$artifact/registration-survivor-get.out"
  registration_fault_effect after
  history_set_phase healing
  k delete networkchaos registration-seed-blackhole -n "$namespace" --wait=true >/dev/null
  wait_agreed_leader 'voters converge after registration seed healing' 35 >/dev/null
  echo 'PASS: joiner reached Serving with an exact receipt while its first seed was blackholed'
}
