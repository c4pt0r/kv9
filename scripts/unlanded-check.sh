#!/usr/bin/env bash
#
# Is any comment still claiming that task #N has not landed?
#
#   scripts/unlanded-check.sh --task <N> [--head <rev>] [--repo <dir>]
#
# Exit codes:
#   0  no marker for that task -- the card may close
#   1  usage
#   2  markers remain (listed with exact file:line); the card may not close
#      until they are removed, or transferred by an explicit reviewed decision
#   3  the search instrument failed its own positive control
#
# WHY A MARKER AND NOT A WORDING SEARCH
#
# A comment saying "X is not implemented yet" becomes false the moment X lands,
# and nothing forces anyone back to it. We hit exactly that: a comment telling
# readers to document raw reads as "fresh in normal operation, never strongly
# consistent" survived the barrier landing and was found only because someone
# happened to grep the old promise's phrasing. That was luck -- a different
# original wording would have hidden it.
#
# So the search key is one this project INVENTED. That is what buys a
# zero-invariant: `UNLANDED(task #NNN)` has no legitimate second use, so "zero
# hits" is a criterion. A wording family like "not implemented yet" is content;
# it has endless legitimate uses, so the best it can carry is "every hit is
# classifiable", never "must be zero" (Cindy).
#
# Corollary, and the reason clause 3 of the card forbids regex families here:
# the reach of a wording search equals the set of phrasings the searcher happened
# to think of. The reach of a fixed marker does not depend on the searcher at all.
#
# WHAT THIS DOES NOT DO
#
# It does not find historical stale comments. It is prospective: it can only see
# markers someone wrote. Running it and getting 0 means "no marked debt for this
# task", never "no stale comments about this task". Those are different claims
# and the second one is not available from any search.
set -uo pipefail

task="" ; head_rev="HEAD" ; repo="."
while [ $# -gt 0 ]; do
  case "$1" in
    --task) task="${2:?}"; shift 2 ;;
    --head) head_rev="${2:?}"; shift 2 ;;
    --repo) repo="${2:?}"; shift 2 ;;
    *) printf 'unknown argument: %s\n' "$1" >&2; exit 1 ;;
  esac
done
[ -n "$task" ] || { printf 'usage: %s --task N [--head REV] [--repo DIR]\n' "$0" >&2; exit 1; }
case "$task" in ''|*[!0-9]*) printf 'task must be a number, got: %s\n' "$task" >&2; exit 1 ;; esac

marker="UNLANDED(task #${task})"

# Fixed-string, exact file:line, per clause 3 of the card.
#
# NO SELF-TEST REDS ON THE -F, and I could not write one. With this marker shape
# it is inert: git grep defaults to basic regex, where `(` `)` `#` are all
# literal, so `-F` and the default return the same hits. Measured, not assumed.
# It is kept because the marker format is not frozen -- the day someone writes
# `UNLANDED(task #NNN.1)` or a bracketed variant, the default reading starts
# matching things it should not. Stated here rather than left looking guarded:
# removing it reds nothing today.
hits="$(git -C "$repo" grep -n -F -- "$marker" "$head_rev" 2>/dev/null || true)"

# Positive control: the instrument must be able to find this marker shape at all.
# Without it, "0 hits" is indistinguishable from a search that can never match --
# a broken pattern, the wrong head, an empty repo. The control is built at run
# time so it cannot itself be committed into the corpus being searched.
# Honours TMPDIR rather than hardcoding /tmp: it is the caller's environment to
# choose, and it makes the control's own failure reachable without a test-only
# backdoor -- an unusable TMPDIR is a real way for the probe to be impossible.
probe_dir="$(mktemp -d "${TMPDIR:-/tmp}/unlanded-probe.XXXXXX" 2>/dev/null || true)"
trap 'rm -rf "$probe_dir"' EXIT
if [ -z "$probe_dir" ] || [ ! -d "$probe_dir" ]; then
  printf 'INSTRUMENT FAILED: could not create the positive-control probe.\n' >&2
  printf 'A zero result from this run would mean nothing.\n' >&2
  exit 3
fi
( cd "$probe_dir" && git init -q . >/dev/null 2>&1 \
  && printf '// %s expires when the card closes\n' "$marker" > probe.rs \
  && git add -A >/dev/null 2>&1 \
  && git -c user.email=c@c -c user.name=c commit -q -m probe >/dev/null 2>&1 )
control="$(git -C "$probe_dir" grep -c -F -- "$marker" HEAD 2>/dev/null || true)"
if [ -z "$control" ] || [ "$control" = "0" ]; then
  printf 'INSTRUMENT FAILED: the positive control did not find a known marker.\n' >&2
  printf 'A zero result from this run would mean nothing.\n' >&2
  exit 3
fi

if [ -n "$hits" ]; then
  printf 'task #%s still has %s marked comment(s) at %s:\n' \
    "$task" "$(printf '%s\n' "$hits" | wc -l | tr -d ' ')" "$head_rev" >&2
  printf '%s\n' "$hits" >&2
  printf '\nRemove them, or record an explicit reviewed decision to transfer the debt.\n' >&2
  exit 2
fi
printf 'no %s markers at %s (instrument positive control: ok)\n' "$marker" "$head_rev"
exit 0
