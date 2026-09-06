#!/usr/bin/env bash
# Self-tests for scripts/apply-store-tripwire.sh -- see that file for the contract.
#   scripts/apply-store-tripwire-selftest.sh "$(pwd)/scripts/apply-store-tripwire.sh"
#
# Every run gets one unique scratch tree, removed on every terminal path.
# The needle below is assembled from fragments so this file itself never
# trips a scan of any directory that happens to contain it.
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
# food) and a raft dir that does not.
mkdir -p "$R/crates/raft/src" "$R/crates/engine/src"
printf 'pub fn apply() {}\n' > "$R/crates/raft/src/lib.rs"
printf 'pub trait %s { fn put(&self); }\n' "$NEEDLE" > "$R/crates/engine/src/lib.rs"

# T1 clean tree -> 0, and the pass names both the zero and the control count.
t1=$("$CHK" --repo "$R" 2>&1); check "T1 clean raft crate -> passes" 0 $?
if printf '%s' "$t1" | grep -q "0 reference(s)"; then
  printf "  PASS  %-58s\n" "T1b legitimate zero reports an explicit count"; pass=$((pass+1))
else printf "  FAIL  %-58s\n" "T1b legitimate zero reports an explicit count"; fail=$((fail+1)); fi

# T2 FIRING: a reference in the raft crate must red with exact file:line.
printf 'use kv9_engine::%s;\n' "$NEEDLE" > "$R/crates/raft/src/bad.rs"
"$CHK" --repo "$R" >/dev/null 2>&1; check "T2 reference in raft crate -> fires" 2 $?
t2=$("$CHK" --repo "$R" 2>&1)
if printf '%s' "$t2" | grep -q "bad.rs:1"; then
  printf "  PASS  %-58s\n" "T2b report gives exact file:line"; pass=$((pass+1))
else printf "  FAIL  %-58s\n" "T2b report gives exact file:line"; fail=$((fail+1)); fi
rm "$R/crates/raft/src/bad.rs"

# T3 restored -> green again (the fire test does not poison the fixture).
"$CHK" --repo "$R" >/dev/null 2>&1; check "T3 reference removed -> passes again" 0 $?

# T4 INSTRUMENT: an empty positive control is rc 3, never a pass. This is the
# guard against "scan finds nothing anywhere and calls the tree clean".
: > "$R/crates/engine/src/lib.rs"
"$CHK" --repo "$R" >/dev/null 2>&1; check "T4 dead positive control -> instrument failure" 3 $?
printf 'pub trait %s { fn put(&self); }\n' "$NEEDLE" > "$R/crates/engine/src/lib.rs"

# T5 INSTRUMENT: a missing scan directory is rc 3, never a pass.
mv "$R/crates/raft/src" "$R/crates/raft/gone"
"$CHK" --repo "$R" >/dev/null 2>&1; check "T5 missing scan dir -> instrument failure" 3 $?
mv "$R/crates/raft/gone" "$R/crates/raft/src"

printf '%d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
