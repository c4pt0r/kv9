#!/usr/bin/env bash
set -euo pipefail
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$repo/scripts/membership-write.sh"
work="$(mktemp -d /tmp/kv9-membership-write-controls.XXXXXX)"
trap 'rm -rf "$work"' EXIT
leader_id() { echo 2; }
client() {
  if [ "$*" != 'create-keyspace --addr 127.0.0.1:23102 --name post-membership-failover --api-type raw' ]; then
    echo 'FAIL: retry changed the expected request arguments' >&2
    return 99
  fi
  local number=0
  if [ -f "$work/count" ]; then number="$(cat "$work/count")"; fi
  number=$((number + 1)); echo "$number" >"$work/count"
  if [ "$scenario" = transient ] && (( number == 1 )); then
    echo 'client request failed: raft error: CreateKeyspace RPC: code: FailedPrecondition, message: "not leader"' >&2
    return 1
  fi
  if [ "$scenario" = timeout ]; then echo 'request deadline exceeded' >&2; return 124; fi
  if [ "$scenario" = duplicate ]; then echo 'keyspace name already exists' >&2; return 1; fi
  printf 'keyspace_id=101\nproposed_term=6\nproposed_index=138\n'
}
scenario=transient
output="$(membership_write_after_failover "$work" 23100 1 2)"
test "$(cat "$work/count")" = 2
test "$output" = $'keyspace_id=101\nproposed_term=6\nproposed_index=138'
grep -Fxq exit_code=1 "$work/post-failover-1.request"
grep -Fxq exit_code=0 "$work/post-failover-2.request"
echo 'PASS: transient leadership rejection retries the same name and requires a real receipt'
for scenario in timeout duplicate; do
  rm "$work/count"
  if membership_write_after_failover "$work" 23100 1 2 >"$work/$scenario.out" 2>"$work/$scenario.err"; then
    echo "FAIL: $scenario was accepted" >&2; exit 1
  fi
  test "$(cat "$work/count")" = 1
  test ! -s "$work/$scenario.out"
  grep -Fxq 'FAIL: post-failover creation failed outside the observed leadership rejection' "$work/$scenario.err"
  echo "PASS: $scenario stops without a receipt or an extra attempt"
done
# An absent candidate cannot spin forever or emit a false receipt.
leader_id() { echo 1; }
if membership_write_after_failover "$work" 23100 1 0 >"$work/budget.out" 2>"$work/budget.err"; then exit 1; fi
test ! -s "$work/budget.out"
grep -Fxq 'FAIL: post-failover creation obtained no successful RPC receipt within its retry budget' "$work/budget.err"
echo 'PASS: exhausted routing budget emits no receipt'

# Advance this shell's clock deterministically while simulating status snapshots.
# The candidate function runs in a command substitution, so its observation
# counter is a file rather than a subshell-local variable.
sleep() { SECONDS=$((SECONDS + 1)); }
leader_id() {
  local number=0
  if [ -f "$work/selections" ]; then number="$(cat "$work/selections")"; fi
  number=$((number + 1)); echo "$number" >"$work/selections"
  if [ "$selection" = delayed ]; then
    if ((number == 2)); then echo 9; return 0; fi
    if ((number == 3)); then echo 3; return 0; fi
  fi
  return 1
}
selection=delayed
if ! output="$(membership_wait_for_leader "$work/selections.tsv" delayed 3)"; then
  echo 'FAIL: leader selection did not survive the observed election gap' >&2
  exit 1
fi
test "$output" = 3
test "$(cat "$work/selections")" = 3
test "$(wc -l <"$work/selections.tsv")" = 3
echo 'PASS: delayed leader selection ignores an invalid candidate and captures one successful observation'

selection=absent
rm "$work/selections"
if membership_wait_for_leader "$work/absent.tsv" absent 2 >"$work/absent.out" 2>"$work/absent.err"; then exit 1; fi
test ! -s "$work/absent.out"
test "$(cat "$work/selections")" = 2
grep -Fxq 'FAIL: no leader observed for absent within its selection budget' "$work/absent.err"
echo 'PASS: a persistent election gap exhausts one absolute budget without returning a candidate'

rm "$work/selections"
if membership_wait_for_leader "$work/zero.tsv" zero 0 >"$work/zero.out" 2>"$work/zero.err"; then exit 1; fi
test ! -s "$work/zero.out"
test ! -e "$work/selections"
test ! -e "$work/zero.tsv"
echo 'PASS: an exhausted selection budget does not query or fabricate a leader'
echo 'PASS: membership routing controls completed all seven cases'
