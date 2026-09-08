#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --workspace
exec python3 scripts/minio-kv-e2e.py
