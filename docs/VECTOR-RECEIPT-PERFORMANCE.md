# Vector receipt lookup: mixed gain, no general promotion

Candidate [7c8cd25](https://github.com/c4pt0r/kv9/commit/7c8cd25e9531809c9d5bd40faee116a7d6d5a3ff)
keeps the original vector push-then-drain retention and changes only receipt
lookup to binary search under a checked ordering certificate. The completed
screen improves c64 mixed throughput **2.589%**, with lower GET and PUT mean
and p99 in both repetitions. Pure c64 GET throughput falls **0.349%** and c1
throughput/mean also regress. Keep CRC selected; retain this mixed-traffic
candidate without general promotion or a claim of statistical significance.

## Same-run throughput and whole-call latency

Each row pools both ten-second forward/reverse repetitions. Percentiles are
merged raw histogram bucket intervals; no percentile or per-repeat QPS is
averaged. Mixed GET and PUT retain separate distributions.

| Metric | Selected CRC | Vector lookup | Redis |
| --- | ---: | ---: | ---: |
| c1 GET calls/s | 26,537 | 26,479 | 173,699 |
| c1 GET mean us | 37.571 | 37.650 | 5.680 |
| c1 GET p99 us | 50.688-51.199 | 49.664-50.175 | 7.296-7.359 |
| c64 GET calls/s | 345,613 | 344,406 | 508,212 |
| c64 GET mean us | 185.051 | 185.702 | 125.820 |
| c64 GET p99 us | 348.160-352.255 | 352.256-356.351 | 231.424-233.471 |
| c64 mixed combined calls/s | 171,677 | 176,121 | 502,276 |
| c64 mixed GET mean us | 384.797 | 374.367 | 127.309 |
| c64 mixed GET p99 us | 622.592-630.783 | 606.208-614.399 | 233.472-235.519 |
| c64 mixed PUT/SET mean us | 360.528 | 352.153 | 127.263 |
| c64 mixed PUT/SET p99 us | 589.824-598.015 | 573.440-581.631 | 233.472-235.519 |

C64 mixed QPS improves 2.861% / 2.317% in the two repetitions. Pure c64 QPS
falls 0.536% / 0.163%; its p99 is unchanged in the first repetition and worse
in the second. Pure c1 QPS falls 0.220% pooled and mean increases 0.212%,
although p95/p99 improve in both repetitions. C1 mixed QPS falls 0.809%, with
worse GET mean/p99. The [complete readout](vector-receipt-screen-v1/READOUT.md)
and [all repetitions](vector-receipt-screen-v1/PER-REPEAT.md) retain every row.

## What the experiment establishes

The [selected-source profile](READ-PATH-CPU-PROFILE.md) found 4.189% of mixed
CPU samples in the linear receipt-search loop. The previous
[deque/indexed candidate](INDEXED-RECEIPT-READ-MIXED.md) changed both lookup and
retention. This candidate preserves the exact vector push/drain order,
allocation policy and eviction behavior, isolating the source change to the
indexed lookup adapter and its ordering flag. Duplicate or decreasing indexes
permanently restore the original oldest-first iterator; full term/outcome and
absence remain observable exactly as before.

The mixed improvement remains useful, but this screen does not explain the
small pure-read shift or prove that the earlier deque caused it. Different
recordings do not establish a causal deque-versus-vector comparison. Normal
pure GET does not search write receipts. Repeating this held screen without a
new hypothesis is not the next development step.

## Validation and measurement scope

The exact source passes **213 Raft tests/doctests**, formatting and Raft
all-target Clippy with warnings denied. Both compiled semantic controls are
rejected by the intended assertions: unchecked ordering loses a retained
fence-rejection receipt; wrong-end eviction changes the retained sequence.
Each baseline and restored test passes. These are not compile-error kills.

The conditional local TLA+/TLAPS proof passes **13 parameterized theorems and
122 obligations per positive run**, with four positive runs and 14 controlled
cases. The explicit vector/deque sequence equality lemma bridges the actual
Vec transition to the inherited model variable. The pinned slice-search
contract and ordering certificate preserve the complete first match or
absence. This is not a whole-Rust/Raft, liveness, allocation or crash proof.
The preparation-only moving-HEAD comparison error is retained; it launched
no proof. No formal gate was rerun.

Ordinary stream/unary leader-loss and original-directory restart histories
pass independent checking: **351 calls, 321 OK, 30 unknown**. All unknowns
remain in the histories; all five server and two client lifetimes terminate.
This is process SIGKILL/restart, not actual Chaos Mesh or power-loss evidence.
The [correctness bundle](vector-receipt-correctness-v1/README.md) includes the
proof, source/build bindings, controls and complete process histories.
Full workspace and exact-candidate Chaos acceptance have not run.

The same v3 clients run 24 timing cohorts and 12 separate two-second smoke
cohorts: c1/c64, point read50/read100, CRC/candidate/Redis, 4,096 keys plus a
sentinel, 128-byte values, seed 71, 128 warmup calls and a 1,500-ms deadline.
All **49,724,855 measured calls = issued calls = attempts = successes**;
all other measured outcomes and dropped slots are zero. Across initialization,
warmup, measurement and verification, 50,022,911 calls succeed in 50,022,927
attempts; the extra 16 are initial leader-routing attempts.

The first independent audit accepts 80 exited lifetimes, 48 fresh drains,
48 writer/listener bindings, 2,348 role/source checks and 4,677 resource samples.
It checks 638 retained files totaling 4,614,768,209 bytes and exact restoration
of all three owned containers' CPU settings and namespace identities. The
[screen bundle](vector-receipt-screen-v1/README.md) preserves reports, raw
histograms, protocols, accepted audit and terminal/statistics records.

Clients use CPUs 0-1; voters or Redis share CPUs 2-5; helpers use 6-15,22-31.
KV9 retains three-voter Raft quorum and sync calls on volatile **tmpfs WAL**.
Redis is standalone, with persistence and pipelining disabled. This short
shared-host loopback screen is not equal-durability, disk, cross-host, sustained
capacity, full batch qualification or complete linearizability evidence.
No builds, proofs, tests, audits, profiles or fault runs overlap timing.
All CI remains local.

## Next implementation

Move to per-request RPC metadata allocation on the selected CRC base. The
earlier [immutable stream metadata adapter](STREAM-AUTHORIZATION-METADATA-SCREENING.md)
had a favorable isolated system-allocator GET screen and its own Chaos
acceptance. Combining it with the selected allocator/worker/CRC runtime is a
new experiment; old gains and fault acceptance do not transfer. Reuse only
the immutable authorization representation, authenticate every frame, and
retain immediate role/principal/revocation behavior. Validate current consumer
boundaries and source checks before recovery and the matched pure/mixed screen.

The new [integration bb13e43](https://github.com/c4pt0r/kv9/commit/bb13e4313c313ca910969d1f01ef862619b57906)
is implemented and pushed on its experiment branch. Its six Rust adapter files
and current metadata-independent consumer sections match the original adapter.
Fresh checks pass 228 default and 238 experimental server tests/doctests,
with one ignored in each overlapping configuration, formatting and experimental
server Clippy. A clean original default release binds all 596 tested sources
and 11 first-observed compiled units. The [source evidence](stream-metadata-crc-source-v1/README.md)
retains exact logs and release identities. Recovery, performance and actual
Chaos for this combination are pending; no combined gain is claimed.

A useful candidate still needs the full workload matrix and exact-source
actual Chaos Mesh before promotion. Fresh Safe ReadIndex, sealed groups,
full pump/apply/view fences, durable writes, deadlines and reservation ownership
remain required. Dynamic multi-Raft and automatic range splits follow the read
milestone; no issue #9 work package closes on this narrow experiment.
