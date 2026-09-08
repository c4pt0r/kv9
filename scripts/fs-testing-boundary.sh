#!/usr/bin/env bash
# Check a separate dependency graph so workspace dev-feature unification cannot
# make an accidentally shipped filesystem injector look intentional.
set -euo pipefail
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
probe="$(mktemp -d /tmp/kv9-fs-boundary.XXXXXX)"
trap 'rm -rf "$probe"' EXIT
python3 - "$repo" "$probe" <<'PY'
import json
from pathlib import Path
import sys
repo, probe = map(Path, sys.argv[1:])
(probe / 'src').mkdir()
(probe / 'Cargo.toml').write_text('''[package]
name = "kv9-fs-boundary"
version = "0.0.0"
edition = "2021"
[workspace]
[dependencies]
kv9-common = { path = ''' + json.dumps(str(repo / 'crates/common')) + ''' }
''')
(probe / 'src/main.rs').write_text('use kv9_common::fs::OsFileSystem;\nfn main() { let _ = OsFileSystem; }\n')
PY
export CARGO_TARGET_DIR="$probe/target"
cargo check --manifest-path "$probe/Cargo.toml" > "$probe/positive.log" 2>&1 || {
  cat "$probe/positive.log"; exit 1;
}
cat > "$probe/src/main.rs" <<'EOF'
use kv9_common::fs::testing::ModelFs;
fn main() { let _ = ModelFs::default(); }
EOF
if cargo check --manifest-path "$probe/Cargo.toml" > "$probe/negative.log" 2>&1; then
  echo 'FAIL: filesystem fault injection is available to a default production dependency' >&2
  exit 1
fi
grep -Fq 'could not find `testing` in `fs`' "$probe/negative.log" || {
  cat "$probe/negative.log"; exit 1;
}
echo 'PASS: production filesystem compiles; default fault-injection import is rejected'
