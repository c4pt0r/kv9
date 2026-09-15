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

## Current capacity and missing inputs

The latest retained capacity observation has 879,960,866,816 bytes available on
the NVMe root and 9,705,253,265,408 bytes on the data volume. An external cleanup
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
