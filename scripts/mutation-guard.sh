#!/usr/bin/env bash
#
# Run one mutation against a declared set of files and restore exactly those,
# refusing loudly at every point where a mutation run can silently lie.
#
# Why this exists: "commit before running a mutation" failed three times in one
# day (Ren twice, Rafa once) as a remembered rule, because it asks you to think
# of it while you are concentrating on the mutation. Every check below is a
# refusal, not a reminder.
#
#   scripts/mutation-guard.sh \
#       --repo <dir> --file <path> [--file <path>...] \
#       --test "<full cargo test args>" --expect-n <count> \
#       [--old-once <literal>] \
#       -- <command that applies the mutation>
#
# Exit codes are distinct so a caller can tell refusal from result:
#   0  baseline passed, mutant FAILED (the mutation was caught)  -- the good case
#   1  usage
#   2  gate 1: worktree not clean at start (nothing was modified)
#   3  gate 2: --old-once literal does not occur exactly once
#   4  gate 3: the mutation did not change any declared file
#   5  gate 4: an undeclared path changed (left in place, NOT reverted for you)
#   6  baseline did not select exactly --expect-n tests, or did not pass
#   7  mutant did not select exactly --expect-n tests
#   8  cleanup failed -- tree dirty or HEAD moved after restore
#   9  MUTANT SURVIVED: baseline and mutant both passed
#
# Contract notes, each from a specific failure:
#
#   * Cleanliness is `git status --porcelain`, never `git diff --quiet`. The
#     latter only asserts "no tracked file changed" and reports a tree littered
#     with untracked mutation leftovers as clean -- its success value coincides
#     with its blind spot (Cindy).
#   * Outputs go to a temp dir OUTSIDE the repo. Cindy's own harness lived in
#     the tree under test, so one gate saw its files and another did not, and
#     the same tree got opposite verdicts. This script may live in scripts/
#     because a tracked, unmodified file does not appear in porcelain -- but its
#     working files must not.
#   * `--old-once` takes the FULL literal the mutation replaces, never a prefix.
#     A two-line prefix of one of Ren's patterns matched 2 sites here and 226 in
#     Cindy's tree; "it only matched a couple extra" is not a safe intuition.
#   * --test takes the FULL cargo test argument list, e.g. "-p kv9-server foo".
#     The expect-n gate is a SECOND, fail-closed line under this: drop the word
#     splitting and the first real multi-word invocation is refused loudly rather
#     than passing silently. That bounds the damage; it is not coverage. A
#     capability the docs promise needs a self-test standing on it, or nothing
#     distinguishes a build that has it from one that does not -- @Cindy proved
#     exactly that by mutating this line and watching all 11 cases stay green
#     (T8 in the self-tests now reds on that mutation).
#     A bare filter at a workspace root whose root is itself a package silently
#     runs only that package: here `cargo test fence_firing` selects 0 while
#     `cargo test -p kv9-server fence_firing` selects 3. The expect-n gate turns
#     that into a refusal instead of a meaningless green.
#   * Baseline, mutant and restore all assert the SAME selected count. A filter
#     that silently matches zero or the wrong tests reports "ok" and proves
#     nothing (TESTING.md).
#   * Restore is registered BEFORE the mutation is applied, so an interrupt or a
#     panic still restores. A cleanup failure is loud and keeps its own exit
#     code rather than masking the run's result.
#   * The tree must be clean INCLUDING untracked files, so anything your build
#     creates must be gitignored or committed. An untracked Cargo.lock, a stray
#     .out file, a scratch script -- each is an undeclared change and each is
#     refused. That is deliberate: an untracked leftover from a previous run is
#     exactly what silently contaminates the next one.
#   * Undeclared changes are reported and LEFT ALONE. Deleting a file this
#     script did not create, to tidy up after itself, is not tidying.
set -uo pipefail

die() { printf '%s\n' "$*" >&2; exit "${2:-1}"; }
note() { printf '  %s\n' "$*" >&2; }

repo="" ; expect_n="" ; old_once="" ; files=() ; test_args=()
while [ $# -gt 0 ]; do
  case "$1" in
    --repo) repo="${2:?}"; shift 2 ;;
    --file) files+=("${2:?}"); shift 2 ;;
    --test) read -r -a test_args <<<"${2:?}"; shift 2 ;;
    --expect-n) expect_n="${2:?}"; shift 2 ;;
    --old-once) old_once="${2:?}"; shift 2 ;;
    --) shift; break ;;
    *) die "unknown argument: $1" 1 ;;
  esac
