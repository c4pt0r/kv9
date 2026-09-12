# Coalesced owner notification performance

The notification candidate improves loaded reads modestly in the completed
matched screen: c64 GET throughput rises **1.123%**, and c64 mixed throughput
rises **2.730%**, with better loaded and mixed-read latency in both repetitions.
Single-request GET does not improve: throughput falls **0.248%** and mean
latency rises **0.260%**. Keep the candidate experimental; selected runtime
remains CRC `ca0002c7`. The Redis-class read milestone remains open.

Candidate [42e0117](https://github.com/c4pt0r/kv9/commit/42e0117b13bed9671b672436c6b36bfe63c462af)
only suppresses `WorkSignal::notify_one` while its mutex-protected pending bit
is already true. Its [formal/source/recovery validation](COALESCED-OWNER-VALIDATION.md)
passes. No executor, queue, read authority, quorum, durability or timer change
is included. The prior rejected outbound executor is not part of this candidate.

## Same-run results

Rates and means pool both forward/reverse repetitions using actual elapsed
time and operation counts. P99 values are merged raw histogram bucket intervals;
no percentile is averaged. All rates are calls/s, all latencies are microseconds.

| Metric | CRC control | Notification candidate | Redis |
| --- | ---: | ---: | ---: |
| c1 GET calls/s | 26,461.551 | 26,395.976 | 174,159.224 |
| c1 GET mean | 37.674 | 37.772 | 5.665 |
| c1 GET p99 | 50.176–50.687 | 50.176–50.687 | 7.360–7.423 |
| c64 GET calls/s | 345,626.302 | 349,507.003 | 513,504.422 |
| c64 GET mean | 185.046 | 182.990 | 124.523 |
| c64 GET p99 | 352.256–356.351 | 331.776–335.871 | 229.376–231.423 |
| c64 mixed combined calls/s | 171,594.529 | 176,278.501 | 503,716.521 |
| c64 mixed GET mean | 384.744 | 375.026 | 126.920 |
| c64 mixed GET p99 | 622.592–630.783 | 606.208–614.399 | 233.472–235.519 |
| c64 mixed PUT/SET mean | 360.940 | 350.838 | 126.921 |
| c1 mixed combined calls/s | 18,915.512 | 18,971.023 | 172,064.978 |
| c1 mixed GET mean | 47.197 | 47.118 | 5.718 |

Both c64 GET repetitions improve throughput and mean/p99, as do c64 mixed
throughput and its separate GET mean/p99. C1 GET throughput is slightly lower
in both repetitions; its p99 improves in the first and worsens in the second.
C1 mixed throughput improves in the first repetition and declines slightly in
the second. The [complete readout and per-repeat tables](coalesced-owner-performance-v1/README.md)
retain these directions and every GET/PUT/SET population.

The candidate reaches about 68.1% of Redis c64 GET throughput (a 1.469x gap).
C1 GET mean remains about 6.67x Redis. Notification coalescing helps the loaded
path but does not resolve isolated-request latency. These are short shared-host
measurements, not statistical significance or a general no-regression bound.

## Workload, correctness and resources

The unchanged four-cell protocol crosses concurrency 1/64 with read-only and
50/50 point GET/PUT. There are 12 smoke cohorts followed by 24 ten-second timed
cohorts in two opposite run orders, with 4,096 keys and 128-byte values. Fixed
v3 clients and the original CRC executable are retained. Candidate and CRC use
the same Rust 1.94.0 compiler and ordinary three-voter Raft quorum/sync calls.

This is shared-host loopback with **tmpfs WAL**. Redis is standalone with
persistence and pipelining disabled. It is not a durability-equivalent write
comparison, a real-disk measurement, a cross-host result or sustained capacity.
Clients use CPUs 0–1, voters/Redis use 2–5, and helpers use 6–15/22–31. The
original isolated-container wrapper and all storage guards are unchanged.

All **49,944,895 measured calls** succeed in one attempt, with no dropped slots.
Full phase accounting retains initialization routing attempts separately. The
independent audit accepts all 24 cohorts, 80 exited lifetimes, 48 qualifying
drains, 48 voter writer/listener bindings, 4,679 resource samples and exact
configured/effective CPU and namespace restoration for the three containers.
All seven driver/binding and 17 auditor contracts pass. No runtime cohort was
rerun or omitted.

Before smoke, root reclaims 8,739,483,648 available bytes from 16,785 precisely
selected obsolete debug `.rlib`/`.rmeta`/incremental files in seven old kv9
worktrees. The selection excludes executables, hardlinks, sources, logs,
manifests, locks, all release trees, evidence and WALs. A privileged reference
scan under preserved cache locks checks that only the cleanup owner's own lock
descriptors are allowed. Shared current build caches and crate archives remain.
The preflight 96 GiB retention/32 GiB tmpfs and runtime 64 GiB/16 GiB guards
remain unchanged. This reclaims reproducible intermediates, not experiment data.

Original timing: root session 58345, exit 0 (`b236db`). Independent audit:
63127, exit 0 (`995b0b`). Statistics: exit 0 (`c9b7df`). The reporting archive
preserves selected original bytes and the complete numerical readouts; raw WAL,
executables and bulky host observations remain locally retained under their
original inventories. Integrity verification does not rerun acceptance.

## Next development path

1. Keep `42e0117` frozen as a modest loaded-read candidate. Complete applicable
   broader point/batch API measurements and actual exact-source Chaos Mesh
   acceptance before considering default promotion. Neither this screen nor
   ordinary process recovery closes the candidate Chaos gate.
2. Prioritize isolated GET latency next: localize the remaining serial RPC,
   owner-service and completion-wait costs with a bounded diagnostic. This
   change gives no evidence that another worker or transport sweep would help.
   Retain fresh Safe ReadIndex, sealed groups and successful pump/apply/view
   fences; no read lease or weaker acknowledgement is introduced.
3. Keep read performance ahead of dynamic multi-Raft, epoch routing/membership
   and automatic range splits. Preserve their existing storage/recovery/proof
   prerequisites and the no-service-critical-singleton requirement except for
   object storage. DPDK still requires cross-host/NIC evidence.

All CI and acceptance ran locally. No hosted workflow was dispatched.
