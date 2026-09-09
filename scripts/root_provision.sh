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
