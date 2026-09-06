#!/usr/bin/env bash
#
# Does the manifest pair's key prefix appear outside its owning module?
#
#   scripts/manifest-key-tripwire.sh [--repo <dir>]
#
# Exit codes:
#   0  the prefix tokens appear ONLY in the owning module, at the expected
#      definition count
#   1  usage
#   2  the prefix escaped (listed with exact file:line), or the definition
#      count in the owning module changed
#   3  the search instrument failed its own positive control
#
# WHAT THIS GUARDS (task #9, precondition P4 — and what it does NOT)
#
# The authoritative (generation, last_change_id) pair may be written only by
# the ManifestChange CAS arm in state_machine.rs. That is currently a
# MEASUREMENT (one enumerated production writer), not a type-level
# guarantee: the key is ordinary bytes, and any holder of ApplyStore::write
# can reach the same physical key. This check is a RESIDENT VISIBILITY
# GUARD, not a guarantee (ruling: Tess, after Cindy's tier correction): it
# reds when the prefix token spreads to another module or when a second
# in-module definition appears, so the common escapes show up in a diff.
#
# HONEST CAP — this is a TEXT match. It cannot catch the same physical key
# assembled differently: split literals (b"manifest_" followed by "pair"),
# concat!, byte arrays, or runtime concatenation all produce the identical
# key without ever spelling the token below. A real guarantee needs a
# closed key/capability design at the engine boundary; until then, green
# here means "nobody wrote the token elsewhere", never "nobody can write
# the key".
set -uo pipefail

repo="."
while [ $# -gt 0 ]; do
  case "$1" in
    --repo) repo="${2:?}"; shift 2 ;;
    *) printf 'unknown argument: %s\n' "$1" >&2; exit 1 ;;
  esac
done

owner="$repo/crates/raft/src/state_machine.rs"
scan_root="$repo/crates"
# The key-LITERAL tokens (the distinctive spelling inside the byte-string
# prefixes), one per key family — NOT the identifier family, which the read
# API legitimately carries through other modules. Expected in-owner counts
# are pinned so a SECOND in-module literal (a new writer path) also reds.
tokens="x00manifest_pair x00manifest_gen"
expected_in_owner=2

[ -f "$owner" ] || { printf 'INSTRUMENT FAILED: %s missing.\n' "$owner" >&2; exit 3; }

fail=0
total_owner=0
for tok in $tokens; do
  # Positive control per token: it must hit in the owner at all.
  in_owner=$(grep -c -F -- "$tok" "$owner") || true
  if [ "$in_owner" -lt 1 ]; then
    printf 'INSTRUMENT FAILED: token %s not found in %s — scan cannot be trusted.\n' \
      "$tok" "$owner" >&2
    exit 3
  fi
  total_owner=$((total_owner + in_owner))

  escapes="$(grep -r -n -F -- "$tok" "$scan_root" 2>&1 | grep -v -F "state_machine.rs")"; rc=$?
  if [ "$rc" -eq 0 ]; then
    printf 'PREFIX ESCAPED: %s appears outside the owning module:\n' "$tok"
    printf '%s\n' "$escapes"
    fail=1
  elif [ "$rc" -gt 1 ]; then
    printf 'INSTRUMENT FAILED: scan for %s errored (rc=%s).\n' "$tok" "$rc" >&2
    exit 3
  fi
done

if [ "$total_owner" -ne "$expected_in_owner" ]; then
  printf 'IN-OWNER COUNT CHANGED: %s total hits for {%s} in %s (expected %s).\n' \
    "$total_owner" "$tokens" "$owner" "$expected_in_owner"
  printf 'A new in-module use may be a second writer path — reviewed diff required.\n'
  fail=1
fi

if [ "$fail" -ne 0 ]; then
  printf 'The manifest pair has ONE writer (the CAS arm); see P4 in task #9.\n'
  exit 2
fi

printf '0 escapes; %s in-owner hits for {%s} (expected %s).\n' \
  "$total_owner" "$tokens" "$expected_in_owner"
exit 0
