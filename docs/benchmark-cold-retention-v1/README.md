# Completed benchmark WAL cold retention

**552 WAL files are now COLD; the one-file pilot is RESTORED.** This overlay supersedes older descriptions that all selected WALs remain at their original paths. Original accepted audits, inventories, retention records and histories remain unchanged. Their original-run acceptance is historical; running those full audits again requires prior rehydration.

Root completed all original transactions before the subsequent ThinLTO smoke and 24-cohort timing. This report reads only metadata and performs file existence/allocation observations; it does not reread or reverify any payload.

| Transaction group | Cold files | Cold logical bytes | Net allocated savings, excluding transaction metadata |
| --- | ---: | ---: | ---: |
| Original batches 00–08 | 409 | 9,128,520,780 | 3,213,029,376 bytes |
| Supplemental batches 00–02 | 143 | 3,112,819,202 | 1,082,044,416 bytes |
| Retained object for restored pilot | 0 | 0 | −2,220,032 bytes |
| Total | 552 | 12,241,339,982 | **4,292,853,760 bytes (3.998032 GiB)** |

The accounting subtracts current recorded compressed-object/staging allocation from original allocated source bytes, counting each object/staging inode once, and includes the restored pilot’s remaining object cost. Transaction catalogs, journals, receipts, directories and earlier synthetic/sample artifacts are excluded. The observed filesystem free-space series is not a clean savings measurement: source-cache growth and later ThinLTO outputs also consumed space.

## Preserved provenance and failures

All **66** copied controller-stage receipts are complete and exit 0. The pilot restored **3,449,270 bytes**, original SHA `d15f523ddb830d528402fa466a0e377da56feafcd2fe02e3a881201cc18f3288`. Its final RESTORED catalog SHA is `6b9021813c97aeeda20b204a41e965b4ce1fd8f9c0d5f6a6c035899bd4f4792e`. Root’s successful unchanged restore and retained decoder records establish the byte restoration; this publication performed metadata readback only. No original inode identity is claimed.

The separate root authorization attempt **fba658 / exit 1** is retained: a catalog digest was miscopied with duplicated characters, and the hash assertion rejected it before mutation. Root corrected only that reviewed argument; supplemental batch 02 then completed, with last root receipt **39d2a3 / exit 0**. See the exact `rejected-root-release-argument.json` copy and successful child receipts. This failure is not erased or relabeled.

The first ordinary-user metadata directory listing also encountered root-owned directory permissions (tool b3103e / exit 1); subsequent publication reads used sudo. No original transaction was repeated for that read-only inspection.

Both frozen controllers use the unchanged **96 GiB (103,079,215,104-byte)** phase floor. No benchmark storage guard or validation predicate was lowered. All 552 COLD source paths were absent during this metadata readback. The pilot source was present with its recorded restored metadata.

## Rehydration before old full audits

Exact controller-pinned argv for every transaction are in [restore-commands.json](restore-commands.json). Use original controller `42f3b410…` for original batches and pilot, and supplemental controller `859ff219…` for supplemental batches. Their different hash identities deliberately prohibit catalog migration; preserve both helpers and all verified objects. No restore is executed by this publication.

| Original accepted campaign | Cold files to restore | Additional uncompressed bytes |
| --- | ---: | ---: |
| `kv9-coalesced-owner-screen-timing-first` | 205 | 4,606,022,459 |
| `kv9-peer-executor-isolation-screen-timing-first` | 204 | 4,522,498,321 |
| `kv9-inbox-vector-reuse-screen-timing-first` | 143 | 3,112,819,202 |

Simultaneous rehydration of all cold members requires up to **12,241,339,982 additional uncompressed bytes**, beyond retained compressed objects, the 96 GiB floor and metadata overhead. Restore only when that capacity is available, and restore every cold member needed by a campaign before invoking its unchanged auditor into a fresh output directory. Restored paths and bytes match the original inventory; inode, ctime, birth time and physical allocation are not reproduced. A successful archival operation does not guarantee future simultaneous restoration capacity.

Cold objects are local on the same filesystem, not an independent backup failure domain. No cleanup or object garbage collection is authorized by this report.

The first publication builder stopped with a metadata-schema KeyError (tool 6b6df1 / exit 1): original batch argv are nested while supplemental argv are direct. That draft and its partial metadata copies remain in `first-attempt/`; only the argv lookup was corrected. No payload operation or transaction was rerun.

## Metadata publication selection

`metadata-inventory.json` maps each original metadata path to its byte count, SHA-256 and exact copied path under `metadata/`. It includes frozen controller/test/readiness/command pins, current catalogs and selections, original acceptance metadata, root-reviewed/consumed releases, journals, root invocation/outcome logs and the preserved wrong-hash rejection. No WAL, compressed object, native binary or full host-process dump is packaged. `outcome.json` contains per-batch arithmetic and exact catalog/controller hashes.

The publication inventory authenticates the copied metadata only. Compressed-payload hashes recorded inside the original verified catalogs remain historical verification evidence; they were not recomputed by this metadata report.
