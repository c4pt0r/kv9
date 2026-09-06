#!/usr/bin/env bash
#
# Does the apply-side crate reference the object store at all?
#
#   scripts/apply-store-tripwire.sh [--repo <dir>]
#
# Exit codes:
#   0  zero references -- the invariant's name-level face holds
#   1  usage
#   2  references present (listed with exact file:line)
#   3  the search instrument failed its own positive control
#
# THE INVARIANT THIS GUARDS (task #9; contract in docs/OBJECT-STORAGE.md §3.1)
#
# Apply never touches the ObjectStore. Uploads complete durably BEFORE a
# ManifestChange is proposed; the ordered apply installs already-durable
# references and advances the watermark. Deadline and blocking risk stay on
# the engine's upload thread; the pump thread never waits on remote storage.
#
# WHY A CONTENT-WORD ZERO IS A CRITERION HERE, WHEN ELSEWHERE IT IS NOT
#
# unlanded-check.sh explains why a wording family can never carry "must be
# zero" -- content words have endless legitimate uses. This check is the
# narrow exception that proves that rule: the SCOPE makes the count a
# criterion, not the word. Within crates/raft there is no legitimate use of
# the identifier `ObjectStore` today (measured at introduction: engine 17,
# raft 0, all other crates 0). The day a legitimate use appears, this check
# reds and must be changed in a reviewed diff -- that visibility is the whole
# point. Weakening the tripwire cannot happen silently.
#
# WHAT THIS DOES NOT CATCH (honest cap -- do not widen claims from a green)
#
# Indirect reach: a helper that already holds a store, reached through a
# generic bound or a glob import, never spells the name and passes this scan.
# The name-level zero is the TRIPWIRE half of the invariant; the guarantee
# half is the capability-narrowed apply face (this card's later commits),
# under which apply-side types cannot hold a store regardless of naming.
# Green here means "nobody wrote the name", never "nobody can reach a store".
set -uo pipefail

repo="."
while [ $# -gt 0 ]; do
  case "$1" in
    --repo) repo="${2:?}"; shift 2 ;;
    *) printf 'unknown argument: %s\n' "$1" >&2; exit 1 ;;
  esac
done

needle="ObjectStore"
scan_dir="$repo/crates/raft/src"
control_dir="$repo/crates/engine/src"

[ -d "$scan_dir" ] || { printf 'INSTRUMENT FAILED: %s is not a directory.\n' "$scan_dir" >&2; exit 3; }

# Positive control FIRST: the instrument must be able to find the needle where
# it legitimately lives. A scan that cannot hit anything reports every tree as
# clean -- an instrument failure dressed as a pass.
control_hits=$(grep -r -n -F -- "$needle" "$control_dir" 2>/dev/null | wc -l)
if [ "$control_hits" -lt 1 ]; then
  printf 'INSTRUMENT FAILED: positive control found 0 hits for %s under %s.\n' \
    "$needle" "$control_dir" >&2
  printf 'This is NOT a clean result; the scan cannot be trusted.\n' >&2
  exit 3
fi

# The target scan's exit code is load-bearing: grep rc 0 = matched,
# 1 = legitimately no match, >1 = the scan could not be performed.
hits="$(grep -r -n -F -- "$needle" "$scan_dir" 2>&1)"; scan_rc=$?
if [ "$scan_rc" -gt 1 ]; then
  printf 'INSTRUMENT FAILED: could not scan %s (grep rc=%s).\n' "$scan_dir" "$scan_rc" >&2
  [ -n "$hits" ] && printf '%s\n' "$hits" >&2
  exit 3
fi

if [ "$scan_rc" -eq 0 ]; then
  count=$(printf '%s\n' "$hits" | wc -l)
  printf '%s reference(s) to %s under %s:\n' "$count" "$needle" "$scan_dir"
  printf '%s\n' "$hits"
  printf 'Apply-side crate must not name the object store (task #9, OBJECT-STORAGE §3.1).\n'
  exit 2
fi

printf '0 reference(s) to %s under %s (positive control: %s hit(s) in %s).\n' \
  "$needle" "$scan_dir" "$control_hits" "$control_dir"
exit 0
