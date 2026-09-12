# Historical owner-poll recovery evidence

The exact original archive is retained in the already-published commit
[`928b2699751416414a7fc6005ee219431c596e53`](https://github.com/c4pt0r/kv9/commit/928b2699751416414a7fc6005ee219431c596e53):

- [Original evidence archive](https://github.com/c4pt0r/kv9/blob/928b2699751416414a7fc6005ee219431c596e53/docs/owner-poll-recovery-v1/original-evidence.tar.gz)
- [Exact-byte download](https://raw.githubusercontent.com/c4pt0r/kv9/928b2699751416414a7fc6005ee219431c596e53/docs/owner-poll-recovery-v1/original-evidence.tar.gz)
- Bytes: **7,989,590**
- SHA-256: **8dd36db2191f2d06111b76197a59152118cfd115f6377ab8fb0da74074114a9e**

Only the duplicate archive file in current main was removed. The original
[223-member archive inventory](archive-inventory.json), [validation summary](summary.json),
historical Git blob and original local experiment artifacts remain unchanged.
[archive-location.json](archive-location.json) binds the exact commit, path,
Git object, length and hash. This is build/recovery evidence for the owner-poll
candidate that was subsequently [rejected on performance](../OWNER-POLL-PERFORMANCE.md),
not current lease implementation evidence or a new validation result.

To recover the archive, use the pinned download above or retrieve the same
blob from a local repository that contains the commit:

```sh
git show 928b2699751416414a7fc6005ee219431c596e53:docs/owner-poll-recovery-v1/original-evidence.tar.gz > /fresh/external/directory/original-evidence.tar.gz
```

Use a fresh destination outside the current source tree; do not overwrite an
existing artifact. Require the exact byte count and SHA-256 above before
opening it. For extraction, use a fresh directory and verify every member's
name, size and SHA-256 against the unchanged archive inventory. Historical
process re-auditing still requires its original source/build/run inputs and
environment; this archive location change does not replace those predicates
or claim a complete clean-machine replay.

The source inventory's existing 32 MiB per-archive and 64 MiB aggregate
allowances are unchanged. Older build/source manifests retain their original
identities; subsequent clean builds bind the new source tree normally. No
archive was re-encoded, no measurement or audit was rerun, and no remote
history was rewritten for this relocation.
