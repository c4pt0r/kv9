#!/usr/bin/env bash
# Self-tests for scripts/manifest-key-tripwire.sh -- see that file for the
# contract.
#   scripts/manifest-key-tripwire-selftest.sh "$(pwd)/scripts/manifest-key-tripwire.sh"
#
# Unique scratch tree per run, removed on every terminal path. The needles
# are assembled from fragments so this file never trips a scan of any tree
# that contains it.
set -uo pipefail
CHK="${1:?usage: $0 /path/to/manifest-key-tripwire.sh}"
OUT=$(mktemp -d /tmp/mkt-run.XXXXXX)
trap 'rm -rf "$OUT"' EXIT INT TERM
R="$OUT/repo"
pass=0; fail=0
check() { if [ "$2" = "$3" ]; then printf "  PASS  %-62s rc=%s\n" "$1" "$3"; pass=$((pass+1));
          else printf "  FAIL  %-62s expected rc=%s got rc=%s\n" "$1" "$2" "$3"; fail=$((fail+1)); fi; }

PAIR="x00manifest_""pair"
GEN="x00manifest_""gen"

owner_fixture() {
  mkdir -p "$R/crates/raft/src" "$R/crates/region/src"
  {
    printf 'fn pair_key() -> Vec<u8> { b"\\%s\\x00".to_vec() }\n' "$PAIR"
    printf 'fn gen_key() -> Vec<u8> { b"\\%s\\x00".to_vec() }\n' "$GEN"
  } > "$R/crates/raft/src/state_machine.rs"
  printf 'pub fn apply() {}\n' > "$R/crates/region/src/lib.rs"
}
owner_fixture

# T1 clean tree passes with explicit counts.
t1=$("$CHK" --repo "$R" 2>&1); check "T1 clean tree -> passes" 0 $?
if printf '%s' "$t1" | grep -q "0 escapes; 2 in-owner"; then
  printf "  PASS  %-62s\n" "T1b pass names both measured numbers"; pass=$((pass+1))
else printf "  FAIL  %-62s\n" "T1b pass names both measured numbers"; fail=$((fail+1)); fi

# T2 FIRING: an escape in another file reds with exact file:line.
printf 'const K: &[u8] = b"\\%s\\x00";\n' "$PAIR" > "$R/crates/region/src/bad.rs"
"$CHK" --repo "$R" >/dev/null 2>&1; check "T2 escape in another file -> fires" 2 $?
rm "$R/crates/region/src/bad.rs"

# T3 THE ROUND-3 BYPASS, pinned as a named regression: an escape in a file
# whose NAME CONTAINS the owner's filename must still fire. The substring
# exclusion passed this with rc=0.
printf 'const K: &[u8] = b"\\%s\\x00";\n' "$PAIR" > "$R/crates/region/src/not_state_machine.rs"
"$CHK" --repo "$R" >/dev/null 2>&1
check "T3 escape in not_state_machine.rs -> STILL fires" 2 $?
rm "$R/crates/region/src/not_state_machine.rs"

# T4 a second in-owner literal (possible second writer path) fires.
printf 'const SECOND: &[u8] = b"\\%s\\x00";\n' "$PAIR" >> "$R/crates/raft/src/state_machine.rs"
"$CHK" --repo "$R" >/dev/null 2>&1; check "T4 second in-owner literal -> fires" 2 $?
owner_fixture

# T5 restored -> green again.
"$CHK" --repo "$R" >/dev/null 2>&1; check "T5 restored -> passes again" 0 $?

# T6 INSTRUMENT: dead positive control (owner lost its tokens) is rc 3.
printf 'fn nothing() {}\n' > "$R/crates/raft/src/state_machine.rs"
"$CHK" --repo "$R" >/dev/null 2>&1; check "T6 dead positive control -> instrument failure" 3 $?
owner_fixture

# T7 INSTRUMENT: missing owner file is rc 3, never a pass.
mv "$R/crates/raft/src/state_machine.rs" "$R/crates/raft/src/gone.rs"
"$CHK" --repo "$R" >/dev/null 2>&1; check "T7 missing owner -> instrument failure" 3 $?
mv "$R/crates/raft/src/gone.rs" "$R/crates/raft/src/state_machine.rs"

printf '%d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
