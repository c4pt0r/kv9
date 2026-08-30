#!/usr/bin/env bash
# Root-certified three-node fault acceptance on a real Chaos Mesh installation.
set -euo pipefail

kubectl_bin="${KUBECTL:-kubectl}"
kind_bin="${KIND:-kind}"
kind_cluster="${KV9_KIND_CLUSTER:-kv9-chaos}"
image="${KV9_CHAOS_IMAGE:-kv9-chaos:e2e}"
bin="${KV9_BIN:-./target/debug/kv9}"
kubeconfig="${KUBECONFIG:-}"

[ -n "$kubeconfig" ] || { echo "FAIL: KUBECONFIG must name the isolated Chaos Mesh cluster" >&2; exit 1; }
[ -x "$bin" ] || { echo "FAIL: $bin is not executable" >&2; exit 1; }
command -v "$kubectl_bin" >/dev/null || { echo "FAIL: kubectl is unavailable" >&2; exit 1; }
command -v "$kind_bin" >/dev/null || { echo "FAIL: kind is unavailable" >&2; exit 1; }
command -v docker >/dev/null || { echo "FAIL: docker is unavailable" >&2; exit 1; }

run_id="$(date +%s)-$$"
namespace="kv9-chaos-$run_id"
artifact="$(mktemp -d /tmp/kv9-chaos-e2e.XXXXXX)"
bootstrap_token="chaos-bootstrap-$run_id"
cluster_token="chaos-cluster-$run_id"
client_token="chaos-client-$run_id"
root="$artifact/root.bin"

k() {
  KUBECONFIG="$kubeconfig" "$kubectl_bin" "$@"
}

collect_scene() {
  k get all -n "$namespace" -o wide >"$artifact/kubernetes.txt" 2>&1 || true
  k get podchaos,networkchaos -n "$namespace" -o yaml >"$artifact/chaos.yaml" 2>&1 || true
  local pod
  while read -r pod; do
    [ -n "$pod" ] || continue
    k describe pod -n "$namespace" "$pod" >"$artifact/$pod.describe" 2>&1 || true
    k logs -n "$namespace" "$pod" --all-containers >"$artifact/$pod.log" 2>&1 || true
    k logs -n "$namespace" "$pod" --all-containers --previous \
      >"$artifact/$pod.previous.log" 2>&1 || true
    k exec -n "$namespace" "$pod" -- cat /data/status \
      >"$artifact/$pod.status" 2>&1 || true
  done < <(k get pods -n "$namespace" -o name 2>/dev/null | sed 's#pod/##')
}

cleanup() {
  local rc=$?
  trap - EXIT
  if k get namespace "$namespace" >/dev/null 2>&1; then
    (( rc == 0 )) || collect_scene
    k delete podchaos,networkchaos --all -n "$namespace" --ignore-not-found \
      --wait=true >/dev/null 2>&1 || true
    k delete namespace "$namespace" --wait=true >/dev/null 2>&1 || true
  fi
  if (( rc == 0 )); then
    rm -rf "$artifact"
  else
    echo "FAIL: preserving Chaos Mesh evidence at $artifact" >&2
  fi
  exit "$rc"
}
trap cleanup EXIT

wait_until() {
  local label="$1" timeout="$2"; shift 2
  local deadline=$((SECONDS + timeout))
  while (( SECONDS < deadline )); do
    if "$@"; then return 0; fi
    sleep 0.25
  done
  echo "FAIL: timed out waiting for $label" >&2
  collect_scene
  return 1
}

wait_agreed_leader() {
  local label="$1" timeout="$2" leader deadline
  deadline=$((SECONDS + timeout))
  while (( SECONDS < deadline )); do
    if leader="$(agreed_leader)"; then
      printf '%s\n' "$leader"
      return 0
    fi
    sleep 0.25
  done
  echo "FAIL: timed out waiting for $label" >&2
  collect_scene
  return 1
}

wait_majority_leader() {
  local label="$1" timeout="$2" isolated="$3" leader deadline
  deadline=$((SECONDS + timeout))
  while (( SECONDS < deadline )); do
    if leader="$(majority_leader_without "$isolated")"; then
      printf '%s\n' "$leader"
      return 0
    fi
    sleep 0.25
  done
  echo "FAIL: timed out waiting for $label" >&2
  collect_scene
  return 1
}

pod_for() {
  local node="$1"
  k get pod -n "$namespace" -l "app=kv9,kv9-node=$node" \
    --field-selector=status.phase=Running -o jsonpath='{.items[0].metadata.name}' 2>/dev/null
}

pod_uid() {
  local pod
  pod="$(pod_for "$1")" || return 1
  k get pod -n "$namespace" "$pod" -o jsonpath='{.metadata.uid}' 2>/dev/null
}

