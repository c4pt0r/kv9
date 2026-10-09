#!/usr/bin/env bash
# Self-tests for scripts/apply-store-tripwire.sh -- see that file for the contract.
#   scripts/apply-store-tripwire-selftest.sh "$(pwd)/scripts/apply-store-tripwire.sh"
#
# Every run gets one unique scratch tree, removed on every terminal path.
# The needle below is assembled from fragments so this file itself never
# trips a scan of any directory that happens to contain it.
#
# The fixture mirrors the production ALLOWANCE: the three allowed test files
# exist at exactly their pinned counts (2/4/2), so the baseline green is the
# same green the production tree shows -- not the pre-allowance "zero
# everywhere" green, which no real tree exhibits any more.
set -uo pipefail
CHK="${1:?usage: $0 /path/to/apply-store-tripwire.sh}"
OUT=$(mktemp -d /tmp/ast-run.XXXXXX)
trap 'rm -rf "$OUT"' EXIT INT TERM
R="$OUT/repo"
pass=0; fail=0
check() { if [ "$2" = "$3" ]; then printf "  PASS  %-58s rc=%s\n" "$1" "$3"; pass=$((pass+1));
          else printf "  FAIL  %-58s expected rc=%s got rc=%s\n" "$1" "$2" "$3"; fail=$((fail+1)); fi; }

NEEDLE="Object""Store"

# Fixture: an engine dir that legitimately names the store (positive control
# food), a raft dir whose production files do not, and the three ALLOWED test
# files at exactly their pinned counts.
build_fixture() {
  rm -rf "$R"
  mkdir -p "$R/crates/raft/src/snapshot_install/capture" \
           "$R/crates/raft/src/state_machine" \
           "$R/crates/engine/src"
  printf 'pub fn apply() {}\n' > "$R/crates/raft/src/lib.rs"
  printf 'pub trait %s { fn put(&self); }\n' "$NEEDLE" > "$R/crates/engine/src/lib.rs"
  seq_hits() { local n=$1 f=$2; : > "$f"; for _ in $(seq "$n"); do
    printf '// test uses %s\n' "$NEEDLE" >> "$f"; done; }
  seq_hits 2 "$R/crates/raft/src/snapshot_install/capture/tests.rs"
  seq_hits 4 "$R/crates/raft/src/snapshot_install/tests.rs"
  seq_hits 2 "$R/crates/raft/src/state_machine/checkpoint_recovery.rs"
}
build_fixture

# T1 baseline: allowed files at pinned counts -> green, and the pass names
# both the escape-zero and the pinned total (positive witness of the new
# accounting, not a generic ok).
t1=$("$CHK" --repo "$R" 2>&1); check "T1 pinned fixture -> passes" 0 $?
if printf '%s' "$t1" | grep -q "0 escapes; 8 pinned"; then
  printf "  PASS  %-58s\n" "T1b pass names escapes-zero and the pinned total"; pass=$((pass+1))
else printf "  FAIL  %-58s\n" "T1b pass names escapes-zero and the pinned total"; fail=$((fail+1)); fi

# T2 ESCAPE: a reference in a non-allowed raft file must red with file:line.
printf 'use kv9_engine::%s;\n' "$NEEDLE" > "$R/crates/raft/src/bad.rs"
"$CHK" --repo "$R" >/dev/null 2>&1; check "T2 reference outside allowed files -> fires" 2 $?
t2=$("$CHK" --repo "$R" 2>&1)
if printf '%s' "$t2" | grep -q "bad.rs:1"; then
  printf "  PASS  %-58s\n" "T2b escape report gives exact file:line"; pass=$((pass+1))
else printf "  FAIL  %-58s\n" "T2b escape report gives exact file:line"; fail=$((fail+1)); fi
rm "$R/crates/raft/src/bad.rs"

# T3 restored -> green again (fire tests do not poison the fixture).
"$CHK" --repo "$R" >/dev/null 2>&1; check "T3 escape removed -> passes again" 0 $?

# T4 DRIFT UP: one extra hit in an allowed file is a pin violation, not a
# quiet widening of the allowance.
printf '// one more %s\n' "$NEEDLE" >> "$R/crates/raft/src/snapshot_install/tests.rs"
"$CHK" --repo "$R" >/dev/null 2>&1; check "T4 extra hit in allowed file -> fires" 2 $?
build_fixture

# T5 DRIFT DOWN: a LOST pinned hit also fires -- the allowed surface changed
# without review, in either direction.
printf '// test uses %s\n' "$NEEDLE" > "$R/crates/raft/src/state_machine/checkpoint_recovery.rs"
"$CHK" --repo "$R" >/dev/null 2>&1; check "T5 lost pinned hit -> fires" 2 $?
build_fixture

# T6 INSTRUMENT: an empty positive control is rc 3, never a pass. This is the
# guard against "scan finds nothing anywhere and calls the tree clean".
: > "$R/crates/engine/src/lib.rs"
"$CHK" --repo "$R" >/dev/null 2>&1; check "T6 dead positive control -> instrument failure" 3 $?
build_fixture

# T7 INSTRUMENT: a missing scan directory is rc 3, never a pass.
mv "$R/crates/raft/src" "$R/crates/raft/gone"
"$CHK" --repo "$R" >/dev/null 2>&1; check "T7 missing scan dir -> instrument failure" 3 $?
mv "$R/crates/raft/gone" "$R/crates/raft/src"

printf '%d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
