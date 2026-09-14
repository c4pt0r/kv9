# Full CRC regression evidence

This evidence records selected ThinLTO `11113f6` versus CRC `e748620` with the
fixed native v3 client `0be806d9`: 24 smoke and 48 timed native cohorts,
point/batch64 APIs, 0/50/100% reads, c1/c64, and two complete opposite orders.
All 35,103,005 measured calls succeed once, covering 285,605,495 input items,
with zero measured errors, unknown writes or dropped slots.

Read [REPORT.md](REPORT.md), [PER-REPEAT.md](PER-REPEAT.md),
[COMPARISONS.md](COMPARISONS.md), [CPU.md](CPU.md) and [summary.json](summary.json).
Whole batch latency is not divided by 64; mixed operations use the full elapsed
interval. The exact accepted audit and reporting inputs are retained.

The archive contains **6,498 metadata files / 874,334,238 decoded bytes** in
**52 parts / 108,324,271 compressed bytes**. It includes original driver,
protocol, reports, CPU/configuration/lifecycle observations, audit, reporting,
capacity/cache-cleanup records and retention catalogs. Large WAL objects,
executables and source trees remain referenced by their original hashes and
commits; they are not duplicated in this portable archive.

Timing session 76372 ended at `3727d8/0`; independent campaign audit 12850 at
`3b33fa/0`; reporting at `faf0cf/0`. Packaging 83277 ended at `dde2c9/0`;
independent byte verification 36461 at `79fadc/0`. Publication receipts are in
[publication-receipts](publication-receipts). Verification checks every portable
member against the index; it does not rerun original runtime/source/WAL checks.

```sh
python3 -B verify.py --root . --inventory-sha256 2422847486c6a5a56dcba53a4369938b2bc711e40fe69f4a186d792b51e0d212
```

The independent runtime audit decoded 119,471,576,199 logical WAL bytes and
verified 192 timed / 96 smoke lifetimes exited, 144 timed drains/bindings and
restored container CPU/namespace configuration. Raft quorum, synchronization,
durable apply and response fences were preserved. This is shared-host loopback
with volatile tmpfs WAL, not a real-disk/power-loss/cross-host result. Exact-source
CRC proof, ordinary recovery and actual Chaos histories remain separate evidence.
No main promotion, Redis parity, statistical significance or new Chaos claim is
made by packaging. All execution was local; no hosted CI was dispatched.
