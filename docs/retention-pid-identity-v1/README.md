# Retention reader PID reuse repair and actual cohort reconciliation

On 2026-09-15 UTC, the interrupted ordinal-013 transaction completes a fresh
whole-cohort readback and final COLD transition, extending the completed prefix
to **14 of the original 96 cohorts**. The new command succeeds
(`56762/364874/0`); the original campaign remains FAILED (`79586/ce8c44/1`).
Neither its failed reader result nor its controller status is rewritten.

| Actual corrected ordinal 013 | Result |
| --- | ---: |
| Previously restored selected targets, retired after readback | 209 |
| Preserved original objects | 135 |
| Objects checked by the complete corrected reader | 344 |
| Original logical bytes checked | 10,402,566,933 |
| Original compressed bytes checked | 7,362,155,494 |
| Fresh successful readback decoder receipts | 344 |
| Cumulative decoding, including the failed readback | 41,558,085,954 bytes |
| Unchanged cumulative decoding cap | 69,793,218,560 bytes |
| Available space immediately after the command | 57,386,516,480 bytes |

The receipt-tail write candidate remains unmeasured; CRC remains selected.

The [independent actual acceptance](independent-acceptance.json) also passes
(`39909/f0df7e/0`): every retired target is absent, all 135 preserved objects
match complete hashes, and all 2,569 fresh codec lifetimes (including the failed
reader's decoders) and 688 historical producer lifetimes are gone. The complete
14-cohort prefix accounts for **32,065,552,384 bytes** conservative net recovery,
including old controller, failure, repair and new execution metadata plus a
4 MiB audit reservation, before subsequent publication/continuation overhead.

## Cause and repair

The old reader tested whether `/proc/<historical producer pid>` existed.
After decoding all objects, its own Python process reused exited producer PID
**1272867**. On the same boot, the historical start time was **171408954** ticks
while the new reader's was **174257498**. The bare-PID predicate therefore
rejected an unrelated process as a live producer.

All four reader families now have separately bound derivatives comparing the
recorded **boot ID, PID and start time**. The exact live lifetime still causes
refusal. Missing, malformed, unreadable or ambiguous identity evidence fails
closed. All other ownership, exit, catalog, historical budget, hash, full-decode
and resource predicates stay unchanged. Source-inverse controls reproduce each
original reader exactly after removing this one change. Original sources and
the failed attempt remain in place.

The reader repair passes **11 focused metadata controls**, including the actual
self-PID collision. The reconciliation adapter passes **11 controls** and
**five restoration checks**. Independent review caught an initially unreachable
final restore command; the correction makes exact restore-2 and readback-2
reachable under the corrected authority. Both preparation revisions and their
control results are preserved. Runtime acceptance comes from the new actual
readback, not the metadata controls.

## Actual recovery and remaining capacity work

The new command reuses the successful first retirement and restoration. It
writes readback into a fresh directory, retains the failed readback's full
decoding charge, and binds the new success receipt before final retirement.
Original selections, verification, retained toolchain, collision checks,
object hashes and restoration reservations remain in force. Future restore-2
and readback-2 commands are prepared; they were not executed. No third retirement
is supported.

The next continuation starts at ordinal 014 and must validate the completed
original prefix and this actual reconciliation. Its prospective **85 GB
available-space stop** changes capacity planning only. The original plan's
100 GB target remains historical evidence. The receipt screen's empirical
launch budget is **79,455,850,496 bytes**; fresh capacity and exclusive execution
remain required before all eight smokes and sixteen timed cohorts. Their
workload and storage guards are unchanged.

## Retained evidence

`original-evidence.tar.gz` preserves **157 exact files / 5,489,181 bytes**:
both frozen preparations, control revisions, original failure reporting,
actual command/terminal and verification, restoration, readback and COLD results.
`inventory.json` records original paths, lengths and SHA-256 values. Full gzip
EOF and every member's bytes/hash were checked after creation (`8aac00/0`).
Readable files in `source/` are byte-identical archive copies.

`independent-acceptance.tar.gz` separately preserves all five independent audit
files, totaling 1,008,625 original bytes, including its source, complete result
and actual terminal. Its complete archive readback also passes (`a7be62/0`).

This reporting archive does not replace the local payloads, bases, patches or
executable toolchain. Sources retain historical absolute runtime bindings and
are not a portable installed utility. Original transaction and phase receipts
remain under `/tmp/kv9-cross-voter-group-013-fnv-writer-20260915-first` and
`/tmp/kv9-cross-voter-multicohort-execution-20260915-first/013`.

No Raft quorum, sync, publication or acknowledgment rule changed. This repair
establishes no database QPS, power-loss acceptance or industrial checklist
completion. CI remains local.
