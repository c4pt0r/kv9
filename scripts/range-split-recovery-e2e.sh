#!/usr/bin/env bash
# Split recovery under faults (docs/RANGE-SPLIT.md §13).
set -euo pipefail
repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
artifact_dir="${KV9_RECOVER_E2E_DIR:-$(mktemp -d /tmp/kv9-split-recovery.XXXXXX)}"
base_port="${KV9_BASE_PORT:-$((28800 + ($$ % 400)))}"
cd "$repo_dir"
cargo build --locked -p kv9 -p kv9-admin -p kv9-repl --bin kv9 --bin kv9-admin --bin kv9-repl >"$artifact_dir/build.log" 2>&1
python3 scripts/range-split-recovery-e2e.py --bin target/debug/kv9 --admin-bin target/debug/kv9-admin \
  --repl-bin target/debug/kv9-repl --output "$artifact_dir/run" --base-port "$base_port"
echo "Artifacts: $artifact_dir"
