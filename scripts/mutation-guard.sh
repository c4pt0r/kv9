#!/usr/bin/env bash
#
# Run one mutation against a declared set of files and restore exactly those,
# refusing loudly at every point where a mutation run can silently lie.
#
# Why this exists: "commit before running a mutation" failed three times in one
# day as a remembered rule, because it asks you to think of it while you are
# concentrating on the mutation. Every check below is a refusal, not a reminder.
#
#   scripts/mutation-guard.sh \
#       --repo <dir> --file <path> [--file <path>...] \
#       --test "<full cargo test args>" --expect-n <count> \
#       [--old-once <literal>] \
#       -- <command that applies the mutation>
#
# Exit codes are distinct so a caller can tell refusal from result:
#   0   baseline green, mutant RED, restored green -- the mutation was caught
#   1   usage
#   2   gate 1: worktree not clean at start (nothing was modified)
#   3   gate 2: --old-once literal does not occur exactly once
#   4   gate 3: the mutation did not land (nothing changed, or the target
#       literal survived)
#   5   gate 4: an undeclared path changed (left in place, NOT reverted)
#   6   baseline did not select exactly --expect-n, or was not green
#   7   mutant did not select exactly --expect-n
#   8   cleanup failed -- declared files not restored, or HEAD moved
#   9   MUTANT SURVIVED: baseline and mutant both green
#   10  restored phase did not select exactly --expect-n, or was not green
#   130 interrupted (SIGINT) -- declared files restored first
#   143 terminated (SIGTERM) -- declared files restored first
#
# THREE test phases, not two. Baseline, mutant, and *restored* all assert the
# same selected count, and the restored phase must be green. Without the third,
# "the tree was put back" rests on file contents alone -- and a mutation that
# perturbs something outside the declared files (a lockfile, a generated
# artifact) leaves a tree that looks restored and no longer behaves like the
# baseline. The restored run is what makes "no lasting effect" an observation.
#
# Exit-code discipline: the run's own result and a cleanup failure are separate
# facts. If restore fails, the run's result is still printed, but the process
# exits with the cleanup code -- a dirty tree must never be reported as success
# just because the mutation was caught.
#
# Contract notes, each from a specific failure:
#
#   * Cleanliness is `git status --porcelain`, never `git diff --quiet`. The
#     latter only asserts "no tracked file changed" and reports a tree littered
#     with untracked leftovers as clean -- its success value coincides with its
#     blind spot (Cindy).
#   * Changed paths are enumerated NUL-safe. Parsing `git status --porcelain` by
#     whitespace columns loses the NEW path of `R  old -> new` -- `awk '{print $2}'`
#     yields the old one -- so `git mv` out of a declared file read as "only
#     declared files changed" (Tess). In -z form the new path comes FIRST, which
#     is what the fix relies on.
#     The enumerator also emits a rename's OLD path. **No self-test reaches that
#     branch, and I could not construct one**: in every shape I produced the
#     deciding path was already the first record (rename out of a declared file
#     -> new path undeclared; rename onto an existing declared file -> git
#     reports M+D, not R, and the D record refuses). It is kept as defence
#     against a rename-detection configuration where the old path is the only
#     undeclared one -- stated here rather than left looking covered, because
#     removing it reds nothing.
#   * `--old-once` counts BYTE OCCURRENCES across the declared files, not
#     matching lines. `grep -c` collapses `{ 42 + 42 }` to one hit and reports
#     "exactly once" for a literal that appears twice; it also cannot see a
#     multi-line literal at all (Tess).
#   * With `--old-once`, "the mutation landed" means that exact literal went
#     from 1 to 0. Merely observing that some declared file changed lets an
#     unrelated edit -- renaming a function while leaving the target intact --
#     stand in as evidence for the mutation (Tess).
#   * The tree must be clean INCLUDING untracked files, so anything your build
#     creates must be gitignored or committed. An untracked leftover from a
#     previous run is exactly what contaminates the next one.
#   * Undeclared changes are reported and PRESERVED (exit 5) -- deleting a file
#     this script did not create is not tidying -- while the DECLARED files are
#     still restored. Restoring via `checkout <head> --` rather than
#     `checkout --` is what makes that possible for a rename out of a declared
#     path: the plain form cannot resurrect a file the index says is gone.
#     An earlier version left the whole tree alone and this header described
#     that as the contract; it was not. Do not let an implementation rewrite
#     the acceptance criteria it is supposed to meet.
#   * --test takes the FULL cargo argument list, e.g. "-p kv9-server foo".
#     A bare filter at a workspace root whose root is itself a package silently
#     runs only that package. The expect-n gate is a SECOND, fail-closed line
#     under this: drop the word splitting and the first real multi-word call is
#     refused loudly rather than passing silently. That bounds the damage; it is
#     not coverage. A capability the docs promise needs a self-test standing on
#     it -- @Cindy proved exactly that by mutating the splitting away and
#     watching all 11 cases stay green (T8 now reds on it).
#
# The runner may live in scripts/ because a tracked, unmodified file does not
# appear in porcelain. When editing the runner ITSELF, drive from a committed
# baseline or an out-of-repo copy: it gets no dirty-tree exemption for itself.
# Its working files always go outside the tree under test.
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

