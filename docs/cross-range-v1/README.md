# Cross-range validation packet

See [implementation, chunking semantics and remaining scope](../CROSS-RANGE.md).

- 931 passing workspace tests/doctests in the serial qualifying run, 35
  ignored, zero failures; strict Clippy/formatting pass; 8 real-MinIO
  focused installer tests.
- Eleven checked Lean theorems for the cross-range model with five
  semantic mutation controls and two proof-policy controls; twenty-two
  sibling Lean models and the retention TLA/TLAPS model re-accepted
  against the exact working tree.
- Two accepted five-process e2e runs (`e2e-seventh` covers the exact
  final source after alias cleanups; `e2e-sixth` first ran green) and
  three substantive retained failures: the first exposed the real
  foreign-leader design gap (child groups led by different nodes mean no
  single server owns a span), the fourth exposed the empty-page resume
  gap plain pagination cannot express — both solved by the explicit
  `resume_from` cursors now carried by scan pages and delete-range
  receipts — and the fifth was a harness probe-file sort bug. (Two
  further syntax-abort launches left no artifacts.) The accepted runs:
  the full split-and-retire chain, cross-range scans returning BOTH
  halves' keys in order via resume cursors, bounded scans clamping at
  the boundary, a delete-range clearing keys on both sides with
  per-chunk receipts, and surviving keys intact.

[evidence.tar.gz](evidence.tar.gz) contains **2036 readback-verified
files**, 1041874 compressed bytes and 6846411 decoded bytes. SHA-256:
`96984ad70c6e9b6878b8998ac42444cea1ab63d3e1997ce37e07107ae1bccf27`.
[validation.json](validation.json) inventories all members. A separate
extraction re-verified every member and the e2e/proof/retention receipts;
[portable-readback.json](portable-readback.json) records that result.

Tested source is based on `bd91d67` with the exact current
Rust/Cargo/proto source retained under `source/`.

Nothing here provides cross-range transactions or one-snapshot spans
(per-chunk atomicity is the documented contract), automatic triggers,
physical reclamation, or scaling claims. This increment ran no Chaos
Mesh campaign. The independent 3/6/9-host benchmark contract in
[HORIZONTAL-SCALING-PLAN.md](../HORIZONTAL-SCALING-PLAN.md) remains the
acceptance criterion for effective horizontal scaling. No hosted CI was
dispatched.
