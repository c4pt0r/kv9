# Bounded retention expansion: first larger cohort qualified

On 2026-09-15 UTC, the first cohort in a fixed 96-cohort migration plan passes
exact reconstruction, restoration and complete readback. It adds **2,360,422,400
bytes of conservative net recovery**, after charging its transaction, the shared
preparation and dispatcher output. This excludes the separately qualified
[924,991,488-byte predecessor](../cross-voter-cohort-retention-v1/README.md).
Portable publication and subsequent controller preparation have additional overhead.

| Completed cohort 000 | Actual result |
| --- | ---: |
| Selected n2/n3 engine segments | 226 |
| Original compressed objects in the whole cohort | 369 |
| Original logical bytes checked by complete readback | 11,249,173,926 |
| Original compressed bytes checked | 7,961,430,085 |
| Selected original allocation | 2,651,586,560 bytes |
| Patch bytes, with n1 bases retained separately | 228,496,269 |
| Final transaction allocation | 276,627,456 bytes |
| Shared preparation / dispatcher allocation | 14,376,960 / 159,744 bytes |
| Successful, exited codec lifetimes | 2,403 |
| Preserved objects with unchanged complete hashes | 143 |
| Newly executed focused controls | 12 |

Available disk space at the final audit was **27,713,499,136 bytes** before
publication overhead. The receipt write screen remains unmeasured and needs an
estimated 80–100 GB available. No database code, benchmark workload, Raft quorum,
sync, publication or acknowledgment fence changed in this stage.

## What changed in the test environment

The old FNV, frame-buffer and full-CRC retention readers require 48 or 96 GiB
free during current decoding. Those operational floors prevent incremental
capacity recovery even though decoding is bounded and streamed. Separate reader
copies now use an 8 GiB **current decode** floor. Their original `LIMITS`, storage
policy and hash, historical resource samples, catalog equality, scope, child,
time, exact-object and complete-decoding checks remain intact. Independent source
comparison confirms only the live `full_decode` space expression and its new
constant/comment differ. The original readers and original campaign acceptance
are preserved. The directory reader remains unchanged.

Ten actual focused tests cover the floor boundary, rejection of wrong hashes,
lengths, decoder failure and timeout, historical budget validation, path/policy
checks, finite selection, external restoration allocation, interrupted decode
charging and actual selected-reader binding. Two dispatcher controls cover
refusal to replay staging after retirement begins, including a dangling symlink,
and ensure this check precedes dispatch. All 110 tested source inputs remain
unchanged at execution. The predecessor's 19 controls remain separate evidence.

The 96 exact selections cover 6,681 targets with an allocation ceiling of
78,026,149,888 bytes. A fresh metadata check confirms all 12,906 original objects
match their recorded identities, lengths and allocation; independent review
finds no cross-cohort aliases or base also selected for retirement. This ceiling
is not predicted or reclaimed capacity. The larger cohort's 228 MB patch result
also shows why the earlier small-pilot ratio cannot be applied to the whole pool.

Each cohort has an explicit coexistence reservation, up to 7 GiB total allocation,
70 GiB cumulative decoding and 112,197,632 bytes of metadata for the largest
selection. Restored objects outside the transaction directory remain charged;
failed or interrupted decoding remains reserved. The 8 GiB live floor, 512 MiB
codec address space and 128 MiB migration-output bound remain. Complete historical
readback keeps its original larger member bounds. These prospective policies
do not alter any previously recorded benchmark requirement.

## Actual qualification and remaining work

Cohort 000 is FNV timing `008-new-batch64-r000-p10064`. Stage and independent
verification pass with the retained encoder/loader/libraries. Root then releases
the exact verified hash, retires only those targets, restores every exact former
compressed byte to its original path, reads all 369 objects through the explicitly
identified adapted reader, and separately releases final COLD. All metadata
remains unchanged. New inode/ctime after restoration is recorded honestly.
Independent final audit checks every preserved object's full hash, all 2,403
recorded codec lifetimes and actual allocation. Cumulative decoded charging is
33,696,395,574 bytes. The actual stage/verify and finish terminals are
`52821/11fb5e/0` and `56949/ba03d0/0`; final audit is `88896/109415/0`.

Only **one of these 96 cohorts** has run at this checkpoint. Exact stage/verify,
released finish and future restoration/readback commands for all selections are
frozen locally. A controller for the remaining 95 cohorts is under review; it
must validate each actual independent verification before passing that hash to
finish, serialize work, preserve partial failures and stop on actual capacity
or finite exhaustion. Original reader use requires restoring original compressed
paths first. Final restoration/readback commands are prepared, not claimed run.

The local preparation is
`/tmp/kv9-cross-voter-multicohort-migration-preparation-20260915-first`;
the qualified transaction is
`/tmp/kv9-cross-voter-group-000-fnv-writer-20260915-first`.
The 100 GB stop target is a planning margin, not a replacement benchmark guard;
finite exhaustion without reaching it must be reported honestly and capacity
must be requalified for the actual receipt screen.

`original-evidence.tar.gz` preserves 10,491 source/reporting files totaling
52,461,095 bytes, including the original refusals, controls, source review,
independent metadata review, phase receipts, original cohort reports and final
audit. Run `python3 -B docs/cross-voter-multicohort-retention-v1/verify.py` to read
all bytes through gzip EOF and independently check scope, lineage and accounting.
It does not extract or execute archived code. Original base objects, patches and
retained executable payloads stay local; this reporting archive alone cannot
restore the WAL objects. This is not a power-loss or database performance test.

CI stays local. No original industrial roadmap checkbox closes.