restart_count() {
  local pod
  pod="$(pod_for "$1")" || return 1
  k get pod -n "$namespace" "$pod" -o jsonpath='{.status.containerStatuses[0].restartCount}' 2>/dev/null
}

status_value() {
  local pod node="$1" key="$2"
  pod="$(pod_for "$node")" || return 1
  k exec -n "$namespace" "$pod" -- cat /data/status 2>/dev/null |
    awk -F= -v wanted="$key" '$1 == wanted { print substr($0, length($1) + 2); found=1 } END { if (!found) exit 1 }'
}

node_serving() {
  [ "$(status_value "$1" bootstrap_state 2>/dev/null || true)" = Serving ] &&
    [ -z "$(status_value "$1" fatal 2>/dev/null || true)" ]
}

agreed_leader() {
  local node seen leader=""
  for node in 1 2 3; do
    node_serving "$node" || return 1
    seen="$(status_value "$node" leader_id 2>/dev/null || true)"
    [[ "$seen" =~ ^[1-3]$ ]] || return 1
    if [ -z "$leader" ]; then leader="$seen"; fi
    [ "$seen" = "$leader" ] || return 1
  done
  printf '%s\n' "$leader"
}

majority_leader_without() {
  local isolated="$1" node seen leader=""
  for node in 1 2 3; do
    (( node == isolated )) && continue
    node_serving "$node" || return 1
    seen="$(status_value "$node" leader_id 2>/dev/null || true)"
    [[ "$seen" =~ ^[1-3]$ ]] || return 1
    [ "$seen" != "$isolated" ] || return 1
    if [ -z "$leader" ]; then leader="$seen"; fi
    [ "$seen" = "$leader" ] || return 1
  done
  printf '%s\n' "$leader"
}

service_ip() {
  k get service -n "$namespace" "kv9-n$1" -o jsonpath='{.spec.clusterIP}'
}

client() {
  local node="$1"; shift
  local pod
  pod="$(pod_for "$node")"
  k exec -n "$namespace" "$pod" -- env KV9_CLIENT_TOKEN="$client_token" \
    /usr/local/bin/kv9 client "$@"
}

wait_injected() {
  local kind="$1" name="$2"
  k wait -n "$namespace" --for=condition=AllInjected "$kind/$name" --timeout=15s >/dev/null
}

write_service() {
  local node="$1"
  k apply -f - >/dev/null <<YAML
apiVersion: v1
kind: Service
metadata:
  name: kv9-n$node
  namespace: $namespace
spec:
  selector:
    app: kv9
    kv9-node: "$node"
  ports:
    - name: grpc
      port: 20160
      targetPort: 20160
YAML
}

write_deployment() {
  local node="$1" root_config="${2:-kv9-root}" bootstrap="${3:-$bootstrap_token}"
  k apply -f - >/dev/null <<YAML
apiVersion: apps/v1
kind: Deployment
metadata:
  name: kv9-n$node
  namespace: $namespace
spec:
  replicas: 1
  strategy:
    type: Recreate
  selector:
    matchLabels:
      app: kv9
      kv9-node: "$node"
  template:
    metadata:
      labels:
        app: kv9
        kv9-node: "$node"
    spec:
      terminationGracePeriodSeconds: 2
      containers:
        - name: kv9
          image: $image
          imagePullPolicy: IfNotPresent
          command: ["/bin/bash", "-c"]
          args:
            - |
              set -euo pipefail
              if [ ! -f /data/kv9-store-identity ]; then
                KV9_BOOTSTRAP_TOKEN='$bootstrap' /usr/local/bin/kv9 init \
                  --root /root/root.bin --node-id '$node' --data-dir /data
              fi
              exec /usr/local/bin/kv9 start --node-id '$node' \
                --addr 0.0.0.0:20160 --data-dir /data
          env:
            - name: KV9_CLUSTER_TOKEN
              valueFrom:
                secretKeyRef:
                  name: kv9-auth
                  key: cluster-token
            - name: KV9_CLIENT_TOKENS
              valueFrom:
                secretKeyRef:
                  name: kv9-auth
                  key: client-tokens
          volumeMounts:
            - name: data
              mountPath: /data
            - name: root
              mountPath: /root
              readOnly: true
      volumes:
        - name: data
          hostPath:
            path: /var/lib/$namespace/n$node
            type: DirectoryOrCreate
        - name: root
          configMap:
            name: $root_config
YAML
}

