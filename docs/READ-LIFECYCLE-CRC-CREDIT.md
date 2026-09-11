# Read lifecycle: CRC versus one-context admission

Updated: 2026-09-11. Tracking: #9, #13 and #20.

The fresh diagnostic supports testing a two-context local ReadIndex window.
One-context admission reduces confirmation work under mixed traffic but adds
more queueing than it saves. It remains experimental. The selected runtime is
CRC; the latest uninstrumented throughput/latency comparison remains
[READ-CREDIT-CRC-PERFORMANCE.md](READ-CREDIT-CRC-PERFORMANCE.md).

## Successful sampled read intervals

Each row is one five-second instrumented cohort. Means are calculated from
integer histogram sum/count deltas across the same three voter lifetimes.
All intervals are microseconds; samples cover the drained whole-client envelope,
including initialization, warmup and verification, not just timed measurement.

| Source and workload | Samples | Queue | Invocation to observed confirmation | Confirmation to send | Notification | Total |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| CRC, c1 GET | 2,229 | 1.438 | 20.887 | 0.284 | 1.676 | 24.285 |
| One context, c1 GET | 2,139 | 1.443 | 20.767 | 0.277 | 1.700 | 24.186 |
| CRC, c64 GET/PUT 50% | 6,759 | 19.893 | 168.753 | 28.738 | 56.070 | 273.454 |
| One context, c64 GET/PUT 50% | 6,972 | 82.557 | 142.385 | 20.476 | 53.604 | 299.022 |

For mixed reads, queue mean increases **62.664 us**, while observed confirmation,
apply and notification together save **37.096 us**. Total sampled mean rises
**25.568 us (9.350%)**. Queue share rises from **7.275% to 27.609%**. Admitted
members per group increase from **3.144 to 10.241**, showing the batching benefit
and the waiting tradeoff in the same observation.

At c1, observed confirmation accounts for about 86% of the sampled barrier.
This recording does not establish a meaningful isolated-request improvement.
The intervals include owner scheduling and observation delay: they are not pure
network RTT, isolated protocol-credit waiting, or backend/RPC execution time.
There is no per-request correlation with the client latency histogram. Do not
subtract these means from another run's client mean or add stage percentiles.

## Sources, setup and retained evidence

Both diagnostic ports contain the same sampled instrumentation, approximately
one request in 64, without changing their parents' admission/authority decisions:

- CRC: [aee183ef](https://github.com/c4pt0r/kv9/commit/aee183effb1fe4ddf0733eed3844d91255f332b1), parent `86a689c8`.
- One-context admission: [f19fdfb0](https://github.com/c4pt0r/kv9/commit/f19fdfb014699da98ec45f790cd26c155b1d5461), parent `7fccd8ad`.
- Fixed v3 measurement client: `0be806d9671e2c50701a64aa7889c8859b7648ba`.

The fixture uses three voters, normal Raft quorum/sync behavior on **tmpfs WAL**,
loopback, 4,096 keys plus a sentinel, 128-byte values, seed 71 and 128 warmup
calls. It is closed loop with a 1,500-ms deadline and six-attempt ceiling.
Clients use CPUs 0-1, voters 2-5 and helpers/owned containers 6-15,22-31.
Public admission is 64 calls/16 MiB in this fixture, with 128 async read slots;
the production encoded-byte default remains 64 MiB. No CPU profiling, builds,
tests, faults or audits overlap timing. This shared-host observation is not a
physical-disk or cross-host NIC capacity measurement.

The independent readback accepts all four cohorts: **1,986,314 measured calls**,
all successful in one attempt. Across all client phases there are **2,035,990
successful calls / 2,035,994 attempts**; the four extra attempts are initialization
NotLeader responses. All **16 owned process lifetimes** exit. The reader checks
24 metric documents, 12 fresh drain documents, 12 voter/listener bindings and
restoration of all three owned container CPU sets. The full retained runtime
inventory is 156 files / 1,004,813,947 bytes, including local WAL evidence.

Runtime session `63422` exits 0 (`c06869`); independent reader session `94688`
exits 0 (`632a75`). Accepted analysis SHA-256:
`eef7366f18adf541fde5cceb9a1443a7891d559439e3a083b8debaaf6027abd1`.
The arithmetic review independently reproduces the original integer deltas.
Its first script used an incorrect configuration path; that failed derivation
and the corrected attempt are retained. No timing was rerun for the correction.

The [compact publication bundle](read-lifecycle-crc-credit-v1/README.md) includes
raw reporting inputs, original-byte inventories, runner/readback receipts and
source-gate logs. It omits binaries, WAL and full host/container observations.
Its portable verifier checks bytes and arithmetic; it does not repeat the full
runtime acceptance, execute proofs, establish linearizability or run Chaos Mesh.
All work in this checkpoint ran locally; no hosted workflow was dispatched.

## Next experiment and development path

[5654ea59](https://github.com/c4pt0r/kv9/commit/5654ea593fe561c5fd8d800cf498f716eb5bca23)
raises local ReadIndex occupancy from one to two. It retains fresh Safe ReadIndex,
sealed group identities, independent confirmations, cancellation/deadline
ownership and successful pump/apply/read-view fences. It contains no diagnostic
hooks. See its [design and proof scope](https://github.com/c4pt0r/kv9/blob/5654ea593fe561c5fd8d800cf498f716eb5bca23/docs/READ-INDEX-WINDOW.md).

Local verification passes **718 workspace tests/doctests (23 ignored)**,
formatting, warnings-denied Clippy and **14 compiled semantic-control triples**.
All 42 selected Raft test units are freshly compiled. The new counted gate
passes 29 cases, nine theorems and 23 baseline obligations. The prior admission
gate passes 37 cases, 28 theorems and 265 baseline obligations. These abstractions
are not a composed implementation proof. In particular, authenticated remote
MsgReadIndex can bypass the local gate: the always-bounded counter theorem
assumes every queue-increasing transition follows guarded local admission.
A universal ingress bound and its fairness behavior remain open work.

The subsequent exact-source ordinary three-voter recovery also passes. Stream
and unary histories contain 184/188 operations, 171/176 successes and 13/12
unknown outcomes respectively. Both independent history checks are valid;
leader loss, surviving-quorum progress, restart and fresh drains are observed.
All five server and two client lifetimes exit. Runtime session `86371` exits 0
(`75ae84`), followed by independent audit exit 0 (`822904`). Original source,
release and runtime inputs remain unchanged. The bundle includes the complete
histories and checker outputs. This is one-host SIGKILL/restart evidence, not
actual Chaos Mesh or power loss.

The candidate has no accepted performance result yet. Next: matched
uninstrumented c1/c64 point read and mixed screening, followed by the full
point/batch matrix if the candidate is promising.
Evaluate GET separately from PUT in mixed traffic. Reject a throughput gain that
reintroduces repeatable read-tail regressions. Complete exact-source Chaos Mesh
histories and the applicable proof mapping before promotion.

Prioritize single GET turnaround, loaded GET throughput and mixed read tails.
Preserve write durability; evaluate real disk costs with an equivalent quorum/
persistence reference. The Redis read milestone remains open. After it, proceed
to dynamic multi-Raft (#22), routing and replica attachment (#23/#24), automatic
splits (#25) and placement (#27), retaining snapshot/retention prerequisites and
the no-service-singleton requirement. Detailed order remains in
[RAWKV-PERFORMANCE-PATH.md](RAWKV-PERFORMANCE-PATH.md).