done
[ -n "$repo" ] && [ ${#test_args[@]} -gt 0 ] && [ -n "$expect_n" ] && [ ${#files[@]} -gt 0 ] || \
  die "usage: $0 --repo D --file F [--file F...] --test \"CARGO ARGS\" --expect-n N [--old-once LIT] -- CMD..." 1
[ $# -gt 0 ] || die "no mutation command given after --" 1

work="$(mktemp -d /tmp/kv9-mutation-guard.XXXXXX)"   # outside the tree under test
trap 'rm -rf "$work"' EXIT

porcelain() { git -C "$repo" status --porcelain; }

# ---- gate 1: clean start, and say what would have been destroyed ------------
dirty="$(porcelain)"
if [ -n "$dirty" ]; then
  note "REFUSING (gate 1): the worktree is not clean; restoring declared files"
  note "would destroy work that exists nowhere else:"
  printf '%s\n' "$dirty" >&2
  exit 2
fi
head_before="$(git -C "$repo" rev-parse HEAD)"
note "gate 1 ok: clean at $head_before"

# ---- gate 2: the literal is unique, and it is the full literal --------------
if [ -n "$old_once" ]; then
  hits=0
  for f in "${files[@]}"; do
    n=$(grep -F -c -- "$old_once" "$repo/$f" 2>/dev/null || true)
    hits=$((hits + ${n:-0}))
  done
  [ "$hits" -eq 1 ] || die "REFUSING (gate 2): --old-once occurs $hits times across declared files, expected exactly 1 (pass the FULL literal, never a prefix)" 3
  note "gate 2 ok: literal occurs exactly once"
fi

# ---- baseline, before anything is touched ----------------------------------
run_tests() {  # -> writes output to $1, echoes "<selected> <rc>"
  local out="$1" rc sel
  ( cd "$repo" && cargo test "${test_args[@]}" ) >"$out" 2>&1; rc=$?
  sel=$(grep -oE '^running [0-9]+ test' "$out" | grep -oE '[0-9]+' | paste -sd+ - | bc 2>/dev/null || echo 0)
  echo "${sel:-0} $rc"
}
read -r base_sel base_rc <<<"$(run_tests "$work/baseline.out")"
[ "$base_sel" = "$expect_n" ] || die "REFUSING (gate: filter): baseline selected $base_sel tests, expected $expect_n -- a filter matching the wrong count proves nothing" 6
[ "$base_rc" -eq 0 ] || die "REFUSING (gate: baseline): baseline is not green (rc=$base_rc); a mutation result means nothing on a red baseline" 6
note "baseline ok: $base_sel selected, green"

# ---- restore registered BEFORE the mutation exists -------------------------
restore_rc=0
restore() {
  git -C "$repo" checkout -- "${files[@]}" 2>/dev/null || restore_rc=8
  local left; left="$(porcelain)"
  local head_now; head_now="$(git -C "$repo" rev-parse HEAD)"
  if [ -n "$left" ] || [ "$head_now" != "$head_before" ]; then
    note "CLEANUP FAILED: tree not restored or HEAD moved"
    [ -n "$left" ] && printf '%s\n' "$left" >&2
    [ "$head_now" != "$head_before" ] && note "HEAD $head_before -> $head_now"
    restore_rc=8
  fi
}
trap 'restore; rm -rf "$work"; exit $(( restore_rc ? restore_rc : 130 ))' INT TERM
trap 'restore; rm -rf "$work"' EXIT

# ---- apply the mutation ----------------------------------------------------
"$@" >"$work/mutate.out" 2>&1 || { cat "$work/mutate.out" >&2; die "REFUSING (gate 3): the mutation command failed" 4; }

changed="$(porcelain)"
[ -n "$changed" ] || die "REFUSING (gate 3): the mutation changed nothing -- a survivor here would be meaningless" 4

undeclared="$(printf '%s\n' "$changed" | awk '{print $2}' | while read -r p; do
  for f in "${files[@]}"; do [ "$p" = "$f" ] && continue 2; done; printf '%s\n' "$p"
done)"
if [ -n "$undeclared" ]; then
  note "REFUSING (gate 4): the mutation touched undeclared path(s). They are LEFT IN PLACE:"
  printf '%s\n' "$undeclared" >&2
  exit 5
fi
note "gate 3/4 ok: only declared files changed"

# ---- mutant ----------------------------------------------------------------
read -r mut_sel mut_rc <<<"$(run_tests "$work/mutant.out")"
[ "$mut_sel" = "$expect_n" ] || die "REFUSING (gate: filter): mutant selected $mut_sel tests, expected $expect_n -- baseline and mutant must judge the same set" 7

if [ "$mut_rc" -eq 0 ]; then
  note "MUTANT SURVIVED: baseline green and mutant green -- this cell does not discriminate"
  grep -E '^test result:' "$work/mutant.out" >&2 || true
  exit 9
fi
note "caught: mutant is red"
grep -E '^test result:|panicked at|assertion' "$work/mutant.out" | head -6 >&2 || true
exit 0
