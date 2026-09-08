#!/usr/bin/env bash
# Create a new, disposable Kind cluster; never install into an ambient context.
set -euo pipefail
umask 077

kind_bin="${KIND:-kind}"
helm_bin="${HELM:-helm}"
kubectl_bin="${KUBECTL:-kubectl}"
cluster="${KV9_KIND_CLUSTER:?Set a unique kv9-chaos-ci-* cluster name}"
config="${KUBECONFIG:?Set an unused absolute kubeconfig path}"
[[ "$cluster" == kv9-chaos-ci-* && "$cluster" =~ ^[a-z0-9-]+$ ]] || {
  echo 'FAIL: expected a unique kv9-chaos-ci-* cluster name' >&2; exit 1;
}
[[ "$config" == /* && "$config" != *:* && ! -e "$config" ]] || {
  echo 'FAIL: KUBECONFIG must be one unused absolute path' >&2; exit 1;
}
clusters="$("$kind_bin" get clusters)"
if grep -Fxq "$cluster" <<< "$clusters"; then
  echo 'FAIL: refusing to reuse an existing cluster' >&2; exit 1
fi
"$kind_bin" create cluster --name "$cluster" --kubeconfig "$config" --wait 120s \
  --image kindest/node:v1.35.5@sha256:ce977ae6d65918d0b58a5f8b5e940429c2ce42fa3a5619ec2bbc60b949c0ac95
"$helm_bin" install chaos-mesh chaos-mesh --repo https://charts.chaos-mesh.org \
  --version 2.8.4 --namespace chaos-mesh --create-namespace \
  --kubeconfig "$config" --wait --timeout 5m \
  --set chaosDaemon.runtime=containerd \
  --set chaosDaemon.socketPath=/run/containerd/containerd.sock \
  --set controllerManager.enableFilterNamespace=true \
  --set controllerManager.replicaCount=1 \
  --set controllerManager.securityContext.runAsUser=10001 \
  --set controllerManager.securityContext.runAsGroup=10001 \
  --set controllerManager.securityContext.runAsNonRoot=true \
  --set dashboard.create=false
"$kubectl_bin" --kubeconfig "$config" get crd \
  podchaos.chaos-mesh.org networkchaos.chaos-mesh.org iochaos.chaos-mesh.org
# The chart does not give the controller a readiness probe. Helm can report
# success between process start and a controller crash. Exercise both admission
# webhooks with server-side dry run; this never creates an injection resource.
deadline=$((SECONDS + 120))
ready=0
while (( SECONDS < deadline )); do
  if "$kubectl_bin" --kubeconfig "$config" --request-timeout=10s \
    create --dry-run=server -f - >/dev/null <<'YAML'
apiVersion: chaos-mesh.org/v1alpha1
kind: PodChaos
metadata:
  name: kv9-webhook-readiness
  namespace: chaos-mesh
spec:
  action: pod-kill
  mode: one
  selector:
    namespaces: [chaos-mesh]
    labelSelectors:
      app: kv9-readiness-probe-does-not-exist
YAML
  then
    ready=1
    break
  fi
  sleep 1
done
(( ready == 1 )) || { echo 'FAIL: Chaos Mesh webhooks are unavailable' >&2; exit 1; }
echo 'PASS: isolated Kind cluster and Chaos Mesh are ready'
