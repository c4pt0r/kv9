#!/usr/bin/env bash
# Runs after the missing-log matrix, whose fixture supervisors are still live.
# PodChaos kills each original owner. A separately created and prepared PVC
# receives only the old root/store identity bundle, never its lifecycle or log.

replacement_bundle() {
  local stage="$1" mount="$2" pod="$3" file
  mkdir -p "$loss_scene/$stage"
  for file in kv9-root-descriptor kv9-store-identity kv9-store-lifecycle; do
    k exec -n "$namespace" "$pod" -- cat "$mount/$file" >"$loss_scene/$stage/$file"
  done
}

replacement_capture() {
  local stage="$1" folder="$loss_scene/$1"
  mkdir -p "$folder"
  date --iso-8601=ns >"$folder/at.txt"
  k get pod -n "$namespace" "$loss_pod" -o json >"$folder/pod.json"
  k get pvc -n "$namespace" "$replacement_claim" -o json >"$folder/pvc.json"
  replacement_bundle "$stage" /data "$loss_pod"
  k exec -n "$namespace" "$loss_pod" -- /bin/bash -c \
    'test ! -f /tmp/kv9-loss.pid && test ! -e /data/raft && test ! -e /data/status &&
     ! timeout 1 /bin/bash -c "exec 3<>/dev/tcp/127.0.0.1/20160"' \
    >"$folder/stopped-probe.log" 2>&1
  printf 'raft=absent\nstatus=absent\nlistener=absent\n' >"$folder/stopped.txt"
  if [[ "$stage" == rejected-* ]]; then
    k exec -n "$namespace" "$loss_pod" -- cat /tmp/kv9-loss.last-pid >"$folder/pid.txt"
    k exec -n "$namespace" "$loss_pod" -- cat /tmp/kv9-loss.exit >"$folder/exit.txt"
    k exec -n "$namespace" "$loss_pod" -- cat /tmp/kv9-loss.log >"$folder/process.log"
    [ "$(cat "$folder/exit.txt")" = 1 ] &&
      grep -Fq 'config error: prepared store identity does not match root/store identity' "$folder/process.log" &&
      ! grep -Fq panicked "$folder/process.log"
  fi
}

replacement_switch_claim() {
  local claim="$1" label="$2"
  k get deployment -n "$namespace" "kv9-n$victim" -o json >"$loss_scene/$label-deployment-before.json"
  python3 - "$loss_scene/$label-deployment-before.json" "$claim" >"$loss_scene/$label-patch.json" <<'PY'
import json, sys
d=json.load(open(sys.argv[1]))
volumes=d['spec']['template']['spec']['volumes']
indices=[i for i,v in enumerate(volumes) if v['name']=='data']
assert len(indices)==1 and 'persistentVolumeClaim' in volumes[indices[0]]
print(json.dumps([dict(op='replace', path=f'/spec/template/spec/volumes/{indices[0]}/persistentVolumeClaim/claimName', value=sys.argv[2])]))
PY
  k patch deployment "kv9-n$victim" -n "$namespace" --type=json \
    --patch-file "$loss_scene/$label-patch.json" >/dev/null
  k rollout status deployment/"kv9-n$victim" -n "$namespace" --timeout=60s >/dev/null
  loss_pod="$(pod_for "$victim")"
}

