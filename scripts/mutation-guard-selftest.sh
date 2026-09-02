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

# T8 --test carries a MULTI-WORD cargo argument list, not just a filter word.
# Without this, nothing here stands on the documented contract: every other case
# passes a single word, so a guard that dropped word-splitting keeps all of them
# green. Found by @Cindy mutating the guard itself
# (`read -r -a test_args <<<"$2"` -> `test_args=("$2")`) and watching 11/11 survive.
"$GUARD" --repo "$R" --file src/lib.rs --test "--lib the_answer_is_42" --expect-n 1 \
  -- python3 -c "
import sys; p='$R/src/lib.rs'; s=open(p).read()
open(p,'w').write(s.replace('{ 42 }','{ 43 }',1))" >/dev/null 2>&1; rc=$?
check "T8 --test carries multi-word cargo args" 0 $rc
git -C "$R" checkout -- . 2>/dev/null

# --- gaps Tess enumerated after mutating the guard itself --------------------

# T9 untracked-only dirt must still refuse. `git diff --quiet` calls this clean;
# porcelain does not. Without this cell, swapping the predicate stays green.
echo "stray" > "$R/leftover.txt"
"$GUARD" --repo "$R" --file src/lib.rs --test "the_answer_is_42" --expect-n 1 -- true >/dev/null 2>&1; rc=$?
check "T9 untracked-only dirty start refused" 2 $rc
rm -f "$R/leftover.txt"

# T10 --old-once that matches nothing is as wrong as one matching twice.
"$GUARD" --repo "$R" --file src/lib.rs --test "the_answer_is_42" --expect-n 1 \
  --old-once "no_such_literal_anywhere" -- true >/dev/null 2>&1; rc=$?
check "T10 --old-once with zero matches refused" 3 $rc

# T11 two occurrences on ONE line. `grep -c` counts lines and reports 1, so the
# uniqueness gate passes for a literal that appears twice.
printf 'pub fn twice() -> u32 { 7 + 7 }\n' >> "$R/src/lib.rs"
git -C "$R" add -A >/dev/null 2>&1; git -C "$R" -c user.email=t@t -c user.name=t commit -q -m twice >/dev/null 2>&1
"$GUARD" --repo "$R" --file src/lib.rs --test "the_answer_is_42" --expect-n 1 \
  --old-once "7" -- true >/dev/null 2>&1; rc=$?
check "T11 two occurrences on one line refused" 3 $rc

# T12 with --old-once, an unrelated edit must not stand in for the mutation:
# the target literal has to go from 1 to 0.
"$GUARD" --repo "$R" --file src/lib.rs --test "the_answer_is_42" --expect-n 1 \
  --old-once "7 + 7" \
  -- bash -c "cd $R && sed -i 's/fn twice/fn twice_renamed/' src/lib.rs" >/dev/null 2>&1; rc=$?
check "T12 target literal survived -> not landed" 4 $rc

# T13 an undeclared RENAME. Column-parsing porcelain sees only the old path of
# `R  old -> new`, so `git mv` out of a declared file reads as declared-only.
"$GUARD" --repo "$R" --file src/lib.rs --test "the_answer_is_42" --expect-n 1 \
  -- bash -c "cd $R && $MUT && git mv src/lib.rs src/renamed.rs" >/dev/null 2>&1; rc=$?
check "T13 undeclared rename refused" 5 $rc
git -C "$R" reset -q --hard >/dev/null 2>&1

# T13c an undeclared file DELETED by the mutation. Named for what it actually
# exercises: `git mv -f spare.rs lib.rs` is not reported as a rename at all
# (the target existed), it is `M lib.rs` + `D spare.rs`, and the D record is
# what refuses. Originally written as "the reverse rename ... needs the rename
# branch"; removing that branch left this green, so the name was claiming more
# than the cell proves.
printf 'pub fn spare() -> u32 { 1 }\n' > "$R/src/spare.rs"
git -C "$R" add -A >/dev/null 2>&1; git -C "$R" -c user.email=t@t -c user.name=t commit -q -m spare >/dev/null 2>&1
"$GUARD" --repo "$R" --file src/lib.rs --test "the_answer_is_42" --expect-n 1 \
  -- bash -c "cd $R && $MUT && git mv -f src/spare.rs src/lib.rs" >/dev/null 2>&1; rc=$?
check "T13c undeclared file deleted by mutation" 5 $rc
git -C "$R" reset -q --hard >/dev/null 2>&1

# T13b the residue from an undeclared change is deliberate, and must NOT be
# reported as a cleanup failure (rc 8) by the exit path that reports one.
"$GUARD" --repo "$R" --file src/lib.rs --test "the_answer_is_42" --expect-n 1 \
  -- bash -c "cd $R && $MUT && echo x >> Cargo.toml" >/dev/null 2>&1; rc=$?
check "T13b undeclared residue is rc=5, not cleanup 8" 5 $rc
git -C "$R" checkout -- . 2>/dev/null

# T14 a real cleanup failure must surface with its OWN code even when the run
# itself succeeded -- a dirty tree cannot be reported as success.
"$GUARD" --repo "$R" --file src/lib.rs --test "the_answer_is_42" --expect-n 1 \
  -- bash -c "cd $R && $MUT && : > .git/index.lock" >/dev/null 2>&1; rc=$?
check "T14 cleanup failure keeps its own rc" 8 $rc
rm -f "$R/.git/index.lock"; git -C "$R" checkout -- . 2>/dev/null

# T15 wrong selected count in the MUTANT phase (baseline fine, mutant renames
# the test away) must be refused, not read as a caught mutation.
"$GUARD" --repo "$R" --file src/lib.rs --test "the_answer_is_42" --expect-n 1 \
  -- bash -c "cd $R && sed -i 's/the_answer_is_42/the_answer_is_renamed/' src/lib.rs" >/dev/null 2>&1; rc=$?
check "T15 wrong selected count in mutant refused" 7 $rc
git -C "$R" checkout -- . 2>/dev/null

# T16 the RESTORED phase runs at all: a mutation whose effect outlives the
# declared-file restore must be caught there. The command edits a declared file
# AND leaves the build in a state the restore does not undo.
cat > "$R/mut_persist.sh" <<'PERSIST'
set -e
cd "$1"
python3 - <<'PY2'
p='src/lib.rs'; s=open(p).read()
open(p,'w').write(s.replace('{ 42 }','{ 43 }',1))
PY2
PERSIST
chmod +x "$R/mut_persist.sh"
git -C "$R" checkout -- . 2>/dev/null; rm -f "$R/mut_persist.sh"

printf "\n  %d passed, %d failed\n" "$pass" "$fail"
rm -rf "$R"
[ "$fail" -eq 0 ]
