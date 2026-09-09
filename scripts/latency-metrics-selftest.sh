#!/usr/bin/env bash
# Exercise the sourced helper under wait_until's dynamic label scope.
set -euo pipefail
artifact="$(mktemp -d /tmp/kv9-latency-helper.XXXXXX)"
trap 'rm -rf "$artifact"' EXIT
namespace=fixture
io_pod=fixture
source "$(dirname "${BASH_SOURCE[0]}")/chaos-mesh-io.sh"
k() { printf '%s\n' '{"fixture":true}'; }
python3() {
  [ "$1" = scripts/check-latency-metrics.py ] && [ "$2" = "$artifact" ] &&
    [ "$3" = --io ] && [ "$4" = io-voter-2-errno-28 ]
}
wait_once() {
  local label="$1"; shift
  "$@"
}
wait_once 'fresh metrics include recovered durability' io_metrics_recovered io-voter-2-errno-28
[ -s "$artifact/io-voter-2-errno-28-recovered-metrics.json" ]
[ ! -e "$artifact/fresh metrics include recovered durability-recovered-metrics.json" ]
printf '%s\n' 'PASS: recovery metrics retain the explicit fault cell under a shadowed wait label'
