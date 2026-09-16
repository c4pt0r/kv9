# Safe-accessor qualification and write-screen evidence

See the [report](../KEY-CLAMP-PERFORMANCE.md). Qualification and accounting pass;
the write-selection gate fails. Read/range timing is not run; production stays
unchanged.

[evidence.tar.gz](evidence.tar.gz) contains 827 members,
251,802,029 expanded bytes and 9,970,710 compressed bytes.
SHA-256: `2ca7e32b9dabff6d250dc9ae0ca5cdccb37d25f89cee2c8978f3216ca2cc38eb`.
All sizes/hashes passed member readback; [members.json](members.json) and
[manifest.json](manifest.json) record exact provenance and excluded local binaries.

`qualification/` retains the changed candidate workspace, source bindings,
206-test output, 26-theorem/15-control proof, 198 allocation observations and
audit. The source-checked unchanged baseline/control evidence is external below.
`failed-preparation/` retains the initial six builds, first configuration-guard
failure and exact pre-fix tools. It produced no workload or timing samples.
`experiment/` retains the six corrected builds, comparator excerpts, all 78
accepted workload launches/terminals and outputs, raw timing/count windows,
independent analyses and the exact two-guard correction audit.

The [earlier qualification](../entry-buffer-qualification-v1/README.md) supplies
unchanged baseline/control sources and reusable tests. The
[earlier single-buffer packet](../entry-buffer-performance-v1/README.md) supplies
the original codegen control and declared next plan. The corpus is
`outlined/groups.bin` in the [outlined packet](../outlined-mutation-path-v1/README.md).
Manifest hashes bind these inputs. Reproduction starts from `b23506d` and the
recorded paths, or an explicitly identified fresh attempt.

The [radix census](radix-topology.json) checks static topology only. The
[next radix plan](next-radix-plan.json) is prospective: no Rust radix implementation,
algorithm proof, timing or speedup exists. No new database/Redis, MinIO/recovery,
actual Chaos acceptance or industrial checklist closure is included.
