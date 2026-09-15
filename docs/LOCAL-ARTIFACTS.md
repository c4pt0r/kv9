# Local development output placement

Updated 2026-09-15. New bulk development output belongs under
`/mnt/data/kv9-work`, an existing mounted data volume. The directory is owned by
the development user and mode 0700. Use a fresh named directory for each run;
pass its absolute path to the producer's output/evidence argument. This policy
applies to subsequent local automation as well as interactive runs.

Put proof logs, experiment reports, retained evidence, downloaded proof tools
and archive assembly on that volume. Keep active latency-sensitive test data
and the reusable Cargo target on their explicitly selected NVMe filesystem.
Record these two locations separately. Do not globally export `TMPDIR` to the
data volume: the ledger workspace attempt there had six election/apply timeouts
and a measured 2.542-second successful WAL sync during overlapping bulk work.
The same six tests and frozen executable passed on the original NVMe filesystem
after installation finished. This comparison does not isolate those two causes.

Existing evidence is not moved, replaced by symlinks, or deleted as part of the
placement change. Absolute artifact paths, hashes and original execution inputs
remain historical identities. Cleanup must first identify inactive, expendable
outputs; free space alone does not establish that an old binary can be removed.
No global shell profile, mount, Docker configuration or hosted CI was changed.

## Continuing automation

The active write-comparison command manifest now places both the complete
eight-cohort smoke output and sixteen-cohort timed output under
`/mnt/data/kv9-work/upper-bound-requalified-{smoke,timing}-20260915-first`.
Preparation, execution logs, independent readback and compressed retention also
use the data volume. The owned pre-upload crash preparation and follow-up
checkpoint-owner proofs use the same parent directory. New attempts must choose
fresh names there instead of returning to a hard-coded `/tmp/kv9-*` output.

This controls retained output, not every temporary byte: active performance
voters still use `/dev/shm`, and explicitly selected NVMe test fixtures and the
reusable development compiler cache can still grow on the root filesystem.
Continue checking both devices. Do not change a running process's open output
files or silently relocate a benchmark's WAL to a different storage class.

At the current campaign launch check, available space was 877,222,043,648 bytes
on root, 9,689,422,909,440 bytes on the data volume, and 66,274,971,648 bytes on
tmpfs. The full campaign's 79,455,850,496-byte output requirement passed. These
are observed available bytes, not a space reservation or a cleanup claim.

## Chaos output and capacity accounting

`scripts/checkpoint-publication-chaos.py` and its ledger extension accept the
output directory through the existing CLI. Pinned tool locations can be selected
with `KV9_KIND` and `KV9_CHAOS_KUBECONFIG`; the latter is private and is never an
evidence payload. Keep workload inputs and tool digests in the run manifest.

The guard measures both the host root filesystem and the output filesystem,
deduplicating them by device when they are the same. Each distinct filesystem
must satisfy the unchanged 9 GiB + 8 MiB launch headroom and 8 GiB continuous
floor. The sum of their retained maximum decreases must stay within 1 GiB.
Cleanup cannot reset already charged growth. A device change refuses, and
resuming preparation must inherit the original baselines and minima. These are
conservative global free-space observations, not exact attribution to one run.
Ten deterministic controls exercise the same-device and separate-device cases,
both floors, aggregate growth, cleanup and baseline inheritance.

## Current capacity and historical input recovery

The [2026-09-15 20:14 UTC observation](write-receipt-upper-bound-performance-v1/capacity-observation.json)
has 877,209,255,936 bytes available on the NVMe root (about 817 GiB) and
9,638,572,486,656 bytes on the data volume (about 8.77 TiB), after the completed
performance campaign and its reporting packet. An earlier external cleanup
changed capacity during development; this increment does not claim those bytes
as its own cleanup result. Free space is a point-in-time observation, not a
reservation. Subsequent runs must check again.

The 79,455,850,496-byte full performance gate is no longer blocked by the observed
capacity. Its historical fixed v3 client at
`/tmp/kv9-point-write-v3-release-first/native/kv9-batch-benchmark` is currently
missing, however. Restore and verify the exact causal inputs before that screen;
do not substitute a newly built client or a shorter workload without a separately
qualified plan. Old proof tools and kubeconfig also disappeared; exact pinned
tools were restored under the data volume and kubeconfig was exported from the
existing local Kind cluster. No cluster recreation was needed.

One recorded offline rebuild restored all 581 original client source files and
used the retained Rust/Cargo 1.94 toolchain. The new 5,592,624-byte ELF does not
match the historical 5,594,104-byte client. Matching source and compiler-artifact
records do not establish byte identity or qualify a replacement benchmark
client by themselves. Subsequent source tests and all eight actual smokes with
independent dataset/report/lifetime checks have
[qualified that specific replacement](WRITE-CLIENT-REQUALIFICATION.md). The
[complete matched timing and retention audit](WRITE-RECEIPT-UPPER-BOUND-PERFORMANCE.md)
subsequently pass with 50,835,156,992 allocated bytes retained on the data
volume; candidate promotion remains held because of the batch tradeoff. Recovery
inputs, comparison and build output are retained under
`/mnt/data/kv9-work/performance-input-recovery-20260915-first`.
