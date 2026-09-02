#!/usr/bin/env bash
# Self-tests for scripts/mutation-guard.sh. Builds a throwaway cargo repo in
# Self-tests for scripts/mutation-guard.sh — see that file for the contract.
# Run:  scripts/mutation-guard-selftest.sh "$(pwd)/scripts/mutation-guard.sh"
# /tmp so the tests do not depend on kv9's own suite, and so a failing self-test
# cannot damage a real worktree.
set -uo pipefail
GUARD="$1"
# One unique outer directory per run, holding BOTH the scratch repo and this
# run's expected-artifact files. The artifacts used to go to fixed paths under
# /tmp ("$R/../mg-expected-*.txt" -- `..` discards the unique suffix) and were
# never removed, so two concurrent runs shared them and each run's leftovers
# became the next run's external state (Tess). That is precisely the cross-run
# contamination the guard exists to refuse; the self-test must not carry the
# same blind spot.
OUT=$(mktemp -d /tmp/mg-run.XXXXXX)
ART="$OUT/artifacts"; mkdir -p "$ART"
R="$OUT/repo"; mkdir -p "$R"
trap 'rm -rf "$OUT"' EXIT INT TERM
pass=0; fail=0
check() { # name expected_rc actual_rc
  if [ "$2" = "$3" ]; then printf "  PASS  %-52s rc=%s\n" "$1" "$3"; pass=$((pass+1));
  else printf "  FAIL  %-52s expected rc=%s got rc=%s\n" "$1" "$2" "$3"; fail=$((fail+1)); fi
}
cd "$R"
cargo init --lib -q --name mg 2>/dev/null
cat > src/lib.rs <<'RS'
include!("generated.rs");
pub fn answer() -> u32 { 42 + EXTRA }
#[cfg(test)]
mod tests {
    extra_test!();
    #[test] fn the_answer_is_42() { assert_eq!(super::answer(), 42); }
}
RS
# A gitignored generated file the crate includes. It is what lets T16 construct a
# mutation whose effect OUTLIVES the declared-file restore: checkout cannot undo
# it and porcelain cannot see it, which is exactly the state the restored phase
# exists to catch.
printf 'pub const EXTRA: u32 = 0;\nmacro_rules! extra_test { () => {} }\n' > src/generated.rs
printf 'target\nsrc/generated.rs\n' > .gitignore
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
open(p,'w').write(s.replace('{ 42 + EXTRA }','{ 43 + EXTRA }',1))" >/dev/null 2>&1; rc=$?
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

# T16 the RESTORED phase. A mutation that edits a declared file AND poisons a
# gitignored generated file: the mutant is red (caught), `git checkout -- src/lib.rs`
# puts the declared file back, but the poison survives -- checkout cannot undo it
# and porcelain cannot see it. Only a third test run notices, so this is the one
# cell that stands on blocker #1's fix.
#
# It exists because the first thing written here was a comment named T16, a
# fixture built and immediately deleted, and no call to the guard at all: labelled
# as covered, contributing zero. Deleting the whole restored phase left 21/21
# green (@Cindy). A cell that never invokes the thing under test is worse than an
# absent one, because its name answers the question nobody then asks.
"$GUARD" --repo "$R" --file src/lib.rs --test "the_answer_is_42" --expect-n 1 \
  -- bash -c "cd $R && printf 'pub const EXTRA: u32 = 100;\\nmacro_rules! extra_test { () => {} }\\n' > src/generated.rs && $MUT" >/dev/null 2>&1; rc=$?
check "T16 effect outliving restore caught in restored" 10 $rc
printf 'pub const EXTRA: u32 = 0;\nmacro_rules! extra_test { () => {} }\n' > "$R/src/generated.rs"
git -C "$R" checkout -- . 2>/dev/null

# T17 gate 4 must RESTORE the declared file while PRESERVING the undeclared one.
# The earlier version left the whole tree alone and called that the contract.
"$GUARD" --repo "$R" --file src/lib.rs --test "the_answer_is_42" --expect-n 1 \
  -- bash -c "cd $R && $MUT && echo x >> Cargo.toml && cp Cargo.toml $ART/expected-cargo.txt" >/dev/null 2>&1; rc=$?
check "T17 gate 4 returns 5" 5 $rc
# worktree AND index, separately -- a restore that fixes only the worktree
# leaves the index staged and the next run refuses at gate 1 (@Cindy).
if git -C "$R" diff --quiet HEAD -- src/lib.rs && git -C "$R" diff --cached --quiet HEAD -- src/lib.rs; then
  printf "  PASS  %-52s\n" "T17b declared restored in worktree AND index"; pass=$((pass+1))
