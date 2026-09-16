# Single-buffer write-screen evidence

See the [report](../ENTRY-BUFFER-PERFORMANCE.md). Correctness and allocation
accounting pass; the declared write gate fails. Read/range timing is not run.
Production remains unchanged.

[evidence.tar.gz](evidence.tar.gz) contains 444 members,
192,563,670 expanded bytes and 10,934,172 compressed bytes.
SHA-256: `93931fa137c702f53b81f6a87af4a2ba42252a60de2eb6b0d23b4056c8514db5`.
Every member size/hash passed readback; [members.json](members.json) and
[manifest.json](manifest.json) record all retained bytes and exclusions.

The packet contains four identified screen release builds, generated harnesses,
source correspondence, all 52 workload launch/terminal records and outputs,
32 timing / 16 allocation rows and every raw window, independent dataset,
allocation, statistic and final-footprint analysis, and both declared/executable
plans. The corpus and qualified source packet below are external, hash-pinned
inputs. Exact binaries remain local with identities in their build records.

The [qualification packet](../entry-buffer-qualification-v1/README.md) supplies
the measured baseline/candidate engine/common sources and prior proof/models.
The original corpus is `outlined/groups.bin` in the
[outlined packet](../outlined-mutation-path-v1/README.md). Reproduction uses source
base `e17d1b3` and recorded paths; otherwise identify a fresh attempt without
relabeling existing results.

A fifth release build is a **codegen-only** safe key-accessor control, with its
source copy, exact one-expression change, compiler outputs and disassembly.
It executes zero workloads and has no new proof/model qualification or timing.
The initial inspector newline-hash mismatch is retained separately with its
correction; no measured output or gate changed. The
[next plan](next-key-clamp-plan.json) requires qualification before any timing.

No database/Redis result, new MinIO/recovery/actual Chaos acceptance or industrial
checklist closure follows. Keep the failed write gate and skipped read stage.
