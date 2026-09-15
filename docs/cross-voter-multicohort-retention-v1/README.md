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

Only **one of these 96 cohorts** had run at that checkpoint. Exact stage/verify,
released finish and future restoration/readback commands for all selections are
frozen locally. The [remaining-cohort controller](controller/README.md) subsequently
passed 15 controls and independent review, then completed its
[first actual iteration](controller-execution-001/README.md). It validates each
actual independent verification before passing that hash to finish, serializes
work, preserves partial failures and stops on actual capacity or finite
exhaustion. Original reader use requires restoring original compressed
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

## Continuous execution checkpoint

The controller has completed ordinal 001, a frame-buffer cohort with 216 selected
targets and 355 original objects. Exact restoration and the adapted full reader
pass over all 10,784,920,007 logical bytes, followed by final COLD. Both actual
controller children exit successfully and are reaped. This extends actual reader
qualification beyond the earlier FNV cohort; full-CRC reader execution is still
pending at this checkpoint.

The completed boundary accounts for **two of 96 plan cohorts**, including the
earlier bootstrap, with **4,691,595,264 bytes** of conservative net recovery after
transaction, shared preparation, dispatcher and controller allocation. It excludes
the separate 924,991,488-byte predecessor and subsequent portable publication.
Actual available space at that boundary is **30,024,470,528 bytes**. Ordinal 002
has started under the same live tool session, `79586`; the overall campaign is
not terminal. No new performance result or benchmark-readiness claim follows.

A [later progress snapshot](campaign-progress-20260915.json) records 13 completed
plan cohorts, **29,876,060,160 bytes** of conservative net recovery and
**55,200,915,456 bytes available** at the last completed boundary. Ordinal 013 was
running under the same session at capture. The snapshot binds completed controller
records 001 through 012 and includes the separately qualified bootstrap cohort
000 in its accounting. It does not count the in-progress transaction as recovered
capacity or reexecute payload verification. The overall campaign and the receipt
performance screen remain incomplete.

That controller subsequently exited with failure during ordinal 013: its full
reader confused a reused historical PID with a live producer. The separate
[PID identity repair and actual reconciliation](../retention-pid-identity-v1/README.md)
preserve that failure, pass a fresh complete 344-object readback and final COLD,
and extend the completed prefix to 14 cohorts. Available space immediately after
the new command is 57.387 GB. The old controller remains FAILED. The separately
[qualified continuation](continuation/README.md) starts at ordinal 014 under
actual session 85630 after validating the completed prefix. Its first actual
cohort now passes complete corrected full-CRC readback and final COLD, extending
the prefix to 15 cohorts with 34.087 GB conservative net recovery and 59.401 GB
available at that boundary. It uses the repaired reader authority and a
prospective 85 GB capacity stop, preserving the original plan and benchmark
guards. The original snapshot above remains historical evidence.

The [later continuation snapshot](continuation/progress-20260915.json) advances
the completed prefix to 19 cohorts, with 41.339 GB conservative net recovery
and 66.648 GB available at the latest completed boundary. Ordinal019 was in
finish at capture; no in-progress cohort is counted as recovered capacity.

## Actual coverage of all four reader families

The first completed cohort from each reader family now passes exact restoration,
whole-cohort readback and final COLD. The [bound results](reader-family-qualification.json)
cover the following actual executions, without rerunning their historical
performance workloads:

| Reader family | Ordinal | Restored targets | Whole-cohort objects | Logical bytes read |
| --- | ---: | ---: | ---: | ---: |
| FNV writer | 000 | 226 | 369 | 11,249,173,926 |
| Frame-buffer / CRC | 001 | 216 | 355 | 10,784,920,007 |
| Published directory | 002 | 215 | 353 | 10,709,293,587 |
| Full CRC regression | 003 | 215 | 353 | 10,708,617,059 |
| Total | | 872 | 1,430 | 43,452,004,579 |

The directory reader is byte-identical to its original. The other three execute
the explicitly identified current-floor adaptations, with their original
historical validation intact. This qualifies an actual representative cohort
for each reader path; it does not accept all 96 cohorts or a new performance run.

The previously unreported full-CRC path also passes an
[independent metadata review](reader-family-review.json), `f6ecc1/0`: all stage,
verification, restoration, readback, final-COLD and controller-release hashes
join; all 138 untouched object identities remain; all 215 retired targets are
absent. Its 2,288 actual codec lifetimes, six phase children and two dispatcher
children are successful and no recorded lifetime remains. The current full
readback has 353 decoder receipts. Its inherited `observed.codec_lifetimes=706`
and historical timestamps retain their original producer meaning and are not
current execution counts/times. The initial nonprivileged review read refused
access; privileged read-only inspection resolved that without a workload rerun.

The four-family summary is a metadata extraction (`21b88f/0`), not another
payload verification. Detailed original receipts and payloads remain at its
recorded local paths. No new bytes are credited by publishing this coverage.
