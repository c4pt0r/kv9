# V3 workload performance evidence

The [completed report](../../../docs/V3-WORKLOAD-PERFORMANCE.md) compares point
and batch64 GET/PUT/mixed traffic across baseline 5ee897a, read-credit candidate
57ff6851 and Redis. The [full readout](READOUT.md), per-repeat and pooled CSVs,
and statistics JSON retain all 72 cohorts. This recording changes no server
implementation and establishes no general promotion or Redis parity.

`PREPARATION.md`, the frozen protocol/driver/auditor, their contract logs,
preparation identities and `results-first/` retain the independent audit.
`outer/` retains timed invocation, isolation/restoration summaries and matrix.
`smoke/matrix.json` and `root-smoke-*` retain the direct smoke invocation and
terminal readback. The root ancillary freeze-check failure, postlaunch
inventory and first statistics-reader failure remain in their original files.
The separate corrected reader did not modify or rerun the frozen audit.

`stages/` contains the independent endpoint reader, its original arithmetic-only
version, binding-review correction, first full integration result and pooled
means. Endpoint intervals include setup, warmup, measurement, drain and final
verification, overlap, and have independent populations. They are not an
additive latency budget or a measured-only profile.

Full raw files, large retained WALs and executables remain under the exact
absolute paths recorded by the input inventories, including
`/tmp/kv9-v3-workload-matched-diagnostic-attempt1` and the separate smoke root.
Container inspection documents and binaries are excluded from this publication.
This is a compact evidence publication, not a relocatable runtime bundle.
Source/build/helper identities remain pinned to their original trees.

`exact-copy-inventory.json` maps each copied file to its original path, byte
count and SHA-256; this authored index and the inventory itself are excluded.
Retained diffs preserve original whitespace. The first publication copy's
incorrect smoke-directory assumption and its correction are also retained.
All recording and validation ran locally; no GitHub CI was dispatched. The
next priority is point/batch write CPU attribution and a measured apply-path
improvement with unchanged Raft, recovery, snapshot and backpressure contracts.
