#!/usr/bin/env bash
# Regenerate kv9/_gen from the server's proto. Run from any directory:
#   python -m pip install grpcio-tools && sdk/python/gen.sh
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
proto_dir="$here/../../crates/server/proto"
out="$here/kv9/_gen"
mkdir -p "$out"
"${PYTHON:-python3}" -m grpc_tools.protoc -I "$proto_dir" \
  --python_out="$out" --pyi_out="$out" --grpc_python_out="$out" "$proto_dir/kv9.proto"
# grpc_tools emits an absolute import; the stubs live inside the kv9 package.
sed -i 's/^import kv9_pb2 as kv9__pb2$/from . import kv9_pb2 as kv9__pb2/' "$out/kv9_pb2_grpc.py"
cat >"$out/__init__.py" <<'EOF'
"""Generated gRPC stubs for kv9.v1 (see gen.sh). Do not edit by hand."""
EOF
echo "generated into $out"
