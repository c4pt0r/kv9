# ThinLTO original qualification and performance evidence

This bundle retains 1,243 original files, 212,101,156 decoded bytes, in five
bounded archive parts (10,294,468 compressed bytes). `inventory.json` binds every
selected original path, size and SHA-256. `verify.py` checks archive member and
top-level byte identity without extraction, runtime execution or re-auditing.

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONOPTIMIZE=0 python3 verify.py
```

[READOUT.md](READOUT.md) and [PER-REPEAT.md](PER-REPEAT.md) are the unchanged
accepted statistical reader output. All four point-read/mixed cells improve
throughput, mean and p99 in both run orders. This is a favorable short screen;
broader API and actual exact-build Chaos Mesh gates precede default promotion.
Selected runtime remains CRC `ca0002c7`.

Candidate source: `02d0c01024b65a84b220c6948ff2224bfa7900bc`.
Retained server SHA-256:
`dd028cb2f61633dda133b05d814a6d173a79a83145d8ced2f81dc0cf8e0f33bc`.
Fixed native/Redis measurement clients remain the original `0be806d9` v3 builds.
Full workspace qualification passes 709 tests/doctests, with 23 existing ignored;
ordinary recovery retains 359 operations (326 OK / 33 unknown).

All 12 smoke and 24 timed cohorts complete on their first executions. Accepted
timing session `79984/0` contains 50,708,100 measured single-attempt successes
and zero dropped slots. The independent audit and unchanged arithmetic reader
validate the complete original inputs; no partial result or averaged percentile
is used. Setup routing attempts remain separately accounted.

The compact selection includes source/workspace checks, actual production
codegen/cache receipts, ordinary recovery histories, frozen drivers/auditors,
accepted reports/statistics and independent metadata review. Current experiment
binaries, raw WALs and bulky host observations remain local under their original
inventories. Historical cold WAL retention is documented in the separate
retention overlay; absent cold paths require rehydration before old full audits.

Scope: shared-host loopback, ordinary three-voter quorum/sync on tmpfs WAL versus
standalone Redis without persistence/pipelining. No equivalent durability,
physical-disk, cross-host, sustained-capacity, statistical-significance, compiler
verification, complete implementation proof or actual candidate Chaos claim.
No hosted CI ran.
