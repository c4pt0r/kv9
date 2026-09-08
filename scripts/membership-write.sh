#!/usr/bin/env bash
# Shared by the real membership E2E and its bounded retry controls.
# `leader_id` supplies a routing candidate; only a successful RPC receipt proves
# that a write completed. Failed attempts remain recorded, not counted as writes.
membership_write_after_failover() {
  local artifact="$1" base="$2" failed="$3" budget="${4:-20}"
  local deadline=$((SECONDS + budget)) attempt=0 candidate rc output errors
  while (( SECONDS < deadline )); do
    candidate="$(leader_id 2>/dev/null || true)"
    if [[ ! "$candidate" =~ ^[1-5]$ ]] || [ "$candidate" = "$failed" ]; then
      sleep 0.05
      continue
    fi
    attempt=$((attempt + 1))
    output="$artifact/post-failover-$attempt.stdout"
    errors="$artifact/post-failover-$attempt.stderr"
    printf 'candidate=%s\ninvoked_at=%s\n' "$candidate" "$(date --iso-8601=ns)" \
      >"$artifact/post-failover-$attempt.request"
    if client create-keyspace --addr "127.0.0.1:$((base + candidate))" \
      --name post-membership-failover --api-type raw >"$output" 2>"$errors"; then
      printf 'exit_code=0\nreturned_at=%s\n' "$(date --iso-8601=ns)" >>"$artifact/post-failover-$attempt.request"
      cat "$output"
      return 0
    else
      rc=$?
    fi
    printf 'exit_code=%s\nreturned_at=%s\n' "$rc" "$(date --iso-8601=ns)" >>"$artifact/post-failover-$attempt.request"
    # This is the observed public CreateKeyspace leadership rejection. Retry
    # the SAME unique name, never fabricate a receipt or discard the attempt.
    # Generic timeouts/transport errors and duplicate-name errors fail here;
    # an ambiguous original success cannot become a second accepted creation.
    if (( rc != 1 )) || ! grep -Eq '^client request failed: raft error: CreateKeyspace RPC: code: .*message: "not leader"$' "$errors"; then
      cat "$errors" >&2
      echo 'FAIL: post-failover creation failed outside the observed leadership rejection' >&2
      return 1
    fi
    sleep 0.05
  done
  echo 'FAIL: post-failover creation obtained no successful RPC receipt within its retry budget' >&2
  return 1
}
