# Local acceptance helper. Preparation outputs live outside each data directory
# so another root fixture can refer to a running overlapping voter's public id.
prepare_root_stores() {
  local executable="$1" base="$2" voters="$3" item node output identity result=""
  local -a members
  IFS=, read -r -a members <<< "$voters"
  for item in "${members[@]}"; do
    node="${item%%@*}"
    [[ "$node" =~ ^[1-9][0-9]*$ ]] || return 1
    output="$base/n$node.prepare"
    if [[ ! -f "$output" ]]; then
      "$executable" store-prepare --node-id "$node" --data-dir "$base/n$node" > "$output"
    fi
    identity="$(awk -v expected="$node" '{
      for(i=1;i<=NF;i++) {split($i,pair,"="); fields[pair[1]]=pair[2]}
      if(fields["store_prepared"]=="true" && fields["node_id"]==expected) print fields["store_incarnation"]
    }' "$output")"
    [[ "$identity" =~ ^[0-9a-f]{32}$ ]] || return 1
    result="${result:+$result,}$node=$identity"
  done
  printf '%s\n' "$result"
}

# Select the executable Cargo actually built, including CARGO_TARGET_DIR and
# target/profile configuration. A stale ./target/debug binary is not evidence
# that the requested feature (for example partition-testing) was exercised.
build_fixture_binary() {
  local repository="$1" transcript="$2"; shift 2
  cargo build --locked --manifest-path "$repository/Cargo.toml" --bin kv9 \
    --message-format=json-render-diagnostics "$@" >"$transcript" || return
  python3 - "$transcript" <<'PYTHON'
import json, pathlib, sys
records = [json.loads(line) for line in pathlib.Path(sys.argv[1]).read_text().splitlines()]
executables = {r['executable'] for r in records if r.get('reason') == 'compiler-artifact'
               and r.get('target', {}).get('name') == 'kv9' and r.get('executable')}
if len(executables) != 1:
    raise SystemExit('FAIL: Cargo did not identify exactly one kv9 executable')
print(executables.pop())
PYTHON
}
