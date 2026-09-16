# Static pointer callback performance evidence

See the [report](../STATIC-POINTER-CALLBACK-PERFORMANCE.md). All 18 write cells
improve in both orders, but material-gain and read-tail gates fail. No runtime
promotion or database-QPS gain is claimed.

The [archive](evidence.tar.gz) retains 1804 files, 53415387 expanded bytes.
It includes the exact plan, independent corpus/probe preparation, both source
and build graphs, 338 process terminal records, every timing/count row, analysis
and audit, read-pass diagnosis, codegen and the unqualified next hypothesis.

Archive SHA-256: `a1602dee6c9e62c6805d50816131549c4a0b9c1a78490ada2f588a0488b6f92f`.

[manifest.json](manifest.json) records identity and exclusions;
[members.json](members.json) binds every member. Every archive member passed
SHA-256/size readback. The archery source snapshots are accompanied by their
[original MIT license](ARCHERY-LICENSE.md).

[analysis.json](analysis.json) publishes all 42 case comparisons.
[read-pass-review.json](read-pass-review.json) decomposes all read rows without
discarding samples or changing acceptance. The next mutation-adapter patch is
a proposal only; it has no accepted build, tests, proof or timing.
