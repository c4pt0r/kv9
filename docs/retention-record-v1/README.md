# Retention record component evidence

See [the implementation and proof report](../RETENTION-RECORD.md) for the exact
scope, source correspondence, commands and open integration obligations.

`evidence.tar.gz` retains 111 files (383,109 file-content bytes), including all
protocol results, original failed proof, source-fault outputs and common-test
log. `inventory.json` lists each member hash and the original local roots. The
archive is 47,494 bytes with SHA-256
`e0e866e120ab50fb50d6391905ee69d78b824661fa8147f40a1dcda6defbf24f`.
Every member was read back byte-for-byte after packaging. Local originals were
not removed. No database binary, build target or large model cache is included.

The passing common tests (36) overlap the standalone source-control baseline
(7). Five model counterexamples and four compiled source faults are expected
failures, not successful workloads. The original strict proof draft failed one
of 59 obligations; the corrected 10-theorem proof passes all 59. No ledger,
complete anchor, physical deletion or new Chaos acceptance is claimed.
