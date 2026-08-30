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
  # A timeout path may collect here and cleanup collects once more. Keep both
  # instants: overwriting the first scene with the cleanup scene destroys the
  # evidence needed to tell "stopped" from "recovered just after timeout".
  # mktemp also stays unique when this function runs inside a command
  # substitution (and therefore a subshell).
  local scene
  scene="$(mktemp -d "$artifact/scene.XXXXXX")"
  date --iso-8601=ns >"$scene/collected-at.txt" 2>&1 || true
  k get all -n "$namespace" -o wide >"$scene/kubernetes.txt" 2>&1 || true
  k get pods,services,endpoints,endpointslices,deployments,persistentvolumeclaims \
    -n "$namespace" -o yaml >"$scene/topology.yaml" 2>&1 || true
  k get events -n "$namespace" --sort-by=.metadata.creationTimestamp \
    >"$scene/events.txt" 2>&1 || true
  k get podchaos,networkchaos -n "$namespace" -o yaml >"$scene/chaos.yaml" 2>&1 || true
  local pod
  while read -r pod; do
    [ -n "$pod" ] || continue
    k describe pod -n "$namespace" "$pod" >"$scene/$pod.describe" 2>&1 || true
    k logs -n "$namespace" "$pod" --all-containers >"$scene/$pod.log" 2>&1 || true
    k logs -n "$namespace" "$pod" --all-containers --previous \
      >"$scene/$pod.previous.log" 2>&1 || true
    k exec -n "$namespace" "$pod" -- cat /data/status \
      >"$scene/$pod.status" 2>&1 || true
    # /proc/net/tcp preserves SYN_SENT vs ESTABLISHED even though the minimal
    # image intentionally carries no ss/netstat package. This is the direct
    # discriminator for a peer worker stuck in connect/handshake.
    k exec -n "$namespace" "$pod" -- /bin/bash -c \
      'cat /proc/net/tcp; cat /proc/net/tcp6' \
      >"$scene/$pod.net-tcp" 2>&1 || true
  done < <(k get pods -n "$namespace" -o name 2>/dev/null | sed 's#pod/##')

  # Record the actual new-connection topology rather than treating a Chaos
  # resource's AllInjected condition as proof of which edges are open.
  local from to
  : >"$scene/tcp-matrix.txt"
  for from in 1 2 3; do
    for to in 1 2 3; do
      (( from == to )) && continue
      if tcp_probe "$from" "$to"; then
        printf 'n%s -> n%s reachable\n' "$from" "$to" >>"$scene/tcp-matrix.txt"
      else
        printf 'n%s -> n%s blocked-or-unavailable\n' "$from" "$to" \
          >>"$scene/tcp-matrix.txt"
      fi
    done
  done
  echo "FAIL: collected Chaos scene at $scene" >&2
}

