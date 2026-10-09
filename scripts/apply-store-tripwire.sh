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
#
# TEST-ONLY ALLOWANCE (reviewed adjustment, 2026-10-08; the contract above
# said the day a legitimate use appears this check reds and is changed in a
# reviewed diff -- this is that diff)
#
# Snapshot capture/install and checkpoint recovery gained real-MinIO
# INTEGRATION TESTS inside crates/raft. Those tests legitimately construct a
# store; the invariant was always about the PRODUCTION apply path, and the
# production modules still measure zero. The allowance is by EXACT PATH with
# a PINNED per-file count (the manifest-key tripwire's design): a new hit in
# an allowed file, a lost hit, or any hit in any other file reds, so every
# change to the allowed surface shows up in a reviewed diff. No pattern
# allowlist, no "tests are excluded" blanket.
#
# HONEST CAP of the pin for checkpoint_recovery.rs: that file is production
# code whose hits sit inside its #[cfg(test)] module. A text scan cannot see
# module boundaries, so the pin holds the COUNT, not the location within the
# file: a production use added while a test use is simultaneously removed
# would keep the count and pass. The guarantee half remains the capability-
# narrowed apply face; this remains the visibility half.
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

# Exact relative path (under $repo) : pinned hit count. Everything else: zero.
allowed_counts() {
  cat <<'ALLOWED'
crates/raft/src/snapshot_install/capture/tests.rs 2
crates/raft/src/snapshot_install/tests.rs 4
crates/raft/src/state_machine/checkpoint_recovery.rs 2
ALLOWED
}

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

# Classify hits against the allowed exact paths. Any hit outside an allowed
# path is an escape; an allowed path whose count differs from its pin is a
# drift. Either reds. The pins are checked even when the file has ZERO hits:
# a lost pinned hit means the allowed surface changed without review.
escapes=""
drift=""
if [ "$scan_rc" -eq 0 ]; then
  while IFS= read -r hit; do
    [ -n "$hit" ] || continue
    hit_path="${hit%%:*}"
    rel="${hit_path#"$repo"/}"
    if ! allowed_counts | grep -q -F -- "$rel "; then
      escapes="${escapes}${hit}
"
    fi
  done <<EOF_HITS
$hits
EOF_HITS
fi

while read -r rel pinned; do
  [ -n "$rel" ] || continue
  actual=$(grep -c -F -- "$needle" "$repo/$rel" 2>/dev/null)
  case "$actual" in ''|*[!0-9]*) actual=0 ;; esac
  if [ "$actual" -ne "$pinned" ]; then
    drift="${drift}${rel}: ${actual} hit(s), pinned ${pinned}
"
  fi
done <<EOF_ALLOWED
$(allowed_counts)
EOF_ALLOWED

if [ -n "$escapes" ] || [ -n "$drift" ]; then
  if [ -n "$escapes" ]; then
    printf 'ESCAPE: %s named outside the allowed test files:\n' "$needle"
    printf '%s' "$escapes"
  fi
  if [ -n "$drift" ]; then
    printf 'PINNED COUNT CHANGED (reviewed diff required to move a pin):\n'
    printf '%s' "$drift"
  fi
  printf 'Production apply path must not name the object store (task #9, OBJECT-STORAGE §3.1).\n'
  exit 2
fi

allowed_total=$(allowed_counts | awk '{s+=$2} END {print s}')
printf '0 escapes; %s pinned test hit(s) across %s allowed file(s) (positive control: %s hit(s) in %s).\n' \
  "$allowed_total" "$(allowed_counts | wc -l)" "$control_hits" "$control_dir"
exit 0