apply_partition() {
  local node="$1"
  k apply -f - >/dev/null <<YAML
apiVersion: chaos-mesh.org/v1alpha1
kind: NetworkChaos
metadata:
  name: isolate-leader
  namespace: $namespace
spec:
  action: partition
  mode: all
  selector:
    namespaces: ["$namespace"]
    labelSelectors:
      app: kv9
      kv9-node: "$node"
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
          values: ["$node"]
YAML
}

apply_delay() {
  local node="$1"
  k apply -f - >/dev/null <<YAML
apiVersion: chaos-mesh.org/v1alpha1
kind: NetworkChaos
metadata:
  name: delay-follower
  namespace: $namespace
spec:
  action: delay
  mode: all
  selector:
    namespaces: ["$namespace"]
    labelSelectors:
      app: kv9
      kv9-node: "$node"
  direction: both
  delay:
    latency: 200ms
    jitter: 50ms
    correlation: "25"
  target:
    mode: all
    selector:
      namespaces: ["$namespace"]
      labelSelectors:
        app: kv9
      expressionSelectors:
        - key: kv9-node
          operator: NotIn
          values: ["$node"]
YAML
}

echo "Building kv9 and loading $image into kind/$kind_cluster"
cargo build --bin kv9 >/dev/null
docker build -q -f chaos/Dockerfile -t "$image" . >/dev/null
KUBECONFIG="$kubeconfig" "$kind_bin" load docker-image --name "$kind_cluster" "$image" >/dev/null

k create namespace "$namespace" >/dev/null
k annotate namespace "$namespace" chaos-mesh.org/inject=enabled --overwrite >/dev/null
k create secret generic kv9-auth -n "$namespace" \
  --from-literal=cluster-token="$cluster_token" \
  --from-literal=client-tokens="admin=$client_token" >/dev/null

for node in 1 2 3 9; do write_service "$node"; done
voters="1@$(service_ip 1):20160,2@$(service_ip 2):20160,3@$(service_ip 3):20160"
KV9_BOOTSTRAP_TOKEN="$bootstrap_token" "$bin" root-create --output "$root" --voters "$voters" \
  >"$artifact/root-create.out"
k create configmap kv9-root -n "$namespace" --from-file="root.bin=$root" >/dev/null
for node in 1 2 3; do write_deployment "$node"; done

echo "Stage: bootstrap and baseline mutation"
leader="$(wait_agreed_leader "root-certified cluster Serving" 45)"
create_out="$(client "$leader" create-keyspace --addr "$(service_ip "$leader"):20160" \
  --name chaos --api-type raw)"
keyspace="$(awk -F= '$1=="keyspace_id" {print $2}' <<<"$create_out")"
[[ "$keyspace" =~ ^[0-9]+$ ]] || { echo "FAIL: create-keyspace returned no id" >&2; exit 1; }
client "$leader" raw-put --addr "$(service_ip "$leader"):20160" --keyspace "$keyspace" \
  --key-hex 62617365 --value-hex 7631 >"$artifact/baseline-put.out"

# A second root that overlaps node 1 cannot become endorsed by the live root.
echo "Stage: conflicting root cannot cross-endorse"
wrong_root="$artifact/wrong-root.bin"
KV9_BOOTSTRAP_TOKEN=wrong-root "$bin" root-create --output "$wrong_root" \
  --voters "1@$(service_ip 1):20160,9@$(service_ip 9):20160" >"$artifact/wrong-root-create.out"
k create configmap wrong-root -n "$namespace" --from-file="root.bin=$wrong_root" >/dev/null
write_deployment 9 wrong-root wrong-root
sleep 4
[ "$(status_value 9 bootstrap_state 2>/dev/null || true)" != Serving ] || {
  echo "FAIL: wrong root crossed the cluster trust boundary" >&2; exit 1;
}
wait_until "live root remains Serving during wrong-root contact" 15 agreed_leader >/dev/null
k delete deployment kv9-n9 -n "$namespace" --wait=true >/dev/null
k delete service kv9-n9 configmap wrong-root -n "$namespace" --ignore-not-found >/dev/null

# Pod kill must replace the exact selected member without losing durable identity.
echo "Stage: Pod kill and replacement"
leader="$(wait_agreed_leader "pre-Pod-kill agreement" 15)"
old_uid="$(pod_uid "$leader")"
k apply -f - >/dev/null <<YAML
apiVersion: chaos-mesh.org/v1alpha1
kind: PodChaos
metadata:
  name: kill-member
  namespace: $namespace
spec:
  action: pod-kill
  mode: one
  selector:
    namespaces: ["$namespace"]
    labelSelectors:
      app: kv9
      kv9-node: "$leader"
