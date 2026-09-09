#!/usr/bin/env bash
# Preserve command failure and stream diagnostics before a hosted job deadline.
set -uo pipefail
if (( $# < 4 )); then
  echo 'usage: run-bounded-log.sh LOG DURATION KILL_GRACE COMMAND [ARG ...]' >&2
  exit 64
fi
log="$1"
duration="$2"
grace="$3"
shift 3
if [[ -e "$log" ]]; then
  echo 'FAIL: refusing to overwrite an existing command log' >&2
  exit 64
fi
timeout --signal=TERM --kill-after="$grace" "$duration" "$@" 2>&1 | tee -- "$log"
statuses=("${PIPESTATUS[@]}")
result="${statuses[0]}"
if (( result == 0 )); then result="${statuses[1]}"; fi
printf 'COMMAND_EXIT=%s LOG_EXIT=%s\n' "${statuses[0]}" "${statuses[1]}" >>"$log" || {
  if (( result == 0 )); then result=1; fi
}
exit "$result"
