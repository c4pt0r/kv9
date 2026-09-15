# First completed continuous-controller iteration

On 2026-09-15 UTC, the frozen controller completes ordinal 001 without a manual
per-cohort release. Its deterministic checks bind the actual stage and independent
verification results before releasing original retirement, exact restoration,
whole-cohort readback and final COLD.

- 216 selected targets restore to their exact former compressed bytes.
- The adapted frame-buffer reader accepts all 355 original objects and
  10,784,920,007 logical bytes with its historical predicates intact.
- Both controller children terminate with exit code 0 and are reaped; all six
  original dispatcher phases have successful terminal records.
- The completed boundary reports two of the fixed 96 cohorts COLD, including
  bootstrap 000. Net recovery is 4,691,595,264 bytes after transaction, preparation,
  dispatcher and controller accounting, excluding the separate predecessor and
  this later publication. Available space is 30,024,470,528 bytes.

`checkpoint.json` preserves the completed-boundary accounting and acceptance
pins. `inventory.json` records exact-copy provenance for the controller release
and completion, child and dispatcher terminals, full original readback result,
final retirement result and root launch records. Original migration payloads and
the remaining per-object migration receipts stay at the recorded local paths;
this reporting snapshot alone cannot restore the WAL objects.

The overall campaign remains live under tool session 79586 and has begun ordinal
002. The retained launch handle is not a successful campaign terminal. Partial or
unknown iterations are preserved and never automatically skipped or restarted.
The 100 GB stop target does not replace the independent benchmark capacity and
correctness guards. Receipt performance remains unmeasured.
