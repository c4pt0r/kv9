# Local test capacity recovery

On 2026-09-15 UTC, safe cache retirement and lossless diagnostic retention
recovered enough measured free space for the next complete matched write
screen. No database runtime, durability rule or benchmark workload changed.

| Completed action | Conservative net recovery |
| --- | ---: |
| Retire 39,303 exact inactive build-cache files | 6,118,625,280 bytes |
| Archive 309 files from two completed diagnostic profiles | 2,398,953,472 bytes |
| Total, excluding earlier cleanup | 8,517,578,752 bytes |

Cache retirement preserved invalidation metadata before unlinking, held the
existing Cargo/build locks, checked process references and verified all ten
protected executable hashes before and after. Already absent or changed files
received no recovery credit. Sources, offline dependencies and retained
executables were preserved. Its actual terminal is `59306/d979d9/0`.

The two profile transactions retain all 8,231,119,541 original logical bytes
in checksummed compressed objects. Separate decoders read every byte and
matched the original hashes before exact per-transaction eviction. Both
transactions are complete (`36702/3a84ce/0`, `53154/a0dfe7/0`); all 309 absent
original paths have matching COLD catalog entries. Accounting charges the
entire new profile output tree, including controls, metadata and staging,
against recovered allocation. An accounting helper rejected its own symlink
negative-test fixtures; that failure is preserved. Separate physical
accounting counts link allocation without following links and avoids counting
hardlinks twice. Restore is exclusive and preserves file contents, mode,
ownership and timestamps; it does not preserve inode/ctime identity. Readers
requiring the original raw paths must restore the complete relevant profile.

The final capacity observation was **79,727,611,904 bytes available**, against
the prospective storage-v3 empirical launch budget of **79,455,850,496 bytes**.
Further profile migration stopped after this observation. The budget includes
52,612,304,896 historically observed retained bytes plus a 25 GiB operating
envelope. It is an estimate, not a maximum-output guarantee; fresh launch and
continuous space checks remain mandatory, and any failed cohort is retained.
No reserved filesystem blocks or assumed compression savings count as free
space. The runtime floor, retention floor and maximum fresh-restore allocation
are distinct requirements. Passing capacity alone does not accept a benchmark.

Original catalogs, independent readbacks, exact release records and actual
terminals remain under the local
`/tmp/kv9-published-directory-capacity-actions-preparation-20260915-first/`
tree. This note does not claim a portable copy of those diagnostic WAL files.
All CI and test work remains local; hosted workflows are manual only.

## Subsequent receipt-screen preparation

The completed directory comparison consumed its retained space; the earlier
79.73 GB observation is not current capacity. The next receipt comparison has
about 25 GB available and an estimated 80–100 GB launch budget.

A [six-member cross-voter WAL patch pilot](cross-voter-retention-pilot-v1/README.md)
now passes complete original-hash reconstruction and eight refusal controls.
Including the retained base, its two selected triplets reduce ordinary
compressed sizes by 63.404% and 66.568%. These are sample results, not reclaimed
space. No original object or catalog was retired. A follow-up also reconstructs
the exact six old compressed-object identities and independently decodes their
complete logical bytes. These two original pilots reclaimed no bytes.

The subsequent [whole-cohort migration](cross-voter-cohort-retention-v1/README.md)
now passes 19 controls, all 80 target reconstructions, exact original-path
restoration and the unchanged reader over all 150 original objects. The final
COLD transaction recovers **924,991,488 allocated bytes** after charging all
transaction output; portable reporting has separate overhead. All 70 preserved
objects retain their full hashes, original metadata is unchanged, and all 870
fresh codec lifetimes have exited. Available space at its final audit was
25,352,613,888 bytes, before publication overhead. This remains below the next
complete receipt-screen estimate. Extend migration in bounded groups using
actual capacity, with separately reviewed live readback-floor adaptations for
older campaign readers and all historical validation preserved.

The [first larger cohort](cross-voter-multicohort-retention-v1/README.md) now also
passes: 226 exact target restores, complete readback of 369 objects / 11.249 GB
logical bytes, all 143 preserved object hashes unchanged, and all 2,403 codec
lifetimes exited. It adds **2,360,422,400 bytes** of net recovery after transaction,
shared preparation and dispatcher allocation; publication has separate overhead.
Its final audit observes 27,713,499,136 bytes available. Twelve focused controls
and independent review qualify the finite 96-cohort extension. Older reader
copies change only the current decode floor to 8 GiB; their original historical
checks remain intact. At that checkpoint only cohort 000 had run. The 100 GB
stop target is a planning margin and does not replace the unchanged benchmark guards.

The [continuous controller](cross-voter-multicohort-retention-v1/controller/README.md)
subsequently passes 15 controls and independent review, then completes
[ordinal 001](cross-voter-multicohort-retention-v1/controller-execution-001/README.md):
216 exact restorations and full readback of 355 frame-buffer objects / 10.785 GB
logical bytes. Two of the 96 plan cohorts are now complete. Their conservative
combined net recovery is **4,691,595,264 bytes**, including controller allocation
and excluding the separate predecessor and later publication. The completed
boundary observes **30,024,470,528 bytes available**. Ordinal 002 has started;
the overall run and receipt performance remain pending. Restoration temporarily
consumes space again, so intermediate free-space peaks are not net recovery.
