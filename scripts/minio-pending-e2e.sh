#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
# The pause gates are compiled out of the default binary.
cargo build --features checkpoint-testing
export KV9_TEST_PENDING_CRASHES=1
exec python3 scripts/minio-kv-e2e.py
