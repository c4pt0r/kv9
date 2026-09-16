# Inline-key write-screen evidence

See the [report](../INLINE-KEY-PERFORMANCE.md). All correctness/accounting checks
pass, but the write gate fails. The declared next read stage is not run.
Production is unchanged.

[evidence.tar.gz](evidence.tar.gz) contains 365 members,
154,073,960 expanded bytes and 4,776,643 compressed bytes.
SHA-256: `80705ec8f41250dd46b6c4ea5ebf2b535aff496e5e2236fa59179307a4645a4a`.
Every member size/hash passed readback; [members.json](members.json) and
[manifest.json](manifest.json) record all retained bytes and exclusions.

The packet includes four identified release builds, exact generated harnesses,
source correspondence, all 52 process launch/terminal records and outputs,
32 timing / 16 allocation rows and raw windows, independent input/allocation/
statistics checks, final-index requested-byte observations and both declared
and executable stage-one plans. Timing is separate from allocation counting.
All binaries remain local with exact hashes in their records.

The [qualification packet](../inline-key-qualification-v1/README.md) supplies
the exact original/candidate engine/common sources and prior model/proof evidence.
The original corpus is published as `outlined/groups.bin` in the
[outlined packet](../outlined-mutation-path-v1/README.md). Manifest hashes bind
both external inputs. Restore recorded paths/source base `940d13e`, or adapt a
fresh attempt without relabeling existing results.

The [next entry-buffer plan](next-entry-buffer-plan.json) is prospective, not
implemented or timed. No database/Redis result, new MinIO/recovery/Chaos acceptance
or industrial checklist closure is claimed here. Keep all failed gates intact.
