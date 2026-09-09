# First-formation crash acceptance. Sourced by chaos-mesh-e2e.sh.
# The hold file belongs only to this fixture's shell wrapper. The database
# binary has no startup pause flag, and every preparation uses the real PVC.
formation_capture() {
  local phase="$1" node pod
  mkdir -p "$artifact/formation/$phase"
  date --iso-8601=ns >"$artifact/formation/$phase/at.txt"
  for node in 1 2 3; do
    pod="$(pod_for "$node")"
    k get pod -n "$namespace" "$pod" -o json >"$artifact/formation/$phase/n$node.pod.json"
    k exec -n "$namespace" "$pod" -- cat /data/kv9-store-lifecycle \
      >"$artifact/formation/$phase/n$node.lifecycle"
    if [ "$phase" != replaced ]; then
      k exec -n "$namespace" "$pod" -- cat /data/status >"$artifact/formation/$phase/n$node.status"
      k exec -n "$namespace" "$pod" -- timeout 2 /bin/bash -c \
        'exec 3<>/dev/tcp/127.0.0.1/20160'
      echo reachable >"$artifact/formation/$phase/n$node.local-probe"
    else
      k exec -n "$namespace" "$pod" -- /bin/bash -c \
        'test -f /data/preformation-hold && ! timeout 1 /bin/bash -c "exec 3<>/dev/tcp/127.0.0.1/20160"' \
        >"$artifact/formation/$phase/n$node.hold.log" 2>&1
      echo held-without-listener >"$artifact/formation/$phase/n$node.local-probe"
    fi
  done
}

formation_uncommitted() {
  local node pod
  for node in 1 2 3; do
    [ "$(status_value "$node" bootstrap_state)" = Discovering ] || return 1
    [ "$(status_value "$node" raft_owner_started)" = true ] || return 1
    [ "$(status_value "$node" raft_receive_authorized)" = true ] || return 1
    [ "$(status_value "$node" raft_committed)" = 0 ] || return 1
    [ -z "$(status_value "$node" cluster_id)" ] || return 1
    [ -z "$(status_value "$node" fatal)" ] || return 1
    pod="$(pod_for "$node")"
    k exec -n "$namespace" "$pod" -- timeout 2 /bin/bash -c \
      'exec 3<>/dev/tcp/127.0.0.1/20160' >/dev/null 2>&1 || return 1
  done
}

formation_replaced() {
  local node pod old uid
  for node in 1 2 3; do
    old="$(cat "$artifact/formation/n$node.old-uid")"
    uid="$(pod_uid "$node")" || return 1
    [ -n "$uid" ] && [ "$uid" != "$old" ] || return 1
    pod="$(pod_for "$node")"
    # Check the replacement process, never the status file left on its PVC.
    k exec -n "$namespace" "$pod" -- /bin/bash -c \
      'test -f /data/preformation-hold && ! timeout 1 /bin/bash -c "exec 3<>/dev/tcp/127.0.0.1/20160"' \
      >/dev/null 2>&1 || return 1
  done
}

run_formation_crash() {
  local node pod from to
  echo "Stage: all original voters crash before first catalog formation"
  mkdir -p "$artifact/formation"
  for node in 1 2 3; do
    write_deployment "$node"
    k rollout status -n "$namespace" "deployment/kv9-n$node" --timeout=60s >/dev/null
  done
  k apply -f - >/dev/null <<YAML
apiVersion: chaos-mesh.org/v1alpha1
kind: NetworkChaos
metadata:
  name: preformation-partition
  namespace: $namespace
spec:
  action: partition
  mode: all
  selector:
    namespaces: ["$namespace"]
    labelSelectors:
      app: kv9
  direction: both
  target:
    mode: all
    selector:
      namespaces: ["$namespace"]
      labelSelectors:
        app: kv9
YAML
  wait_injected networkchaos preformation-partition
  k get networkchaos preformation-partition -n "$namespace" -o json \
    >"$artifact/formation/networkchaos.json"
  for node in 1 2 3; do
    pod="$(pod_for "$node")"
    k exec -n "$namespace" "$pod" -- rm /data/preformation-hold
  done
  wait_until "all original owners live with no committed catalog" 30 formation_uncommitted
  formation_capture isolated
  : >"$artifact/formation/tcp-matrix.txt"
  for from in 1 2 3; do
    for to in 1 2 3; do
      (( from == to )) && continue
      if tcp_probe "$from" "$to"; then
        echo "FAIL: preformation partition left n$from -> n$to reachable" >&2
        return 1
      fi
      printf 'n%s -> n%s blocked\n' "$from" "$to" >>"$artifact/formation/tcp-matrix.txt"
    done
  done
  formation_uncommitted
  for node in 1 2 3; do
    pod_uid "$node" >"$artifact/formation/n$node.old-uid"
    pod="$(pod_for "$node")"
    k exec -n "$namespace" "$pod" -- touch /data/preformation-hold
  done
  k apply -f - >/dev/null <<YAML
apiVersion: chaos-mesh.org/v1alpha1
kind: PodChaos
metadata:
  name: kill-before-formation
  namespace: $namespace
spec:
  action: pod-kill
  mode: all
  selector:
    namespaces: ["$namespace"]
    labelSelectors:
      app: kv9
YAML
  wait_until "PodChaos replaces every original owner before formation" 60 formation_replaced
  record_fault podchaos kill-before-formation
  k get podchaos kill-before-formation -n "$namespace" -o json \
    >"$artifact/formation/podchaos.json"
  formation_capture replaced
  k delete podchaos/kill-before-formation networkchaos/preformation-partition \
    -n "$namespace" --wait=true >/dev/null
  k delete deployment kv9-n1 kv9-n2 kv9-n3 -n "$namespace" --wait=true >/dev/null
  k wait -n "$namespace" --for=delete pod -l app=kv9 --timeout=45s >/dev/null
  # Maintenance Pods only release the fixture gate. They neither prepare nor
  # initialize nor delete data; the following normal deployment recovers it.
  for node in 1 2 3; do
    maintain_volume "$node" release-formation
  done
  echo "PASS: NetworkChaos held all voters before catalog commit and PodChaos replaced every owner"
}
