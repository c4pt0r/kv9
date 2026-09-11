# Typed authenticated point dispatch: screening decision

**Do not select this candidate as the next performance version.** Removing
request/extensions and an extra async-trait boundary produces only 0.966% and
0.019% single-GET gains in the paired repeats. The second result is effectively
unchanged. BatchGet(1) improves modestly, but this screening does not justify
spending a full acceptance campaign on the candidate for the current target.
No full workspace or exact-candidate Chaos campaign was run after this decision.

The isolated source `dcb862a6569d532ed039ae08b0045f69230c6a51` is pushed on
`codex/typed-point-dispatch` for reproducibility. It starts from f2 with the system
allocator. It does not replace the separate [jemalloc experiment](JEMALLOC-SERVER-PERFORMANCE.md)
or relabel its 304,863–305,903 GET/s result as a regression. Master/default
selection remains unchanged.

The [source correspondence](https://github.com/c4pt0r/kv9/blob/dcb862a6569d532ed039ae08b0045f69230c6a51/docs/TYPED-POINT-DISPATCH.md)
extracts the five authenticated operations into shared inherent async methods.
Per-frame custom authentication, decoded-byte admission, stream-only batch
bounds, backend preparation and response encoding are preserved. This is a
source argument, not a new machine-checked adapter or core-algorithm proof.

Before screening, 37 point-related tests, 31 gRPC tests and 10 optional RPC tests
passed, with overlap between filters; these are not 78 distinct tests. The
server's all-target experimental Clippy check passed with warnings denied.
Four new tests cover identity/role/error precedence, decoded-size admission and
real-stream custom-auth identity changes/revocation. Existing cancellation,
uncertain-write ownership and response-parity tests remain intact.

The default-feature release binds 583 clean source inputs. Its actual
leader-kill/original-directory-restart process E2E passed **381 calls: 351 OK,
30 unknown**. Stream has 194 calls / 179 OK / 15 unknown; unary has 187 /
172 / 15. Both complete atomic histories are valid, unknown writes remain
uncertain without replay, and source/storage/process cleanup bindings passed.
The six-arm correctness smoke completed 2,641,933 successful calls before
isolated timing. Neither process nor smoke numbers are performance claims.

Next inspect repeated authorization metadata construction separately, preserving
per-frame authentication and bounded stream ownership. Do not combine this
unselected extraction with another optimization and attribute a combined result
to one mechanism. Sustained, write/mixed and broader fault/proof gates remain
open before Redis-class RawKV or scale-out acceptance.

## Independent matched report

The first frozen audit passed without rerun. All 12 original cohorts are accepted; root timing session 96016 exited 0. This compares exact f2c4e85 with dcb862a6569d532ed039ae08b0045f69230c6a51, both system allocator, using unchanged native03/Redis b8 clients.

Point GET gains were **0.966% / 0.019%**; the second repetition is effectively unchanged. BatchGet(1) gains were **1.867% / 2.521%**. Pooled point gain is 0.489%, batch gain 2.196%; candidate point throughput is 57.56% of contemporaneous actual Redis GET. This short diagnostic does not establish a meaningful broad performance step or authorize production promotion.

## Original twelve cohorts

QPS uses the original full-call accounting/denominator. All latency columns are whole-call microseconds; p50/p95/p99 are histogram bucket intervals, not statistical confidence intervals. RSS is summed sampled mean across three KV9 voters, or the one Redis process; HWM is the sum of process high-water observations.

| Rep | Arm | Calls/s | Mean µs | p50 µs | p95 µs | p99 µs | RSS MiB | HWM MiB |
|---:|---|---:|---:|---|---|---|---:|---:|
| 1 | old-point | 286,864.0 | 222.948 | 210.944–212.991 | 344.064–348.159 | 393.216–397.311 | 43.290 | 43.711 |
| 1 | old-batch1 | 280,683.2 | 227.837 | 217.088–219.135 | 352.256–356.351 | 401.408–405.503 | 43.114 | 43.531 |
| 1 | new-point | 289,634.3 | 220.820 | 208.896–210.943 | 339.968–344.063 | 389.120–393.215 | 42.980 | 43.430 |
| 1 | new-batch1 | 285,923.2 | 223.648 | 210.944–212.991 | 348.160–352.255 | 397.312–401.407 | 43.117 | 43.531 |
| 1 | redis-mget1 | 493,177.7 | 129.616 | 119.808–120.831 | 169.984–172.031 | 233.472–235.519 | 14.693 | 14.703 |
| 1 | redis-get1 | 508,144.2 | 125.815 | 118.784–119.807 | 159.744–161.791 | 233.472–235.519 | 14.599 | 14.605 |
| 2 | redis-get1 | 500,222.3 | 127.809 | 118.784–119.807 | 165.888–167.935 | 233.472–235.519 | 14.677 | 14.688 |
| 2 | redis-mget1 | 498,644.8 | 128.207 | 119.808–120.831 | 169.984–172.031 | 233.472–235.519 | 14.626 | 14.633 |
| 2 | new-batch1 | 291,814.2 | 219.137 | 210.944–212.991 | 331.776–335.871 | 385.024–389.119 | 43.315 | 43.773 |
| 2 | new-point | 290,799.9 | 219.928 | 206.848–208.895 | 339.968–344.063 | 389.120–393.215 | 42.785 | 43.254 |
| 2 | old-batch1 | 284,639.3 | 224.662 | 215.040–217.087 | 344.064–348.159 | 397.312–401.407 | 43.816 | 44.395 |
| 2 | old-point | 290,744.9 | 219.960 | 208.896–210.943 | 335.872–339.967 | 393.216–397.311 | 43.612 | 44.215 |

## Paired changes

| Rep | API | QPS change | Mean latency change | Aggregate RSS change MiB | HWM sum change MiB |
|---:|---|---:|---:|---:|---:|
| 1 | point | +0.966% | -0.955% | -0.310 | -0.281 |
| 1 | batch1 | +1.867% | -1.839% | +0.003 | +0.000 |
| 2 | point | +0.019% | -0.014% | -0.826 | -0.961 |
| 2 | batch1 | +2.521% | -2.460% | -0.501 | -0.621 |

## Per-voter memory

Each cell is sampled mean RSS / sampled process HWM in MiB. Voter numbers do not prove matching leader roles. HWM includes setup; summing individual HWM is not an observed simultaneous peak. Exact PID/start identities and min/max/first/last sample values remain in statistics.json and audit.json.

| Rep | Arm | Voter 1 | Voter 2 | Voter 3 |
|---:|---|---:|---:|---:|
| 1 | old-point | 13.879 / 13.980 | 15.894 / 16.141 | 13.517 / 13.590 |
| 1 | old-batch1 | 13.546 / 13.617 | 15.989 / 16.223 | 13.579 / 13.691 |
| 1 | new-point | 13.626 / 13.805 | 15.748 / 15.938 | 13.606 / 13.688 |
| 1 | new-batch1 | 13.612 / 13.742 | 15.754 / 15.945 | 13.751 / 13.844 |
| 2 | new-batch1 | 13.960 / 14.066 | 15.639 / 15.895 | 13.716 / 13.812 |
| 2 | new-point | 13.600 / 13.703 | 15.581 / 15.809 | 13.605 / 13.742 |
| 2 | old-batch1 | 13.835 / 13.988 | 16.053 / 16.348 | 13.928 / 14.059 |
| 2 | old-point | 13.938 / 14.059 | 16.005 / 16.379 | 13.669 / 13.777 |

## Acceptance scope and provenance

All 6,452,825 measured calls succeeded (3,452,050 KV9); refusal, unknown-write, read-failure and client-rejection populations are zero. All 40 owned process lifetimes exited. The unchanged auditor verified 2,323 source-file comparisons, 350 resource samples, 24 fresh drain records, 24 voter/writer/listener bindings, 264 retained files / 44,378,002 bytes, source/default-feature/executable identities, original client controls, dataset readback and all three original container CPU restorations. Namespace maps were preserved.

The same 64-worker, 4,096-key, 128-byte, batch-one/read-only protocol uses 128 warmup calls and a 1,500-ms measured window with original cap/deadline/drain rules. Both sides retain unchanged endpoint allocator-override records. Client CPUs 0–1; voters/Redis 2–5. This is shared-host volatile tmpfs and a standalone Redis reference, not equal durability, sustained capacity, cross-host or fault acceptance. Setup/lifetime memory and endpoint provenance are not interval-only peaks or continuous monitoring.

Candidate server SHA `ecdd5d414a2a01a11e0f2b300b1a6502fccc83efbfb9d746ee65a25770e59a9d`; manifest `790b581999d78b3f3d5edd9489930aff4f8a52624a5539cd7ce63256a4bcfa98`. Frozen driver `2fecebc3b07a3c5768bbc46eda40bd698d7f6352a1f41d91c2408cd553fb1e18`; auditor `ce8b82a872115b04cac50bdfeed900e34a80a12357a94f0e5910ae7d563a2021`.

Raw: `/tmp/kv9-typed-dispatch-matched-diagnostic-attempt1/cohorts`.
Independent evidence: `/tmp/kv9-typed-dispatch-matched-independent-first/results-first`.
No source, runtime, timing, predicate or input changes were made during this audit.

## Additional retained local evidence

- Focused checks: `/tmp/kv9-typed-point-dispatch-local-first/focused-summary.json`,
  SHA `f8732d406241b8b3e47a3d0132c49efa48cf121e497d544b5a70d2b11ab71543`.
- Process E2E: `/tmp/kv9-typed-point-dispatch-process-e2e-first/summary.json`,
  SHA `4aea668bfd5ae7d2ab72c501337843008aa917a325d4c58ed1eced25a2dc95c4`.
- Smoke matrix: `/tmp/kv9-typed-dispatch-comparison-smoke-first/matrix.json`,
  SHA `cb6d169b924203a576993fc09b2348c9c5ad803027028b0dd7608080209a8d20`.
- Independent audit: SHA `ea0df741912207dcc3c8e44e6f52a27eb575e4e9b015e1910e7fdcf27228575b`.
- Independent statistics: SHA `b84f8ea4cdd025450cb0574542559d1a77d17ef09f520e3046438f7065f19976`.
- Independent report: SHA `5feb588f6371ed637b34fd7699a6f4075d84847e2bb8a5727146c6faed90c204`.

All gates described above were local. No hosted CI was dispatched. Rejection
at performance screening is not full correctness acceptance or promotion.
