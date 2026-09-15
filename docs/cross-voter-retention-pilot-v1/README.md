# Cross-voter WAL retention: six-member pilot

The bounded Zstandard patch experiment and separate original-hash readback
pass. Two completed c1 cohorts supply one sequence-2 engine WAL from each of
three voters: six members, 100,639,065 logical bytes. All six original hashes
differ. Matching uses cohort, voter and ordinal; it does not assume identical
log records or boundaries.

| Selected workload | Three ordinary objects | One ordinary base + two patches | Reduction |
| --- | ---: | ---: | ---: |
| Point Put | 32,303,604 bytes | 11,821,922 bytes | 63.404% |
| BatchPut(64) | 35,575,001 bytes | 11,893,526 bytes | 66.568% |

Ordinary and patch compression use the same pinned zstd 1.5.5, level 3,
single thread, checksum and explicit window setting. Every reconstructed WAL
matches its complete original SHA-256 and size. Four wrong-base controls fail
the pinned whole-base hash before decoder launch; four corrupted patch copies
produce explicit checksum failures. All 30 actual codec lifetimes exit and are
reaped. Producer `99674/8dfa0b/0`, independent readback `73260/ac2d2e/0`, root
summary `15464b/0`.

The first decoder failed with exit 11 and no decoded bytes because bare
`-M512` was interpreted as 512 bytes on this installed binary, despite its help
text saying megabytes. Seven tiny diagnostic children establish that explicit
`--memory=512MiB` works; the corrected pilot changes only that option and fresh
output bindings. The original failure, a root result-read permission failure
and a refused placeholder substitution remain in the archive.

The resource policy retains 512 MiB address space per codec, one child at a
time, an 8 GiB available-space floor, a 1 GiB added-allocation cap and a shared
1,200-second deadline. Peak sampled pilot allocation is 472,502,272 bytes;
cumulative decoded output, including negative controls, is 334,955,441 bytes.
Minimum observed free space is 24,629,485,568 bytes. No original object,
catalog or evidence file was removed or rewritten: **reclaimed bytes = 0**.

## Capacity decision

A separate metadata-only assessment checks all 72 accepted catalogs across
the completed FNV, directory and frame-buffer write screens. It identifies
4,452 same-cohort n2/n3 targets occupying 51,873,718,272 bytes, while keeping
2,229 n1 bases occupying 25,936,003,072 bytes. One unmatched FNV target is
excluded. No original payload is read during this assessment.

That target allocation is a ceiling, not a forecast. Even impossible zero-byte
patches would bring approximately 24.63 GB free space to only 76.50 GB, below
the roughly 79.46 GB historical launch estimate before charging patches or
metadata. The sample reductions must not be extrapolated into reclaimed space.
Further eligible retention or added disk capacity is required to close that gap.

Before migration, verify reconstruction of the exact original compressed-object
bytes, because existing catalogs and readers pin those hashes too. Then require
bounded staging, independent complete readback, restore/refusal checks and exact
accounting before retiring any named object. Database workloads, Raft rules and
the complete performance comparison remain unchanged.

## Original reporting evidence

Run `python3 -B docs/cross-voter-retention-pilot-v1/verify.py`.
The standalone reader checks all 167 members / 11,051,321 bytes and gzip EOF,
then recomputes base-inclusive comparison arithmetic and the metadata allocation
ceiling. Archive size is 1,002,370 bytes; SHA-256
`4da2528d42ea0c33214f2e5b523e04791f16abe38d3894c620ee7ee7199c8967`.

The archive preserves both frozen preparations, original failures, diagnostic
and actual codec receipts, independent roundtrip/control results, root lifetime
checks and the complete three-screen metadata assessment. Original WALs,
compressed objects, valid patches and decoded/control payloads remain local;
portable reporting readback does not rerun their codec checks. No database
throughput or latency result is claimed.