YAML
replacement_ready() {
  local uid
  uid="$(pod_uid "$leader" 2>/dev/null || true)"
  [ -n "$uid" ] && [ "$uid" != "$old_uid" ] && node_serving "$leader"
}
wait_until "PodChaos replacement with the same store identity" 45 replacement_ready
k delete podchaos kill-member -n "$namespace" --ignore-not-found --wait=true >/dev/null
leader="$(wait_agreed_leader "cluster convergence after Pod kill" 30)"

# Pod failure holds the leader down long enough to force a majority failover.
echo "Stage: sustained Pod failure and failover"
k apply -f - >/dev/null <<YAML
apiVersion: chaos-mesh.org/v1alpha1
kind: PodChaos
metadata:
  name: fail-leader
  namespace: $namespace
spec:
  action: pod-failure
  mode: one
  duration: 12s
  selector:
    namespaces: ["$namespace"]
    labelSelectors:
      app: kv9
      kv9-node: "$leader"
YAML
wait_injected podchaos fail-leader
new_leader="$(wait_majority_leader "surviving majority elects after Pod failure" 20 "$leader")"
client "$new_leader" raw-put --addr "$(service_ip "$new_leader"):20160" --keyspace "$keyspace" \
  --key-hex 706f646661696c --value-hex 7632 >"$artifact/pod-failure-put.out"
k delete podchaos fail-leader -n "$namespace" --ignore-not-found --wait=true >/dev/null
leader="$(wait_agreed_leader "failed leader recovers and catches up" 35)"

# A two-way partition must fence the isolated old leader while the majority writes.
echo "Stage: two-way leader partition and fencing"
apply_partition "$leader"
wait_injected networkchaos isolate-leader
new_leader="$(wait_majority_leader "majority leader under NetworkChaos partition" 20 "$leader")"
old_pod="$(pod_for "$leader")"
if k exec -n "$namespace" "$old_pod" -- env KV9_CLIENT_TOKEN="$client_token" \
  timeout 5 /usr/local/bin/kv9 client raw-put --addr 127.0.0.1:20160 --keyspace "$keyspace" \
  --key-hex 69736f6c61746564 --value-hex 626164 >"$artifact/isolated-write.out" 2>&1; then
  echo "FAIL: isolated old leader acknowledged a mutation" >&2
  exit 1
fi
client "$new_leader" raw-put --addr "$(service_ip "$new_leader"):20160" --keyspace "$keyspace" \
  --key-hex 706172746974696f6e --value-hex 7633 >"$artifact/partition-put.out"
k delete networkchaos isolate-leader -n "$namespace" --ignore-not-found --wait=true >/dev/null
leader="$(wait_agreed_leader "partition healing and catch-up" 35)"

# Delay one follower in both directions; quorum writes must remain available.
echo "Stage: follower latency and recovery"
follower=$(( leader == 1 ? 2 : 1 ))
apply_delay "$follower"
wait_injected networkchaos delay-follower
client "$leader" raw-put --addr "$(service_ip "$leader"):20160" --keyspace "$keyspace" \
  --key-hex 64656c6179 --value-hex 7634 >"$artifact/delay-put.out"
k delete networkchaos delay-follower -n "$namespace" --ignore-not-found --wait=true >/dev/null
leader="$(wait_agreed_leader "delayed follower recovery" 30)"

# Container kill exercises kubelet restart (distinct from deleting the Pod).
echo "Stage: container kill and durable restart"
follower=$(( leader == 3 ? 2 : 3 ))
before_restarts="$(restart_count "$follower")"
k apply -f - >/dev/null <<YAML
apiVersion: chaos-mesh.org/v1alpha1
kind: PodChaos
metadata:
  name: kill-container
  namespace: $namespace
spec:
  action: container-kill
  mode: one
  containerNames: ["kv9"]
  selector:
    namespaces: ["$namespace"]
    labelSelectors:
      app: kv9
      kv9-node: "$follower"
YAML
container_restarted() {
  local after
  after="$(restart_count "$follower" 2>/dev/null || true)"
  [[ "$after" =~ ^[0-9]+$ ]] && (( after > before_restarts )) && node_serving "$follower"
}
wait_until "container restart and durable catch-up" 40 container_restarted
k delete podchaos kill-container -n "$namespace" --ignore-not-found --wait=true >/dev/null
leader="$(wait_agreed_leader "final three-node agreement" 30)"

final_get="$(client "$leader" raw-get --addr "$(service_ip "$leader"):20160" \
  --keyspace "$keyspace" --key-hex 706172746974696f6e)"
grep -q '^value_hex=7633$' <<<"$final_get" || {
  echo "FAIL: replicated value was not readable after the fault matrix" >&2; exit 1;
}

echo "PASS: Chaos Mesh root boundary, Pod kill/failure, partition, delay, and container recovery"