else printf "  FAIL  %-52s\n" "T17b declared restored in worktree AND index"; fail=$((fail+1)); fi
# "preserved" is not "present": assert the bytes are exactly what the mutation
# produced, so a restore that rewrote the residue would still fail here.
# cmp, not $(cat) = $(cat): command substitution strips ALL trailing newlines, so
# the string form only proves equality-after-normalisation and a mutation that
# appends a newline stays green (@Cindy, with a firing counterexample).
if cmp -s "$R/Cargo.toml" "$ART/expected-cargo.txt"; then
  printf "  PASS  %-52s\n" "T17c undeclared residue byte-identical"; pass=$((pass+1))
else printf "  FAIL  %-52s\n" "T17c undeclared residue byte-identical"; fail=$((fail+1)); fi
git -C "$R" checkout -- . 2>/dev/null

# T18 a rename OUT of a declared path: the declared file must come back even
# though the index says it is gone. `checkout --` cannot do this; `checkout HEAD --` can.
"$GUARD" --repo "$R" --file src/lib.rs --test "the_answer_is_42" --expect-n 1 \
  -- bash -c "cd $R && $MUT && git mv src/lib.rs src/moved.rs && cp src/moved.rs $ART/expected-moved.txt" >/dev/null 2>&1; rc=$?
check "T18 rename-out returns 5" 5 $rc
if [ -f "$R/src/lib.rs" ] && git -C "$R" diff --quiet HEAD -- src/lib.rs \
     && git -C "$R" diff --cached --quiet HEAD -- src/lib.rs; then
  printf "  PASS  %-52s\n" "T18b old declared path restored (worktree+index)"; pass=$((pass+1))
else printf "  FAIL  %-52s\n" "T18b old declared path restored (worktree+index)"; fail=$((fail+1)); fi
# The rename DESTINATION must survive as undeclared residue -- asserted apart
# from the restore, so a future "leave the whole tree" regression cannot satisfy
# one combined check (@Cindy).
if [ -f "$R/src/moved.rs" ] && cmp -s "$R/src/moved.rs" "$ART/expected-moved.txt"; then
  printf "  PASS  %-52s\n" "T18c rename destination byte-identical residue"; pass=$((pass+1))
else printf "  FAIL  %-52s\n" "T18c rename destination byte-identical residue"; fail=$((fail+1)); fi
git -C "$R" reset -q --hard >/dev/null 2>&1; rm -f "$R/src/moved.rs"

# T19 a path containing a NEWLINE. Round-tripping paths through command
# substitution + `read` split this into two, and each half could match a declared
# name, so an undeclared path walked straight past gate 4 (Tess).
printf 'a\n' > "$R/a.txt"; printf 'b\n' > "$R/b.txt"
git -C "$R" add -A >/dev/null 2>&1; git -C "$R" -c user.email=t@t -c user.name=t commit -q -m ab >/dev/null 2>&1
"$GUARD" --repo "$R" --file src/lib.rs --file a.txt --file b.txt --test "the_answer_is_42" --expect-n 1 \
  -- bash -c "cd $R && $MUT && python3 -c \"open(chr(97)+'.txt'+chr(10)+chr(98)+'.txt','w').write('x')\"" >/dev/null 2>&1; rc=$?
check "T19 newline in an undeclared path refused" 5 $rc
python3 -c "import os; f=chr(97)+'.txt'+chr(10)+chr(98)+'.txt'; os.path.exists('$R/'+f) and os.remove('$R/'+f)" 2>/dev/null
git -C "$R" checkout -- . 2>/dev/null

# T20 a MULTILINE literal occurring twice must be refused by gate 2. grep cannot
# see a multi-line literal at all, so this only works on byte occurrences.
printf 'pub fn dup_a() -> u32 {\n    5\n}\npub fn dup_b() -> u32 {\n    5\n}\n' >> "$R/src/lib.rs"
git -C "$R" add -A >/dev/null 2>&1; git -C "$R" -c user.email=t@t -c user.name=t commit -q -m dup >/dev/null 2>&1
"$GUARD" --repo "$R" --file src/lib.rs --test "the_answer_is_42" --expect-n 1 \
  --old-once "$(printf ' -> u32 {\n    5\n}')" -- true >/dev/null 2>&1; rc=$?
check "T20 multiline literal occurring twice refused" 3 $rc

# T21 wrong selected count in the RESTORED phase. The mutation renames the test
# via the gitignored include, so restore puts the declared file back but the
# restored run still selects 0.
"$GUARD" --repo "$R" --file src/lib.rs --test "the_answer_is_42" --expect-n 1 \
  -- bash -c "cd $R && printf 'pub const EXTRA: u32 = 0;\\nmacro_rules! extra_test { () => { #[test] fn the_answer_is_42_extra() {} } }\\n' > src/generated.rs && sed -i '/extra_test!();/d' src/lib.rs && $MUT" >/dev/null 2>&1; rc=$?
