# Owner-local ReadIndex pump: measured, not selected

The experiment removes a redundant owner notification, but the measured benefit
is too small to justify promotion. Keep CRC `ca0002c7` selected. Pure GET improves
less than 1% pooled, while mixed throughput and mean latency slightly worsen.
These two short repetitions do not establish statistical significance or a
sustained capacity improvement. Do not spend a full promotion fault campaign
on this candidate without a stronger performance result.

Candidate [3bfb63b](https://github.com/c4pt0r/kv9/commit/3bfb63bcb48e07325ab212d3e1e9d4eacb961b59)
is retained on `codex/owner-read-pump`. Its parent `eecb1aaf` has the selected
CRC runtime; intervening build-safety and documentation changes do not select
a different database implementation. The candidate's initial design document
records its pre-validation state; this report records the completed gates.

## Matched GET results

| Metric | CRC control | Owner-local pump | Redis |
| --- | ---: | ---: | ---: |
| c1 calls/s | 26,518.453 | 26,640.845 | 174,419.231 |
| c1 mean us | 37.590 | 37.425 | 5.656 |
| c1 p99 interval us | 50.176–50.687 | 49.664–50.175 | 7.232–7.295 |
| c64 calls/s | 345,145.331 | 348,189.728 | 514,041.171 |
| c64 mean us | 185.303 | 183.681 | 124.394 |
| c64 p99 interval us | 352.256–356.351 | 352.256–356.351 | 229.376–231.423 |

C1 throughput changes **+0.462%** pooled; paired repetitions are **+0.800% /
+0.123%**. C64 throughput changes **+0.882%** pooled; paired repetitions are
**+1.216% / +0.549%**. C64 GET p99 stays in the same histogram bucket in both
repetitions. The candidate remains **1.476x** behind Redis throughput at c64,
and its c1 mean latency is **6.616x** Redis.

## Mixed traffic, with GET separated from PUT

| c64 metric | CRC control | Owner-local pump | Change |
| --- | ---: | ---: | ---: |
| Combined calls/s | 172,056.321 | 171,638.135 | -0.243% |
| Combined mean us | 371.831 | 372.736 | +0.243% |
| GET mean us | 384.041 | 384.781 | +0.193% |
| PUT mean us | 359.644 | 360.715 | +0.298% |
| GET p95 interval us | 524.288–532.479 | 532.480–540.671 | Higher pooled bucket |
| GET p99 interval us | 622.592–630.783 | 622.592–630.783 | Same bucket |
| Combined p99 interval us | 606.208–614.399 | 614.400–622.591 | Higher in both repeats |

C64 mixed GET and PUT means each rise in both repetitions. C1 mixed combined
throughput falls 0.160% pooled. These small shifts are screening observations,
not proof of a universal regression. They do not support an overall improvement.
[All pooled populations](owner-read-pump-v1/READOUT.md) and
[every repetition](owner-read-pump-v1/PER-REPEAT.md) preserve the complete results.

## Protocol and acceptance boundary

There are 24 timed cohorts: c1/c64, point read50/read100, CRC/candidate/Redis,
two forward/reverse repetitions of ten seconds each. Separate two-second smoke
cohorts cover the 12 forward descriptors. Fixed native/Redis v3 client source is
`0be806d9671e2c50701a64aa7889c8859b7648ba`. All roles use 4,096 keys plus a sentinel,
128-byte values, seed 71, 128 warmup calls, closed-loop load, a 1,500 ms original
deadline, six maximum attempts and the unchanged ten-million-call cap.

The shared-host loopback placement gives clients CPUs 0–1, voters 2–5 and
helpers/owned containers 6–15,22–31. KV9 uses three voters, fresh Safe ReadIndex
and normal WAL sync calls on volatile **tmpfs**. Redis is standalone, with
save/AOF disabled and no pipelining. There is no equivalent write-durability,
physical-disk, cross-host/NIC, open-loop overload or full batch-matrix claim.
The older admission-window studies are separate runs, not causal comparisons
between their candidate speeds and this one.

All **49,860,741 measured calls = issued = attempts = successes**. Other measured
outcomes and dropped slots are zero. Across all phases there are 50,158,797
successful calls and 50,158,813 attempts, including 16 initialization routing
attempts. Native attempt-reason maps cover native calls; Redis attempts remain
included in aggregate accounting. No aggregate benchmark is a complete concurrent
linearizability history.

The independent audit accepts 80 exited lifetimes, 48 fresh drains, 48 voter
writer/listener bindings, 4,678 resource samples and 2,353 source-file checks.
All three owned containers restore exact configured/effective CPU placement;
historical namespace UID maps are preserved. The full local inventory retains
636 files and 4,564,431,479 bytes. Builds, tests, proofs, faults, profiling and
audits did not overlap the timed run. Root timing session `16901` and final
audit session `64504` both terminate with exit 0.

## Correctness gates

- 714 workspace tests/doctests pass, with 23 ignored; formatting and
  warnings-denied all-target Clippy pass. All 601 source files match the clean
  committed tree. The retained release recompiles all 11 first-observed project
  units after explicit invalidation under the shared-target lock.
- Five new tests exercise real three-voter confirmation/apply, the actual owner
  park boundary, external synchronous wakeups, a 65-request retained suffix,
  and a real follower ACK published during the submit/pump callback.
- Three compiled semantic-control triples pass their baseline, intended failure
  and restored source. The first broad omit-pump mutation failed during election
  setup; that rejected attempt is retained. A corrected mutation skips pump only
  after successful read admission and reaches the intended assertion. No compile
  error or fixture failure is accepted as that semantic control.
- The local TLA+/TLAPS gate passes 21 cases, seven named theorems and 20 baseline
  obligations. Two semantic model/proof controls and proof-hole/custom-axiom
  controls are checked. Its boundary is ordinary-return sequencing and retained
  notifications in one same-owner transaction, including typed failure and abort.
  Pump invocation does not mean quorum confirmation or successful read. Whole
  Rust/Raft composition, scheduler liveness and core implementation proofs remain
  separate obligations.
- Exact-source ordinary stream/unary recovery passes two complete atomic/point
  histories across leader loss and original-directory restart: 365 operations,
  337 successes and 28 unknown outcomes. Five server and two client lifetimes
  exit, and both histories independently validate with their unknown outcomes.
  This is local SIGKILL/restart evidence, not actual Chaos Mesh or power loss.

The source/proof gates, including rejected attempts, release provenance, recovery
histories and matched-screen reporting inputs are published in the
[retained evidence bundle](owner-read-pump-v1/README.md). Its byte verifier does
not rerun proofs, histories or runtime acceptance.

## Next experiment and product route

The isolated owner-turn test proves the notification was redundant in that
scenario; the matched screen does not establish it as a major client-visible
cost. Keep this candidate unselected. Before another scheduling change, profile
the selected source's c1 read turnaround and c64 mixed path, separating CPU work
from time blocked on transport/owner/completion handoffs. Use the same source,
clients and workload cells, with profiling outside uninstrumented timing. Select
one measured cost for the next bounded change; avoid another broad wake rewrite
or admission-window change based only on code appearance.

Retain fresh quorum authorization, sealed membership, deadline/cancellation
ownership and full pump/apply/read-view fences. Qualify useful changes with the
full point/batch matrix and exact-source actual Chaos Mesh before promotion.
Tonic streaming remains selected; DPDK requires real NIC/cross-host evidence.
After the Redis-class read milestone, proceed through bounded dynamic
RegionManager/multi-Raft (#22), epoch routing (#23), recoverable membership (#24),
automatic split intent/ownership/recovery (#25), then placement/scaling (#27).
Metadata and scheduling may not become service-critical singletons; only object
store is exempt. TLS remains later work. All CI here is local; no hosted workflow
was dispatched.
