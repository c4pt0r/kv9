# Multi-group throughput baseline validation packet

See [implementation, measured scaling and the bottleneck ledger](../MULTI-GROUP-THROUGHPUT.md).

- 914 passing workspace tests/doctests in the final serial qualifying run, 33
  ignored, zero failures; strict Clippy/formatting pass. Six Lean models and
  the retention TLA/TLAPS model are rechecked against the exact working tree
  after the `data_workers` plumbing (no model covers pool sizing; contracts
  re-pin the touched files).
- **599 qualified single-keyspace workload run reports** across four sweep
  rounds on one 32-core host with tmpfs stores. Headline (equal 32
  workers/group, admission 1024/node, 30 s cells, two reps): put throughput
  126.1k → 203.6k → 270.4k → **292.2k ops/s at eight groups (S(8)=2.32)**.
  Fixed-aggregate panels for put/get/mixed peak at four groups; the
  single-group baseline is consistent with the retained 137.9k/s tmpfs
  measurement. A separate RAID5-HDD panel (0.86k/0.73k ops/s, fsync-bound)
  is recorded and never pooled.
- The bottleneck ledger is fully evidence-backed: the 64/node default
  admission budget binds first (0.55 M refusals in one 30 s cell);
  per-group pipeline depth governs batching at fixed aggregate load; the
  default two shared data workers beat 1/4/8 workers at every measured
  point under and at saturation; sixteen full-rate groups exceed this
  fixture's 2 s read-barrier startup envelope — shown insensitive to
  workers, admission and launch stagger, with failure-moment forensics
  (stable terms/leaders, healthy probes seconds later) retained for all
  six failed g16 panels.
- Early-round harness defects are retained with their failed cells:
  leader-only routability probing, matrix abort on first failure and a
  cross-panel store-path collision; all repairs are harness-only, and the
  measurement methodology was predeclared before any timed run.

[evidence.tar.gz](evidence.tar.gz) contains **6075 readback-verified files**,
4160035 compressed bytes and 235002581 decoded bytes (declared 512 MiB
decoded cap for this packet; the archive itself stays under the inventory's
64 MiB documentation-file bound). SHA-256:
`56d9f131fa8cc1c3c8e7713c4fb16fd501bff063b6f146461e3f273b79f7dcf6`.
[validation.json](validation.json) inventories all members. A separate
extraction re-verified every member and recomputed the headline scaling from
the embedded sweep summary; [portable-readback.json](portable-readback.json)
records that result.

Tested source is based on `eddb7165a814989a752d40711fcb085c74d59e24` with the
exact current Rust/Cargo/proto source retained under `source/`. Workload and
server ELF binaries, the root bootstrap credential and per-cell tmpfs stores
are excluded; the build manifest with full source provenance is embedded in
every run report.

Nothing here is multi-host scaling, online expansion, split/placement,
p99-budget sustained-load acceptance, a durability improvement or a Chaos
Mesh campaign. Proposal batching/group commit remains open as
[#20](https://github.com/c4pt0r/kv9/issues/20). The independent 3/6/9-host
benchmark contract in [HORIZONTAL-SCALING-PLAN.md](../HORIZONTAL-SCALING-PLAN.md)
remains the only acceptance path for effective horizontal scaling. No hosted
CI was dispatched.
