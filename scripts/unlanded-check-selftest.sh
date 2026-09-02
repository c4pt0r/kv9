#!/usr/bin/env bash
# Self-tests for scripts/unlanded-check.sh -- see that file for the contract.
#   scripts/unlanded-check-selftest.sh "$(pwd)/scripts/unlanded-check.sh"
#
# Every run gets one unique outer directory holding the scratch repo, removed on
# every terminal path. Fixed shared paths under /tmp are how a previous harness
# let one run's leftovers become the next run's input.
set -uo pipefail
CHK="${1:?usage: $0 /path/to/unlanded-check.sh}"
OUT=$(mktemp -d /tmp/unl-run.XXXXXX)
trap 'rm -rf "$OUT"' EXIT INT TERM
R="$OUT/repo"; mkdir -p "$R"
pass=0; fail=0
check() { if [ "$2" = "$3" ]; then printf "  PASS  %-54s rc=%s\n" "$1" "$3"; pass=$((pass+1));
          else printf "  FAIL  %-54s expected rc=%s got rc=%s\n" "$1" "$2" "$3"; fail=$((fail+1)); fi; }

cd "$R" && git init -q . >/dev/null
mkdir -p src
printf 'pub fn a() {}\n' > src/a.rs
git add -A >/dev/null 2>&1; git -c user.email=t@t -c user.name=t commit -q -m base >/dev/null 2>&1

# U1 clean repo, no markers -> may close
"$CHK" --repo "$R" --task 27 >/dev/null 2>&1; check "U1 no markers -> may close" 0 $?

# U2 a marker for that task blocks closure. Clause 2 of the card.
# Assembled from fragments: a self-test that writes a LIVE marker into a file
# that itself lives in the scanned repo would block closure of that very task.
# Same rule as a sentinel value never appearing in the notes that document it.
M27="UNLANDED""(task #27)"
printf '// %s expires when raw reads are linearizable\n' "$M27" >> src/a.rs
git add -A >/dev/null 2>&1; git -c user.email=t@t -c user.name=t commit -q -m mark >/dev/null 2>&1
"$CHK" --repo "$R" --task 27 >/dev/null 2>&1; check "U2 marker present -> blocks closure" 2 $?

# U2b the report must name exact file:line, not just a count (clause 3).
out=$("$CHK" --repo "$R" --task 27 2>&1)
if printf '%s' "$out" | grep -q "src/a.rs"; then
  printf "  PASS  %-54s\n" "U2b report names the exact file"; pass=$((pass+1))
else printf "  FAIL  %-54s\n" "U2b report names the exact file"; fail=$((fail+1)); fi

# U3 markers are per-task: #27's marker must not block #28.
"$CHK" --repo "$R" --task 28 >/dev/null 2>&1; check "U3 another task's marker does not block" 0 $?

# U4 fixed-string, not regex: a marker for task 2 must not match task 27's text,
# and regex metacharacters in the marker must be taken literally.
M2="UNLANDED""(task #2)"
printf '// %s unrelated\n' "$M2" >> src/a.rs
git add -A >/dev/null 2>&1; git -c user.email=t@t -c user.name=t commit -q -m two >/dev/null 2>&1
"$CHK" --repo "$R" --task 2 >/dev/null 2>&1; check "U4 task 2 matches only its own marker" 2 $?

# U5 removing the marker unblocks -- the criterion is zero, and it is reachable.
# `git rm` of the last file in src/ also removes the directory, so recreate it:
# without the mkdir this cell passed because the file did not exist at all, which
# is a pass for the wrong reason.
git rm -q src/a.rs >/dev/null 2>&1
mkdir -p src && printf 'pub fn a() {}\n' > src/a.rs
git add -A >/dev/null 2>&1; git -c user.email=t@t -c user.name=t commit -q -m clear >/dev/null 2>&1
if [ -f src/a.rs ] && ! grep -q UNLANDED src/a.rs; then
  printf "  PASS  %-54s\n" "U5a fixture: file exists and carries no marker"; pass=$((pass+1))
else printf "  FAIL  %-54s\n" "U5a fixture: file exists and carries no marker"; fail=$((fail+1)); fi
"$CHK" --repo "$R" --task 27 >/dev/null 2>&1; check "U5 marker removed -> may close again" 0 $?

# U6 an older head still carrying the marker must still block, so the check is
# anchored to the head it is given rather than to the branch tip.
prev=$(git -C "$R" rev-parse HEAD~1)
"$CHK" --repo "$R" --task 27 --head "$prev" >/dev/null 2>&1; check "U6 --head anchors the scan" 2 $?

# U7 usage errors are refusals, not silent zeros.
"$CHK" --repo "$R" >/dev/null 2>&1; check "U7 missing --task refused" 1 $?
"$CHK" --repo "$R" --task 27x >/dev/null 2>&1; check "U7b non-numeric task refused" 1 $?

# U8 the instrument's own positive control. If the probe cannot be built the
# check must refuse (3) rather than report a comfortable zero -- a search that
# can never match reports "no markers" exactly like a clean repo does. Reached
# without a test-only backdoor: an unusable TMPDIR makes the probe impossible.
TMPDIR=/nonexistent-dir "$CHK" --repo "$R" --task 27 >/dev/null 2>&1
check "U8 unusable probe -> instrument refuses" 3 $?

printf "\n  %d passed, %d failed\n" "$pass" "$fail"
[ "$fail" -eq 0 ]