cleanup() {
  local rc=$?
  trap - EXIT
  if k get namespace "$namespace" >/dev/null 2>&1; then
    if (( rc == 0 )); then
      k delete podchaos,networkchaos --all -n "$namespace" --ignore-not-found \
        --wait=true >/dev/null 2>&1 || true
      k delete namespace "$namespace" --wait=true >/dev/null 2>&1 || true
    else
      collect_scene
      echo "FAIL: preserving live namespace $namespace for inspection" >&2
    fi
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
  local state fatal
  state="$(status_value "$1" bootstrap_state 2>/dev/null)" || return 1
  fatal="$(status_value "$1" fatal 2>/dev/null)" || return 1
  [ "$state" = Serving ] && [ -z "$fatal" ]
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

tcp_probe() {
  local from="$1" to="$2" pod addr
  pod="$(pod_for "$from")" || return 1
  addr="$(service_ip "$to")"
  k exec -n "$namespace" "$pod" -- timeout 2 /bin/bash -c \
    "exec 3<>/dev/tcp/$addr/20160" >/dev/null 2>&1
}

tcp_probe_millis() {
  local from="$1" to="$2" pod addr
  pod="$(pod_for "$from")" || return 1
  addr="$(service_ip "$to")"
  k exec -n "$namespace" "$pod" -- timeout 3 /bin/bash -c \
    'start=$(date +%s%N); exec 3<>/dev/tcp/'"$addr"'/20160; end=$(date +%s%N); echo $(((end-start)/1000000))' \
    2>/dev/null
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

write_pvc() {
  local node="$1"
  k apply -f - >/dev/null <<YAML
apiVersion: v1
kind: PersistentVolumeClaim
metadata:
  name: kv9-data-n$node
  namespace: $namespace
spec:
  accessModes: ["ReadWriteOnce"]
  resources:
    requests:
      storage: 128Mi
YAML
}

write_handshake_blackhole() {
  k apply -f - >/dev/null <<YAML
apiVersion: v1
kind: Pod
metadata:
  name: handshake-blackhole
  namespace: $namespace
  labels:
    app: kv9
    kv9-node: "2"
    kv9-fixture: handshake-blackhole
spec:
  terminationGracePeriodSeconds: 0
  containers:
    - name: blackhole
      image: $image
      imagePullPolicy: IfNotPresent
      command: ["/usr/bin/perl", "-MIO::Socket::INET", "-e"]
      args:
        - |
          \$| = 1;
          my \$listener = IO::Socket::INET->new(
            LocalAddr => "0.0.0.0", LocalPort => 20160, Proto => "tcp",
            Listen => 32, Reuse => 1
          ) or die "listen: \$!";
          print "READY\\n";
          my @held;
          while (my \$connection = \$listener->accept()) {
            push @held, \$connection;
            print "ACCEPT " . \$connection->peerhost() . " " . scalar(@held) . "\\n";
          }
YAML
}

blackhole_has_both_voters() {
  local pod1 pod3 ip1 ip3 logs
  pod1="$(pod_for 1)" || return 1
  pod3="$(pod_for 3)" || return 1
  ip1="$(k get pod -n "$namespace" "$pod1" -o jsonpath='{.status.podIP}')"
  ip3="$(k get pod -n "$namespace" "$pod3" -o jsonpath='{.status.podIP}')"
  logs="$(k logs -n "$namespace" handshake-blackhole 2>/dev/null)"
  grep -Fq -- "ACCEPT $ip1 " <<<"$logs" &&
    grep -Fq -- "ACCEPT $ip3 " <<<"$logs"
}

blackhole_detached_from_service() {
  local blackhole_ip="$1" endpoints
  endpoints="$(k get endpoints -n "$namespace" kv9-n2 \
    -o jsonpath='{.subsets[*].addresses[*].ip}' 2>/dev/null || true)"
  ! grep -Fqw -- "$blackhole_ip" <<<"$endpoints"
}

real_node_two_endpoint_ready() {
  local names
  names="$(k get endpoints -n "$namespace" kv9-n2 \
    -o jsonpath='{.subsets[*].addresses[*].targetRef.name}' 2>/dev/null || true)"
  [ -n "$names" ] && ! grep -Fqw -- handshake-blackhole <<<"$names"
}

wrong_root_was_rejected() {
  local state observation
  pod_for 9 >/dev/null || return 1
  state="$(status_value 9 bootstrap_state 2>/dev/null)" || return 1
  observation="$(status_value 9 discovery_seed_1 2>/dev/null)" || return 1
  [ "$state" != Serving ] &&
    [[ "$observation" =~ ,attempts=[1-9][0-9]* ]] &&
    [[ "$observation" =~ ,errors=[1-9][0-9]* ]] &&
    [[ "$observation" == *"last=error:"* ]] &&
    [[ "$observation" == *"root identity mismatch"* ]]
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
          persistentVolumeClaim:
            claimName: kv9-data-n$node
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

for node in 1 2 3 9; do
  write_service "$node"
  write_pvc "$node"
done
voters="1@$(service_ip 1):20160,2@$(service_ip 2):20160,3@$(service_ip 3):20160"
KV9_BOOTSTRAP_TOKEN="$bootstrap_token" "$bin" root-create --output "$root" --voters "$voters" \
  >"$artifact/root-create.out"
k create configmap kv9-root -n "$namespace" --from-file="root.bin=$root" >/dev/null

# Deterministically occupy n2's stable Service with a TCP endpoint that accepts
# connections but never completes HTTP/2. Once both existing voters have a
# peer worker stuck in that handshake, remove the fixture from the Service
# selector WITHOUT killing it, then add the real n2 endpoint. The old TCP
# connections remain alive against the blackhole. A worker without a total
# connect+handshake budget will never notice the replacement and strands the
# legitimate voter at committed=0; a bounded worker reconnects and catches up.
write_handshake_blackhole
k wait -n "$namespace" --for=condition=Ready pod/handshake-blackhole --timeout=20s >/dev/null
blackhole_ip="$(k get pod -n "$namespace" handshake-blackhole \
  -o jsonpath='{.status.podIP}')"
[[ "$blackhole_ip" =~ ^[0-9a-fA-F:.]+$ ]] || {
  echo "FAIL: handshake blackhole has no Pod IP" >&2
  exit 1
}

# Deliberately let a valid two-voter quorum initialize before the third
# declared voter starts, while both voters establish doomed n2 streams.
write_deployment 1
write_deployment 3
wait_majority_leader "two-voter quorum initializes before late voter" 35 2 >/dev/null
wait_until "both voters enter the n2 handshake blackhole" 15 \
  blackhole_has_both_voters
k label pod -n "$namespace" handshake-blackhole kv9-node=blackhole --overwrite >/dev/null
wait_until "blackhole leaves the n2 Service endpoints" 15 \
  blackhole_detached_from_service "$blackhole_ip"
write_deployment 2
wait_until "real n2 endpoint replaces the handshake blackhole" 20 \
  real_node_two_endpoint_ready

echo "Stage: bootstrap and baseline mutation"
leader="$(wait_agreed_leader "root-certified cluster Serving" 45)"
k delete pod handshake-blackhole -n "$namespace" --wait=true >/dev/null
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
wait_until "wrong-root node runs and records the exact root-identity rejection" 15 \
  wrong_root_was_rejected
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
survivor_a=$(( leader == 1 ? 2 : 1 ))
survivor_b=$(( 6 - leader - survivor_a ))
tcp_probe "$survivor_a" "$survivor_b" && tcp_probe "$survivor_b" "$survivor_a" || {
  echo "FAIL: Chaos topology cut the surviving majority edge" >&2
  exit 1
}
if tcp_probe "$survivor_a" "$leader"; then
  echo "FAIL: injected partition still permits a new TCP connection to the isolated leader" >&2
  exit 1
fi
new_leader="$(wait_majority_leader "majority leader under NetworkChaos partition" 20 "$leader")"
old_pod="$(pod_for "$leader")"
isolated_uid="$(pod_uid "$leader")"
k exec -n "$namespace" "$old_pod" -- timeout 2 /bin/bash -c \
  'exec 3<>/dev/tcp/127.0.0.1/20160' >/dev/null 2>&1 || {
  echo "FAIL: isolated old leader was not alive and listening before the fencing probe" >&2
  exit 1
}
set +e
k exec -n "$namespace" "$old_pod" -- env KV9_CLIENT_TOKEN="$client_token" \
  timeout 5 /usr/local/bin/kv9 client raw-put --addr 127.0.0.1:20160 --keyspace "$keyspace" \
  --key-hex 69736f6c61746564 --value-hex 626164 >"$artifact/isolated-write.out" 2>&1
isolated_write_rc=$?
set -e
if (( isolated_write_rc == 0 )); then
  echo "FAIL: isolated old leader acknowledged a mutation" >&2
  exit 1
fi
[ "$(pod_uid "$leader")" = "$isolated_uid" ] && [ "$(pod_for "$leader")" = "$old_pod" ] &&
  k exec -n "$namespace" "$old_pod" -- timeout 2 /bin/bash -c \
    'exec 3<>/dev/tcp/127.0.0.1/20160' >/dev/null 2>&1 || {
  echo "FAIL: old-leader write failed because the Pod/service died, not because Raft fenced it" >&2
  exit 1
}
if (( isolated_write_rc != 124 )) &&
  ! grep -Eiq 'not (the )?leader|deadline|unconfirmed|not reached|timed out' \
    "$artifact/isolated-write.out"; then
  echo "FAIL: old-leader write failed without a Raft fencing/deadline reason (rc=$isolated_write_rc)" >&2
  exit 1
fi
client "$new_leader" raw-put --addr "$(service_ip "$new_leader"):20160" --keyspace "$keyspace" \
  --key-hex 706172746974696f6e --value-hex 7633 >"$artifact/partition-put.out"
k delete networkchaos isolate-leader -n "$namespace" --ignore-not-found --wait=true >/dev/null
leader="$(wait_agreed_leader "partition healing and catch-up" 35)"
isolated_get="$(client "$leader" raw-get --addr "$(service_ip "$leader"):20160" \
  --keyspace "$keyspace" --key-hex 69736f6c61746564)"
[ "$isolated_get" = "found=false" ] || {
  echo "FAIL: isolated old-leader mutation entered the cluster: $isolated_get" >&2
  exit 1
}

# Delay one follower in both directions; quorum writes must remain available.
echo "Stage: follower latency and recovery"
follower=$(( leader == 1 ? 2 : 1 ))
baseline_delay_ms="$(tcp_probe_millis "$leader" "$follower")"
[[ "$baseline_delay_ms" =~ ^[0-9]+$ ]] || {
  echo "FAIL: baseline TCP latency probe returned '$baseline_delay_ms'" >&2
  exit 1
}
apply_delay "$follower"
wait_injected networkchaos delay-follower
injected_delay_ms="$(tcp_probe_millis "$leader" "$follower")"
[[ "$injected_delay_ms" =~ ^[0-9]+$ ]] || {
  echo "FAIL: injected TCP latency probe returned '$injected_delay_ms'" >&2
  exit 1
}
if (( injected_delay_ms < 100 || injected_delay_ms < baseline_delay_ms + 100 )); then
  echo "FAIL: NetworkChaos delay was not observed (baseline=${baseline_delay_ms}ms injected=${injected_delay_ms}ms)" >&2
  exit 1
fi
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
