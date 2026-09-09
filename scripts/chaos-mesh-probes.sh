#!/usr/bin/env bash
# Positive fault probes share the existing exclusive-refusal routing contract.
chaos_probe() {
  local probe_label="$1" excluded="$2"; shift 2
  KV9_CLIENT_TOKEN="$client_token" python3 scripts/chaos_client.py \
    --kubectl "$kubectl_bin" --kubeconfig "$kubeconfig" --namespace "$namespace" \
    --addresses "1=$(service_ip 1):20160,2=$(service_ip 2):20160,3=$(service_ip 3):20160" \
    --exclude "$excluded" --evidence "$artifact/$probe_label-attempts.jsonl" -- "$@"
}
