# Replicated Redis write baseline evidence

The [report](../WRITE-REDIS3-BASELINE.md) covers the selected three-voter KV9
against Redis 7.0.15 with one primary and two replicas, using WAIT 1 and WAIT 2.
The original 12 smoke and 24 timed cohorts pass independent acceptance.
All 18,993,624 measured calls succeed once, covering 259,615,761 input items,
with no measured errors, unknown writes or dropped slots. This is a short,
one-host volatile-storage baseline, not equal durability or a runtime promotion.

The reporting bundle contains **2,303 exact original files / 354,258,232 decoded
bytes**, compressed into **29 parts / 60,782,348 stored bytes**. Concatenated
gzip SHA-256: `5fcd958d38069c0b5b866aa8ec0d4caefe0ff7d1d19b4eb763169aa18ec53037`.
Parts are at most 2 MiB. [The inventory](inventory.json) maps every member to
its original source path, byte length and SHA-256; it also binds each part and
the original summary and reader sources. Packaging checked the existing audit
hashes where present and retained filesystem identities before reading files.
Every member and part passed an independent readback (`40c31f`, exit 0).

The bundle includes all 36 complete client reports and raw histograms,
requested/effective configurations, resource samples, process cleanup,
72 complete Redis dataset exports, native scan pages, replication barriers,
12 native retention catalogs, decoder receipts, original and repaired audits,
all root timing/audit receipts and original preparation/summary failures.
The initial failed audit remains failed. The five smoke-schema controls pass,
and only the corrected audit ran again; no workload was repeated.

[Omitted payload references](omitted-payload-references.json) preserve catalog
identities for 1,328 local compressed objects: 22,546,628,885 stored bytes and
32,215,539,071 decoded original bytes. Those objects, original executables,
old cold-storage objects, Cargo targets and environment credentials are not
included. They remain at their original local paths; their accepted hash/decoder
receipts are retained. This reporting bundle cannot restore WAL data or replay
the complete runtime audit without those local dependencies.

To check published byte identity, run locally:

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONOPTIMIZE=0 python3 -B docs/write-redis3-baseline-v1/verify.py
```

To reproduce the pooled and per-repeat rates, whole-call histogram bounds,
outcomes and sampled process CPU from the archived reporting inputs:

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONOPTIMIZE=0 python3 -B docs/write-redis3-baseline-v1/recompute.py
```

The latter uses an exclusively owned temporary directory, extracts only 74
hash-bound arithmetic inputs and compares the regenerated summary byte for
byte. It passed (`9d130b`, exit 0). It launches no workload, fault, build or WAL
decoder. CPU recomputation requires the recorded 100-Hz process tick unit.
[The validation receipt](publication-validation.json) records the concrete
runtime, audit, packaging and arithmetic terminals. These separate checks are
not additional workload samples or broader correctness qualification.

The archive verifier bounds parts to 2 MiB each; the publisher additionally
caps compressed data at 512 MiB, members at 64 MiB and all added artifacts at
1 GiB. Actual packaging used 60.8 MB of compressed output and preserved the
original 96-GiB free-space floor. The in-memory verifier is bounded by that
512-MiB compressed ceiling; arithmetic extraction is below 512 MiB. No build,
profiler or codec overlapped the original timed windows.

This evidence branch starts from measured selected server `11113f6`. Its
reporting files do not qualify a new production build. Main links this branch's
exact published commit and preserves its existing 64-MiB evidence allowance.
The full source, duration, topology, sampling, durability and history limits
remain in the report. Actual candidate Chaos and write A/B remain next.