check "T21 wrong selected count in restored phase" 10 $rc
printf 'pub const EXTRA: u32 = 0;\nmacro_rules! extra_test { () => {} }\n' > "$R/src/generated.rs"
git -C "$R" checkout -- . 2>/dev/null

# T22 when cleanup fails the ORIGINAL run outcome must still be reported, not
# only the cleanup code.
out=$("$GUARD" --repo "$R" --file src/lib.rs --test "the_answer_is_42" --expect-n 1 \
  -- bash -c "cd $R && $MUT && : > .git/index.lock" 2>&1); rc=$?
if [ "$rc" = "8" ] && printf '%s' "$out" | grep -q "run result was rc=0,"; then
  printf "  PASS  %-52s\n" "T22 cleanup failure still reports run outcome"; pass=$((pass+1))
else printf "  FAIL  %-52s  rc=%s\n" "T22 cleanup failure still reports run outcome" "$rc"; fail=$((fail+1)); fi
rm -f "$R/.git/index.lock"; git -C "$R" checkout -- . 2>/dev/null

# T23 a mutation that STAGES its change to a declared file. `checkout --` reads
# the INDEX, so the restored phase would re-test the mutant and report a phantom
# rc=10 (Tess). Restoring from head_before is what makes this 0.
"$GUARD" --repo "$R" --file src/lib.rs --test "the_answer_is_42" --expect-n 1 \
  -- bash -c "cd $R && $MUT && git add src/lib.rs" >/dev/null 2>&1; rc=$?
check "T23 staged declared mutation still caught" 0 $rc
if git -C "$R" diff --quiet HEAD -- src/lib.rs && git -C "$R" diff --cached --quiet HEAD -- src/lib.rs; then
  printf "  PASS  %-52s\n" "T23b staged mutation restored in both layers"; pass=$((pass+1))
else printf "  FAIL  %-52s\n" "T23b staged mutation restored in both layers"; fail=$((fail+1)); fi
git -C "$R" reset -q --hard >/dev/null 2>&1

# T24/T25 real signals. The mutation applies its change and then signals the
# guard directly ($PPID is the guard's shell), so the handler runs on a genuine
# caught signal rather than a fixture that fakes one -- and an async child cannot
# inherit an ignored disposition, because there is no async child.
for sig in INT TERM; do
  case $sig in INT) want=130 ;; TERM) want=143 ;; esac
  # HEAD as of THIS cell -- earlier cells commit, so the fixture's original H0 is
  # not the right baseline here. Comparing against a stale anchor fails for a
  # reason that has nothing to do with the guard.
  head_at_call="$(git -C "$R" rev-parse HEAD)"
  # Launched through a shim that resets SIGINT/SIGTERM to SIG_DFL before exec.
  # Bash cannot trap a signal that was ignored on entry, and a suite run in the
  # background has SIGINT ignored -- so without this the INT cell silently
  # reports rc=0 and its result depends on how the suite was invoked. Tess named
  # this hazard; my own concurrency probe is what walked into it.
  python3 -c "
import signal, os, sys
signal.signal(signal.SIGINT, signal.SIG_DFL)
signal.signal(signal.SIGTERM, signal.SIG_DFL)
os.execv(sys.argv[1], sys.argv[1:])
" "$GUARD" --repo "$R" --file src/lib.rs --test "the_answer_is_42" --expect-n 1 \
    -- bash -c "cd $R && $MUT && kill -$sig \$PPID" >/dev/null 2>&1; rc=$?
  check "T24/25 $sig restores and exits $want" "$want" $rc
  if git -C "$R" diff --quiet HEAD -- src/lib.rs \
     && git -C "$R" diff --cached --quiet HEAD -- src/lib.rs \
     && [ -z "$(git -C "$R" status --porcelain)" ] \
     && [ "$(git -C "$R" rev-parse HEAD)" = "$head_at_call" ]; then
    printf "  PASS  %-52s\n" "T24/25b $sig left tree clean and HEAD unmoved"; pass=$((pass+1))
  else printf "  FAIL  %-52s\n" "T24/25b $sig left tree clean and HEAD unmoved"; fail=$((fail+1)); fi
  git -C "$R" reset -q --hard >/dev/null 2>&1
done

# T26 this run's artifact directory must not outlive the run. Asserted after the
# removal, so a teardown that silently fails is visible rather than assumed.
rm -rf "$OUT"
if [ ! -e "$OUT" ]; then
  printf "  PASS  %-52s\n" "T26 run artifact dir removed at teardown"; pass=$((pass+1))
else printf "  FAIL  %-52s\n" "T26 run artifact dir removed at teardown"; fail=$((fail+1)); fi

printf "\n  %d passed, %d failed\n" "$pass" "$fail"
[ "$fail" -eq 0 ]
