# Root-trust fixture readiness is evidence from the current live child only.
# The caller owns `artifact` and the associative `node_pids` table.
node_serving() {
  local node="$1" expected_pid="${node_pids[$1]:-}"
  [[ -n "$expected_pid" ]] || return 1
  kill -0 "$expected_pid" 2>/dev/null || return 1
  # Parse one atomic status-file snapshot. An old Serving file from the
  # previous child is not evidence that its replacement has finished recovery.
  awk -F= -v expected="$expected_pid" '
    $1 == "pid" { pid=$2; pids++ }
    $1 == "bootstrap_state" { state=$2; states++ }
    $1 == "fatal" { fatal=substr($0, 7); fatals++ }
    END { exit !(pids == 1 && pid == expected && states == 1 && state == "Serving" && fatals == 1 && fatal == "") }
  ' "$artifact/n${node}/status" 2>/dev/null
}