porcelain() { git -C "$repo" status --porcelain; }

# Undeclared-path check, done END TO END in bytes: porcelain -z is parsed, the
# declared set is compared, and only a human-readable report crosses back. A path
# containing a newline used to be split into two by command substitution + read,
# and each half could match a declared name (Tess) -- so nothing here round-trips
# a path through the shell.
undeclared_report() {   # stdout: escaped undeclared paths; rc 1 if any
  git -C "$repo" status --porcelain -z | python3 -c '
import sys
declared = set(a.encode() for a in sys.argv[1:])
parts = sys.stdin.buffer.read().split(b"\0")
found, i = [], 0
while i < len(parts):
    e = parts[i]
    if not e:
        i += 1; continue
    xy, path = e[:2], e[3:]
    paths = [path]
    if b"R" in xy or b"C" in xy:      # the next record is the ORIGINAL path
        i += 1
        if i < len(parts) and parts[i]:
            paths.append(parts[i])
    for pth in paths:
        if pth not in declared:
            found.append(pth)
    i += 1
for f in found:
    sys.stdout.write(repr(f.decode("utf-8", "backslashreplace"))[1:-1] + "\n")
sys.exit(1 if found else 0)
' "${files[@]}"
}

# Byte occurrences of a literal across the declared files (not matching lines).
occurrences() {
  ( cd "$repo" && python3 -c '
import sys
lit = sys.argv[1].encode()
total = 0
for f in sys.argv[2:]:
    try:
        total += open(f, "rb").read().count(lit)
    except FileNotFoundError:
        pass
print(total)
' "$1" "${files[@]}" )
}

# ---- gate 1: clean start, and say what would have been destroyed ------------
dirty="$(porcelain)"
if [ -n "$dirty" ]; then
  note "REFUSING (gate 1): the worktree is not clean; restoring declared files"
  note "would destroy work that exists nowhere else:"
  printf '%s\n' "$dirty" >&2
  rm -rf "$work"; exit 2
fi
head_before="$(git -C "$repo" rev-parse HEAD)"
note "gate 1 ok: clean at $head_before"

# ---- gate 2: the literal is unique, counted as occurrences ------------------
if [ -n "$old_once" ]; then
  n="$(occurrences "$old_once")"
  [ "$n" = "1" ] || { rm -rf "$work"; die "REFUSING (gate 2): --old-once occurs $n times across the declared files, expected exactly 1 (occurrences, not matching lines; pass the FULL literal)" 3; }
  note "gate 2 ok: literal occurs exactly once"
fi

run_tests() {  # -> "<selected> <rc>"
  local out="$1" rc sel
  ( cd "$repo" && cargo test "${test_args[@]}" ) >"$out" 2>&1; rc=$?
  sel=$(grep -oE '^running [0-9]+ test' "$out" | grep -oE '[0-9]+' | paste -sd+ - | bc 2>/dev/null || echo 0)
  echo "${sel:-0} $rc"
}

read -r base_sel base_rc <<<"$(run_tests "$work/baseline.out")"
[ "$base_sel" = "$expect_n" ] || { rm -rf "$work"; die "REFUSING (gate: filter): baseline selected $base_sel tests, expected $expect_n -- a filter matching the wrong count proves nothing" 6; }
[ "$base_rc" -eq 0 ] || { rm -rf "$work"; die "REFUSING (gate: baseline): baseline is not green (rc=$base_rc); a mutation result means nothing on a red baseline" 6; }
note "baseline ok: $base_sel selected, green"

# ---- restore registered BEFORE the mutation exists -------------------------
cleanup_rc=0
keep_undeclared=0       # gate 4: declared come back, undeclared stay
restore() {
  # `checkout HEAD --` restores index AND worktree, so it also undoes a rename
  # OUT of a declared path (which plain `checkout --` cannot: the index says the
  # file is gone). The rename's destination stays behind as undeclared residue.
  git -C "$repo" checkout "$head_before" -- "${files[@]}" 2>/dev/null || cleanup_rc=8
  local head_now; head_now="$(git -C "$repo" rev-parse HEAD 2>/dev/null)"
  if [ "$head_now" != "$head_before" ]; then
    note "CLEANUP FAILED: HEAD moved $head_before -> $head_now"; cleanup_rc=8
  fi
  # Undeclared residue left on purpose (exit 5) is NOT a cleanup failure; the
  # declared files still have to come back.
  if [ "$keep_undeclared" -eq 1 ]; then
    # Only the declared files are our business here; undeclared residue is the
    # point of the refusal, not a cleanup failure.
    if ! git -C "$repo" diff --quiet "$head_before" -- "${files[@]}" 2>/dev/null; then
      note "CLEANUP FAILED: declared files were not restored"; cleanup_rc=8
    fi
  else
    local left; left="$(porcelain)"
    if [ -n "$left" ]; then
      note "CLEANUP FAILED: tree not clean after restoring declared files"
      printf '%s\n' "$left" >&2; cleanup_rc=8
    fi
  fi
}
# 130/143 are the conventional codes; kept distinct so a self-test can assert
# WHICH signal was handled, not merely that something interrupted the run.
on_int()  { restore; rm -rf "$work"; exit $(( cleanup_rc ? cleanup_rc : 130 )); }
on_term() { restore; rm -rf "$work"; exit $(( cleanup_rc ? cleanup_rc : 143 )); }
trap on_int INT
trap on_term TERM

finish() {   # $1 = the run's own result code
  local run_rc="$1"
  restore
  rm -rf "$work"
  if [ "$cleanup_rc" -ne 0 ]; then
    note "run result was rc=$run_rc, but cleanup failed -- exiting $cleanup_rc"
    exit "$cleanup_rc"
  fi
  exit "$run_rc"
}

# ---- apply the mutation ----------------------------------------------------
if ! "$@" >"$work/mutate.out" 2>&1; then
  cat "$work/mutate.out" >&2
  note "REFUSING (gate 3): the mutation command failed"
  finish 4
fi

changed="$(porcelain)"
if [ -z "$changed" ]; then
  note "REFUSING (gate 3): the mutation changed nothing -- a survivor here would be meaningless"
  finish 4
fi
if [ -n "$old_once" ]; then
  after="$(occurrences "$old_once")"
  if [ "$after" != "0" ]; then
    note "REFUSING (gate 3): the target literal still occurs $after time(s) -- something changed, but not the mutation under test"
    finish 4
  fi
fi

undeclared="$(undeclared_report)" || undeclared_found=1
if [ "${undeclared_found:-0}" -eq 1 ]; then
  note "REFUSING (gate 4): the mutation touched undeclared path(s). They are LEFT IN PLACE:"
  printf '%s\n' "$undeclared" >&2
  # Contract: declared files come back, undeclared residue is preserved.
  keep_undeclared=1
  finish 5
fi
note "gate 3/4 ok: only declared files changed, and the target literal is gone"

# ---- mutant ----------------------------------------------------------------
read -r mut_sel mut_rc <<<"$(run_tests "$work/mutant.out")"
if [ "$mut_sel" != "$expect_n" ]; then
  note "REFUSING (gate: filter): mutant selected $mut_sel tests, expected $expect_n -- baseline and mutant must judge the same set"
  finish 7
fi

if [ "$mut_rc" -eq 0 ]; then
  note "MUTANT SURVIVED: baseline green and mutant green -- this cell does not discriminate"
  grep -E '^test result:' "$work/mutant.out" >&2 || true
  finish 9
fi
note "caught: mutant is red"
grep -E '^test result:|panicked at|assertion' "$work/mutant.out" | head -6 >&2 || true

# ---- restored phase: put it back, then prove it behaves like the baseline ---
# From head_before, NOT `checkout --`: the latter takes content from the INDEX,
# so a mutation that legally `git add`s a declared file would have the restored
# phase re-test the mutant and report a phantom rc=10 (Tess).
git -C "$repo" checkout "$head_before" -- "${files[@]}" 2>/dev/null || { note "CLEANUP FAILED: could not restore declared files"; cleanup_rc=8; finish 0; }
read -r res_sel res_rc <<<"$(run_tests "$work/restored.out")"
if [ "$res_sel" != "$expect_n" ]; then
  note "RESTORED PHASE: selected $res_sel tests, expected $expect_n -- the tree does not behave like the baseline"
  finish 10
fi
if [ "$res_rc" -ne 0 ]; then
  note "RESTORED PHASE: not green (rc=$res_rc) -- the mutation left a lasting effect"
  grep -E '^test result:' "$work/restored.out" >&2 || true
  finish 10
fi
note "restored ok: $res_sel selected, green"
finish 0
