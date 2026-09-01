#!/usr/bin/env bash
# Self-tests for scripts/mutation-guard.sh. Builds a throwaway cargo repo in
# Self-tests for scripts/mutation-guard.sh — see that file for the contract.
# Run:  scripts/mutation-guard-selftest.sh "$(pwd)/scripts/mutation-guard.sh"
# /tmp so the tests do not depend on kv9's own suite, and so a failing self-test
# cannot damage a real worktree.
set -uo pipefail
GUARD="$1"
R=$(mktemp -d /tmp/mg-repo.XXXXXX)
pass=0; fail=0
check() { # name expected_rc actual_rc
  if [ "$2" = "$3" ]; then printf "  PASS  %-52s rc=%s\n" "$1" "$3"; pass=$((pass+1));
  else printf "  FAIL  %-52s expected rc=%s got rc=%s\n" "$1" "$2" "$3"; fail=$((fail+1)); fi
}
cd "$R"
cargo init --lib -q --name mg 2>/dev/null
cat > src/lib.rs <<'RS'
pub fn answer() -> u32 { 42 }
#[cfg(test)]
mod tests {
    #[test] fn the_answer_is_42() { assert_eq!(super::answer(), 42); }
}
RS
echo "target" > .gitignore
# Run the suite ONCE before the first commit so Cargo.lock is tracked, as it is
# in any real repo. Otherwise `cargo test` creates it mid-run and the guard
# correctly refuses it as an undeclared change -- which is the guard working,
# but it would make these self-tests measure my fixture instead of the guard.
cargo test >/dev/null 2>&1
git add -A >/dev/null 2>&1 && git -c user.email=t@t -c user.name=t commit -q -m init >/dev/null 2>&1
H0=$(git rev-parse HEAD)

MUT='python3 -c "
import sys
p=sys.argv[1]; s=open(p).read()
open(p,\"w\").write(s.replace(\"42\",\"43\",1))
" src/lib.rs'

# T1 clean tree, real mutation -> caught (rc 0), and nothing left behind
out=$("$GUARD" --repo "$R" --file src/lib.rs --test "the_answer_is_42" --expect-n 1 -- bash -c "cd $R && $MUT" 2>&1); rc=$?
check "T1 clean start, mutation caught" 0 $rc
[ -z "$(git -C "$R" status --porcelain)" ] && [ "$(git -C "$R" rev-parse HEAD)" = "$H0" ] \
  && printf "  PASS  %-52s\n" "T1b restored: tree clean and HEAD unmoved" && pass=$((pass+1)) \
  || { printf "  FAIL  %-52s\n" "T1b restored"; fail=$((fail+1)); }

# T2 dirty start -> refused, and not one byte changed
echo "// scratch" >> src/lib.rs
before=$(md5sum src/lib.rs | cut -d' ' -f1)
"$GUARD" --repo "$R" --file src/lib.rs --test "the_answer_is_42" --expect-n 1 -- true >/dev/null 2>&1; rc=$?
check "T2 dirty start refused" 2 $rc
after=$(md5sum src/lib.rs | cut -d' ' -f1)
[ "$before" = "$after" ] && printf "  PASS  %-52s\n" "T2b refused without changing a byte" && pass=$((pass+1)) \
  || { printf "  FAIL  %-52s\n" "T2b refused without changing a byte"; fail=$((fail+1)); }
git -C "$R" checkout -- src/lib.rs

# T3 --old-once literal that occurs twice -> refused
printf '\npub fn other() -> u32 { 42 }\n' >> src/lib.rs
git -C "$R" add -A >/dev/null 2>&1 && git -C "$R" -c user.email=t@t -c user.name=t commit -q -m two >/dev/null 2>&1
H1=$(git -C "$R" rev-parse HEAD)
"$GUARD" --repo "$R" --file src/lib.rs --test "the_answer_is_42" --expect-n 1 --old-once "42" -- true >/dev/null 2>&1; rc=$?
check "T3 --old-once occurring twice refused" 3 $rc

# T4 mutation that changes nothing -> refused
"$GUARD" --repo "$R" --file src/lib.rs --test "the_answer_is_42" --expect-n 1 -- true >/dev/null 2>&1; rc=$?
check "T4 mutation that lands nothing refused" 4 $rc

# T5 mutation touching an undeclared path -> refused AND left in place
"$GUARD" --repo "$R" --file src/lib.rs --test "the_answer_is_42" --expect-n 1 \
  -- bash -c "cd $R && $MUT && echo x >> Cargo.toml" >/dev/null 2>&1; rc=$?
check "T5 undeclared path refused" 5 $rc
git -C "$R" status --porcelain | grep -q "Cargo.toml" \
  && printf "  PASS  %-52s\n" "T5b undeclared change LEFT in place" && pass=$((pass+1)) \
  || { printf "  FAIL  %-52s\n" "T5b undeclared change left in place"; fail=$((fail+1)); }
git -C "$R" checkout -- . 2>/dev/null

# T6 wrong --expect-n -> refused before mutating
"$GUARD" --repo "$R" --file src/lib.rs --test "the_answer_is_42" --expect-n 7 -- bash -c "cd $R && $MUT" >/dev/null 2>&1; rc=$?
check "T6 wrong expected selection count refused" 6 $rc
[ -z "$(git -C "$R" status --porcelain)" ] && printf "  PASS  %-52s\n" "T6b refused before mutating" && pass=$((pass+1)) \
  || { printf "  FAIL  %-52s\n" "T6b refused before mutating"; fail=$((fail+1)); }

# T7 a mutation the test cannot see -> reported as SURVIVED, not as success
"$GUARD" --repo "$R" --file src/lib.rs --test "the_answer_is_42" --expect-n 1 \
  -- bash -c "cd $R && sed -i 's/pub fn other() -> u32 { 42 }/pub fn other() -> u32 { 99 }/' src/lib.rs" >/dev/null 2>&1; rc=$?
check "T7 undetected mutation reported as SURVIVED" 9 $rc

printf "\n  %d passed, %d failed\n" "$pass" "$fail"
rm -rf "$R"
[ "$fail" -eq 0 ]
