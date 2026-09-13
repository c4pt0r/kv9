# CRC slicing-by-eight write A/B evidence

The initial write screen accepts eight smoke and sixteen timed cohorts in two
opposite orders. Loaded BatchPut(64) improves 18.807% to 1,063,493.134 items/s;
p99 falls from 9.306–9.437 ms to 6.947–7.012 ms. Loaded point writes improve
2.786% to 139,402.831 calls/s. All 7,126,939 measured calls succeed once,
covering 60,849,685 input items, without unknown writes or drops.

This evidence branch is based on measured candidate
`e748620a7b0ba326ac5f4fa8e3a6b1ff48553c6a`. Added reporting files do not redefine
the measured source or select the candidate by default. The control is
`11113f68f6a5df77da1ffb4fcec850953716ffa3`; the fixed native v3 client is
`0be806d9671e2c50701a64aa7889c8859b7648ba`. Exact binary/build/source pins are
retained in the original summary and manifests.

Both roles use three voters with normal Raft commit, synchronization, durable
apply and response fences, on explicitly volatile tmpfs WAL. Target processes
share CPUs 2–5 and clients CPUs 0–1. Each measured window lasts ten seconds,
with 4,096 mutable keys, a sentinel, 128-byte values and concurrency 1 or 64.
No profiler, build or codec overlaps timing. Whole-call latency merges original
histograms; percentiles remain bucket bounds. CPU includes all three voters
and represents sampled process intervals. This one-host screen establishes no
real-disk, power-loss, independent-host, sustained-capacity or Redis parity claim.
Full read and mixed regressions remain before promotion.

## Results and acceptance

[summary.json](summary.json), [PER-REPEAT.md](PER-REPEAT.md) and
[COMPARISONS.md](COMPARISONS.md) preserve pooled and both individual orders,
all outcomes, attempts, latency intervals and per-voter/client CPU. Throughput
and mean improve in all eight per-order comparisons; p99 improves in seven
and remains in the same bucket for forward c1 point writes. There is no
statistical-significance claim from two short repetitions.

Actual timing session 97690 ends at `5063f4/0`; independent audit session 40042
ends at `1e3734/0`. Original summary arithmetic passes `50f90b/0`. The audit
checks 64 timed lifetimes, 48 fresh voter drains, 48 writer/listener bindings
and 3,071 resource samples; another 32 smoke lifetimes exit. All
69,247,125,269 logical retained WAL bytes are independently decoded and
hash-checked by that original audit. Its actual source/build, placement,
configuration, cleanup and restoration observations remain preserved.

The initial nested-sudo setup failure and smoke-checker import-path failure
remain retained alongside narrow repairs. No cohort was rerun. Capacity
records distinguish archival-only histories from accepted-WAL histories;
compression does not upgrade their acceptance. Both capacity readbacks and
conditional first-party cache invalidation completed before the fresh capacity
releases. Every original benchmark storage cap and floor remains unchanged.

The report also retains the publication selector's first rejected target label
and one-token correction, plus the final metadata binding/sidecar review. No
runtime, auditor or summary arithmetic was changed by publication.

## Exact reporting bytes

The [inventory](inventory.json) binds **2,364 original files**, totaling
**337,214,945 decoded bytes**, in **20 archive parts** totaling
**41,291,854 compressed bytes**. The whole compressed SHA-256 is
`1a03f85371b4d523f655e935a111ee270e0f83c439921757f66073c973fd09a5`.
Its original-path mapping includes all 24 client reports, full native dataset
and drain observations, resource samples, build/helper inputs, original audit,
summary, failures, and bounded capacity completion/hash-index metadata.

The [omitted payload references](omitted-payload-references.json) retain
2,796 catalog-bound WAL object references and exact executable manifests.
Large local compressed objects and executables are not included. This bundle
alone cannot restore the database or replay the live/source/WAL audit. Historical
maintenance stores and full journal trees also remain outside this reporting
bundle, separately bound by the included completion and hash-index authorities.

The [publication validation](publication-validation.json) records actual
publisher, byte-verifier and arithmetic terminals. The original published
inventory SHA-256 is
`4162210534e3ab74006fa9096f7986c9129b7823479d6a1e28e14442f9af4480`.
The README and publication validation are tracked by the evidence commit;
the inventory binds the listed archive and top-level reporting inputs.

## Recompute without the original environment

With Python 3.11+ on POSIX reporting a 100-Hz SC_CLK_TCK unit:

```sh
PYTHONOPTIMIZE=0 PYTHONDONTWRITEBYTECODE=1 python3 -B verify.py
PYTHONOPTIMIZE=0 PYTHONDONTWRITEBYTECODE=1 python3 -B recompute.py
```

`verify.py` checks every archived original byte. `recompute.py` makes three
explicit I/O path substitutions in the exact original summary source, verifies
that all other helper source is unchanged, and checks the regenerated summary
bytes and original input hashes. It uses a bounded temporary directory and
performs reporting arithmetic only. Both commands pass (`9123f5/0` and
`90f0d9/0`); they do not launch a database, restore WALs, rerun the live auditor
or establish new performance/correctness acceptance.

Exact-source proof/recovery and actual Chaos evidence are documented separately
in [qualification](https://github.com/c4pt0r/kv9/blob/10894a62ee39d9573e3e462be4d0834aaa5f048f/docs/write-reference-qualification-v1/README.md)
and [the 21-window Chaos report](https://github.com/c4pt0r/kv9/blob/10894a62ee39d9573e3e462be4d0834aaa5f048f/docs/WRITE-CRC-CHAOS.md).
No original industrial checklist item closes. All execution was local; hosted
CI remains manual.