run_store_replacement_matrix() {
  local victim label loss_pod loss_old_uid loss_scene loss_required_index attempt key_hex
  local replacement_claim maintenance node_name volume
  for victim in 1 2 3; do
    label="store-loss-voter-$victim-pvc-replacement"
    loss_scene="$artifact/$label"
    replacement_claim="kv9-replacement-n$victim"
    maintenance="kv9-replacement-prepare-$victim"
    mkdir -p "$loss_scene"
    echo "Stage: independently prepared replacement PVC after PodChaos kills voter $victim"
    loss_pod="$(pod_for "$victim")"
    wait_until "original voter serves before PVC replacement" 40 loss_serving
    wait_agreed_leader "agreement before PVC replacement" 30 >/dev/null
    key_hex="7076632d3$victim"
    chaos_probe "$label-before-put" 0 raw-put --keyspace "$keyspace" \
      --key-hex "$key_hex" --value-hex 6265666f7265 >"$artifact/$label-before-put.out"
    loss_capture before
    replacement_bundle before /data "$loss_pod"
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
    wait_until "PodChaos replaces the original owner before PVC replacement" 60 loss_replaced
    wait_injected podchaos store-loss-kill
    k get podchaos store-loss-kill -n "$namespace" -o json >"$loss_scene/podchaos.json"
    loss_pod="$(pod_for "$victim")"
    wait_until "original owner releases its store before PVC replacement" 30 loss_owner_released
    date --iso-8601=ns >"$loss_scene/owner-release-at.txt"
    loss_capture held
    replacement_bundle held /data "$loss_pod"
    node_name="$(k get pod -n "$namespace" "$loss_pod" -o jsonpath='{.spec.nodeName}')"
    # Create, rather than apply, so an earlier PVC cannot satisfy preparation.
    k create -f - >/dev/null <<YAML
apiVersion: v1
kind: PersistentVolumeClaim
metadata:
  name: $replacement_claim
  namespace: $namespace
spec:
  accessModes: ["ReadWriteOnce"]
  resources:
    requests:
      storage: 128Mi
---
apiVersion: v1
kind: Pod
metadata:
  name: $maintenance
  namespace: $namespace
  labels:
    app: kv9-provisioning
spec:
  restartPolicy: Never
  terminationGracePeriodSeconds: 0
  nodeSelector:
    kubernetes.io/hostname: $node_name
  containers:
    - name: prepare
      image: $image
      imagePullPolicy: IfNotPresent
      command: ["sleep", "3600"]
      volumeMounts:
        - name: original
          mountPath: /old
          readOnly: true
        - name: replacement
          mountPath: /new
  volumes:
    - name: original
      persistentVolumeClaim:
        claimName: kv9-data-n$victim
    - name: replacement
      persistentVolumeClaim:
        claimName: $replacement_claim
YAML
    k wait -n "$namespace" --for=condition=Ready "pod/$maintenance" --timeout=60s >/dev/null
    k get pod -n "$namespace" "$maintenance" -o json >"$loss_scene/preparation-pod.json"
    k exec -n "$namespace" "$maintenance" -- /usr/local/bin/kv9 store-prepare \
      --node-id "$victim" --data-dir /new >"$loss_scene/replacement.prepare"
    k exec -n "$namespace" "$maintenance" -- cat /new/kv9-store-lifecycle >"$loss_scene/prepared.lifecycle"
    k get pvc -n "$namespace" "$replacement_claim" -o json >"$loss_scene/prepared-pvc.json"
    for attempt in original replacement; do
      volume="$(k get pvc -n "$namespace" "$([ "$attempt" = original ] && echo "kv9-data-n$victim" || echo "$replacement_claim")" -o jsonpath='{.spec.volumeName}')"
      k get pv "$volume" -o json >"$loss_scene/$attempt-pv.json"
    done
    # A valid old bootstrap credential does not grant the new disk permission
    # to take the old incarnation. Record the actual CLI refusal and side effects.
    k exec -n "$namespace" "$maintenance" -- /bin/bash -c \
      'KV9_BOOTSTRAP_TOKEN="$1" /usr/local/bin/kv9 init --root /old/kv9-root-descriptor --node-id "$2" --data-dir /new > /tmp/init.out 2> /tmp/init.err; echo $? > /tmp/init.exit' \
      init "$bootstrap_token" "$victim"
    for attempt in out err exit; do
      k exec -n "$namespace" "$maintenance" -- cat "/tmp/init.$attempt" >"$loss_scene/init.$attempt"
    done
    [ "$(cat "$loss_scene/init.exit")" = 1 ] &&
      grep -Fq 'config error: prepared store identity does not match root/store identity' "$loss_scene/init.err"
    k exec -n "$namespace" "$maintenance" -- /bin/bash -c \
      'test ! -e /new/kv9-root-descriptor && test ! -e /new/kv9-store-identity && test ! -e /new/raft && test ! -e /new/status'
    echo refused-without-bundle-or-raft >"$loss_scene/init-side-effects.txt"
    # Deliberately copy only the previous bundle, leaving the new Prepared
    # lifecycle untouched. The following starts exercise runtime verification.
    k exec -n "$namespace" "$maintenance" -- /bin/bash -c \
      'cp /old/kv9-root-descriptor /old/kv9-store-identity /new/ && touch /new/kv9-loss.hold'
    replacement_bundle forged /new "$maintenance"
    k delete pod "$maintenance" -n "$namespace" --wait=true >/dev/null
    replacement_switch_claim "$replacement_claim" replace
    replacement_capture replacement-held
    k exec -n "$namespace" "$loss_pod" -- rm /data/kv9-loss.hold
    for attempt in 1 2; do
      if (( attempt == 2 )); then
        k exec -n "$namespace" "$loss_pod" -- /bin/bash -c \
          'rm -f /tmp/kv9-loss.exit; touch /tmp/kv9-loss.start'
      fi
      wait_until "replacement PVC refuses actual startup attempt $attempt" 20 loss_stopped
      replacement_capture "rejected-$attempt"
    done
    wait_majority_leader "majority serves with an unauthorized replacement PVC" 30 "$victim" >/dev/null
    history_phase "$label"
    replacement_capture rejected-during-history
    chaos_probe "$label-majority-put" "$victim" raw-put --keyspace "$keyspace" \
      --key-hex "$key_hex" --value-hex 6166746572 >"$artifact/$label-majority-put.out"
    loss_required_index="$(awk -F= '$1 == "applied_index" {print $2}' "$artifact/$label-majority-put.out")"
    [[ "$loss_required_index" =~ ^[1-9][0-9]*$ ]]
    chaos_probe "$label-majority-get" "$victim" raw-get --keyspace "$keyspace" \
      --key-hex "$key_hex" >"$artifact/$label-majority-get.out"
    grep -Fxq value_hex=6166746572 "$artifact/$label-majority-get.out"
    history_set_phase healing
    k delete podchaos store-loss-kill -n "$namespace" --wait=true >/dev/null
    replacement_switch_claim "kv9-data-n$victim" restore
    wait_until "restored original PVC remains held before recovery" 20 loss_replaced
    k exec -n "$namespace" "$loss_pod" -- rm /data/kv9-loss.hold
    wait_until "original PVC recovers the original voter" 40 loss_serving
    wait_agreed_leader "agreement after original PVC restoration" 40 >/dev/null
    wait_until "original PVC applies the majority receipt" 30 loss_caught_up
    loss_capture healed
    replacement_bundle healed /data "$loss_pod"
    chaos_probe "$label-recovered-get" 0 raw-get --keyspace "$keyspace" \
      --key-hex "$key_hex" >"$artifact/$label-recovered-get.out"
    grep -Fxq value_hex=6166746572 "$artifact/$label-recovered-get.out"
    echo "PASS: voter $victim refused a newly prepared PVC carrying its old bundle and recovered the original PVC with majority service"
  done
}
