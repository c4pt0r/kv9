#!/usr/bin/env bash
#
# Does the manifest pair's key prefix appear outside its owning module?
#
#   scripts/manifest-key-tripwire.sh [--repo <dir>]
#
# Exit codes:
#   0  the prefix tokens appear ONLY in the owning file, at the expected
#      definition count
#   1  usage
#   2  the prefix escaped (listed with exact file:line), or the definition
#      count in the owning file changed
#   3  the search instrument failed (positive control dead, scan error,
#      owner missing) — NEVER reported as clean
#
# WHAT THIS GUARDS (task #9, precondition P4 — and what it does NOT)
#
# The authoritative (generation, last_change_id) pair may be written only by
# the ManifestChange CAS arm in state_machine.rs. That is currently a
# MEASUREMENT (one enumerated production writer), not a type-level
# guarantee: the key is ordinary bytes, and any holder of ApplyStore::write
# can reach the same physical key. This check is a RESIDENT VISIBILITY
# GUARD, not a guarantee (ruling: Tess, after Cindy's tier correction): it
# reds when the prefix token spreads to another file or when a second
# in-owner definition appears, so the common escapes show up in a diff.
#
# EXCLUSION IS BY EXACT PATH, NOT SUBSTRING (review round 3, Tess): the
# first version excluded hits with `grep -v "state_machine.rs"`, and a probe
# file literally named `not_state_machine.rs` carried the escaped prefix
# straight through to a green. Hits are compared against the one exact
# owner path; nothing else is excluded.
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

owner_rel="crates/raft/src/state_machine.rs"
owner="$repo/$owner_rel"
scan_root="$repo/crates"
# The key-LITERAL tokens (the distinctive spelling inside the byte-string
# prefixes), one per key family — NOT the identifier family, which the read
# API legitimately carries through other modules. Expected in-owner counts
# are pinned so a SECOND in-owner literal (a new writer path) also reds.
tokens="x00manifest_pair x00manifest_gen"
expected_in_owner=2

[ -f "$owner" ] || { printf 'INSTRUMENT FAILED: %s missing.\n' "$owner" >&2; exit 3; }
[ -d "$scan_root" ] || { printf 'INSTRUMENT FAILED: %s is not a directory.\n' "$scan_root" >&2; exit 3; }

fail=0
total_owner=0
for tok in $tokens; do
  # Positive control per token: the instrument must hit the needle where it
  # legitimately lives, else every tree scans "clean".
  in_owner=$(grep -c -F -- "$tok" "$owner") || true
  case "$in_owner" in ''|*[!0-9]*) in_owner=0 ;; esac
  if [ "$in_owner" -lt 1 ]; then
    printf 'INSTRUMENT FAILED: token %s not found in %s — scan cannot be trusted.\n' \
      "$tok" "$owner" >&2
    exit 3
  fi
  total_owner=$((total_owner + in_owner))

  # Scan errors must fail closed: capture rc BEFORE any filtering, never
  # pipe the scan into a filter that folds a tool error into zero hits.
  scan_out="$(mktemp "${TMPDIR:-/tmp}/mkt-scan.XXXXXX")"
  grep -r -n -F -- "$tok" "$scan_root" >"$scan_out" 2>&1
  rc=$?
  if [ "$rc" -gt 1 ]; then
    printf 'INSTRUMENT FAILED: scan for %s errored (grep rc=%s).\n' "$tok" "$rc" >&2
    cat "$scan_out" >&2
    rm -f "$scan_out"
    exit 3
  fi
  # Exact-path exclusion: keep only hits whose file path differs from the
  # one owner path.
  escapes=""
  while IFS= read -r hit; do
    [ -n "$hit" ] || continue
    hit_path="${hit%%:*}"
    if [ "$hit_path" != "$owner" ]; then
      escapes="${escapes}${hit}
"
    fi
  done <"$scan_out"
  rm -f "$scan_out"
  if [ -n "$escapes" ]; then
    printf 'PREFIX ESCAPED: %s appears outside the owning file:\n' "$tok"
    printf '%s' "$escapes"
    fail=1
  fi
done

if [ "$total_owner" -ne "$expected_in_owner" ]; then
  printf 'IN-OWNER COUNT CHANGED: %s total hits for {%s} in %s (expected %s).\n' \
    "$total_owner" "$tokens" "$owner" "$expected_in_owner"
  printf 'A new in-owner use may be a second writer path — reviewed diff required.\n'
  fail=1
fi

if [ "$fail" -ne 0 ]; then
  printf 'The manifest pair has ONE writer (the CAS arm); see P4 in task #9.\n'
  exit 2
fi

printf '0 escapes; %s in-owner hits for {%s} (expected %s).\n' \
  "$total_owner" "$tokens" "$expected_in_owner"
exit 0
