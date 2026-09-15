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
A subsequent metadata-only assessment of the completed CRC full-regression
screen adds 72 accepted catalogs and 2,447 eligible targets occupying
27,797,909,504 bytes, with no missing, changed or unpaired targets. Together,
the four scopes contain 79,671,627,776 bytes of eligible target allocation.
This establishes an additional candidate scope, not actual savings or capacity.

The exact compressed-object follow-up below now passes for the six samples.
Before retiring any named object, qualify bounded staging, independent complete
readback, restore/refusal checks and exact accounting for a whole selected
cohort. Database workloads, Raft rules and the complete performance comparison
remain unchanged.

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

## Exact original compressed-object follow-up

All six samples also reproduce the original compressed-object SHA-256 and
length: 67,878,587 encoded bytes. The original encoder took a regular file
descriptor as stdin; replaying its exact argv and input mode reproduces its
bytes. The earlier pilot's named-file ordinary encodings are comparison
outputs and are not substituted for these old objects.

A separate reader checks each original compressed identity, then replays its
original decoder and checks all 100,639,065 logical bytes. All twelve encoder
and decoder lifetimes exit. Reconstruction `70c91b/0`, readback `35f5d0/0`, root
summary `795310/0`. The maximum sampled added allocation is 168,681,472 bytes;
minimum observed available space is 24,450,084,864 bytes. Old compressed
objects were not opened by this follow-up; the already verified raw pilot
files and original catalog digests supply its inputs. No object is retired.

Run `python3 -B docs/cross-voter-retention-pilot-v1/verify-followup.py` for the
separate follow-up reporting archive. It verifies 68 members / 6,007,742 bytes,
the original compressed/logical counts and the additional CRC metadata ceiling.
Portable readback is `4b3867/0`. Archive size is 550,941 bytes; SHA-256
`0ad4c34b52c050d14b99cee4d71c5fe7755c35edacef3d2553e1daffe29cb62e`.
This proves compatibility for six sampled objects, not a complete migration
or a legacy whole-cohort audit after restoration. Those remain the next gate.
