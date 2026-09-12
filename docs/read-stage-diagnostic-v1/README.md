# Read-stage diagnostic evidence

This bundle preserves 291 original files (39,605,602 decoded bytes) from source
qualification, retained release provenance, the two complete diagnostic fixtures,
independent analysis, runtime contracts and the prior accepted metric snapshots
used to choose the boundary. Two compressed parts total 2,307,812 bytes.

Run `python3 docs/read-stage-diagnostic-v1/verify.py` from the repository root to
verify part/member hashes and bounded extraction. Verification establishes
integrity, not replacement runtime or correctness acceptance.

The [report](../READ-STAGE-RESULTS.md) explains the stage populations, source and
runtime scopes. Source checks: session 89777, exit 0, terminal `3ff3d5`. Original
release: 45892/0, `86897c`. Runtime: 7526/0, `ce7697`. Independent readback:
`5126c0`, exit 0. Metric/build contracts `719e98`, fixture contracts `c1736b`,
and source preflight `c5be87` all exit 0. Original fixtures were not rerun.

Executables and WAL remain local under `/tmp/kv9-read-stage-release-first` and
`/tmp/kv9-read-stage-isolation-first`. The original input inventory binds the
retained files. Published raw reporting selections preserve all original bytes;
no binary or WAL is represented as newly uploaded here.

This is opt-in instrumented diagnosis, with full client phase accounting. It
does not establish a new uninstrumented QPS, candidate Chaos qualification,
independent host-failure result or industrial readiness.
