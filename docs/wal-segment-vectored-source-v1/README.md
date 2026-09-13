# Vectored segmented-WAL source evidence

The [source report](../WRITE-SEGMENT-VECTORED.md) describes the change, proof
premises and remaining release/recovery/Chaos/performance gates.

`original-evidence.tar.gz` retains **65 original files / 784,944 decoded bytes**
in **159,459 compressed bytes**, SHA-256:

```text
8bbc9f705dfc5bf6429260a099098cbcd5535199374650ca68f92ba68af30d05
```

`inventory.json` maps original absolute paths to exact sizes and hashes. Archive
paths use `original/` followed by the original path without its leading slash.
Extract into a fresh review directory. The records retain their host paths.

- Proof terminal `e0624f/0`: three universal SMT checks and three countermodels;
  exact segment-module change and helper hash bound to selected `11113f6`.
- First source terminal `072087/1`: formatting failure before compilation/tests.
- Corrected source terminal `78636/e08c43/0`: 714 tests/doctests, 23 existing
  ignored, formatting, Clippy and fresh first-party Cargo artifacts. Source
  snapshots are unchanged during the successful gate.
- Syscall terminal `082a89/0`, readback `8d19b2/0`: three frame `writev` calls,
  each immediately followed by successful `fsync`; the existing replay test
  succeeds. Its retained local test executable SHA-256 is
  `9f65816f0ee3d1b59c6d31aff5a962cbf6cdfdc3865230857224abb6dd0aa655`.

The 98,697,176-byte debug test executable and tiny WAL fixture remain local and
are excluded from this metadata archive. This archive supports review of the
recorded qualification; rebuilding requires the candidate source and toolchain.
It is not a release artifact or an actual Chaos/performance acceptance record.
Independent archive readback is in `readback.json`.
