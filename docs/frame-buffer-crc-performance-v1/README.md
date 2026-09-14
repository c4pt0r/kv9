# Frame-buffer-on-CRC write comparison

**Keep CRC main; the frame-buffer candidate remains experimental.** Loaded point Put changes -0.517%. BatchPut(64) changes +0.379% with a worse pooled p99, and both loaded workloads reverse throughput direction between the two orders. The small c1 batch improvement does not establish general selection or statistical significance.

The original integrated CRC server is `bd42e60f84657e22e36e34924a5c80a08eac623a`; the candidate is `e9249f2cbd069dcdc44312be826a68494cf694db`. Both use the exact native v3 client. All 16 timed cohorts and eight smokes pass independent acceptance. All **7,247,954 measured calls / 65,011,016 input items** succeed once, without errors, unknown writes or dropped slots. Smokes are separate: 810,022 calls and 8,246,542 items.

- [Pooled throughput, whole-call latency and CPU](REPORT.md)
- [Both individual orders](PER-REPEAT.md)
- [Directional comparisons](COMPARISONS.md)
- [Complete machine-readable statistics](summary.json)

Timing uses three voters with ordinary Raft/sync/apply/response fences and volatile tmpfs WAL, one shared host, 4,096 keys plus sentinel, 128-byte values, seed 71, 128 warmup calls, point Put/BatchPut(64), c1/c64, ten-second measurements and two opposite complete orders. This is not a disk, power-loss, cross-host, full read/mixed regression or equal-durability Redis comparison. Redis was not rerun. CPU samples are not request service-time attribution.

The independent campaign audit checks 64 timed / 32 smoke lifetime exits, 48 fresh timed drains and writer/listener bindings, 2,306 role/source checks, 3,070 resource samples and exact environment restoration. It independently decodes and hash-checks 74,191,158,465 logical WAL bytes; resident allocation is 52,317,179,904 bytes. Original WAL/objects/ELFs remain local and are catalog-bound in [omitted payload references](omitted-payload-references.json).

Actual timing terminal: **96921/e49a51/0**. Independent campaign audit: **54774/5c1dec/0**. Five reporting controls and reporting: **821d81/0**. The portable package contains **2,291 files / 319,523,066 decoded bytes** in **19 parts / 38,290,587 compressed bytes**. It preserves reports, configurations, source/build identities, resource samples, audit results and the importable reporting implementation. Package terminal **7627/9d19e5/0** and independent byte readback **d4d29f/0** pass; [publication receipts](publication/terminal.json) retain these separate stages.

From this directory:

```sh
python3 -B verify.py --root . --inventory-sha256 356a8de20958357d56a22c56d6cb11399a5a3ac2e582ff335196e11b0e9414a0
```

This verifier checks exact portable bytes, not excluded WAL/ELFs or a new runtime execution. All original guards remain; no hosted CI was dispatched.
