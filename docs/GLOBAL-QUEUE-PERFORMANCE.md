# Global queue polling experiment — 2026-09-11

**Reject the fixed-interval-eight scheduler candidate.** The complete matched
screen shows lower c64 GET throughput and worse mean/p99 latency in both run
orders. The measured scheduling hypothesis did not improve the database.
Selected runtime remains CRC `ca0002c7`; no runtime change is merged on main.

## Same-run throughput and latency

| Metric | Selected CRC | Interval-eight candidate | Redis |
| --- | ---: | ---: | ---: |
| c1 GET calls/s | 26,542.953 | 26,391.062 | 174,382.402 |
| c1 GET mean us | 37.562 | 37.779 | 5.658 |
| c1 GET p99 us | 49.664–50.175 | 50.176–50.687 | 7.296–7.359 |
| c64 GET calls/s | 346,069.116 | 337,799.779 | 511,088.898 |
| c64 GET mean us | 184.807 | 189.335 | 125.114 |
| c64 GET p99 us | 352.256–356.351 | 356.352–360.447 | 231.424–233.471 |
| c64 mixed combined calls/s | 170,326.223 | 168,640.129 | 503,781.883 |
| c64 mixed GET mean us | 387.535 | 390.292 | 126.912 |
| c64 mixed GET p99 us | 630.784–638.975 | 630.784–638.975 | 233.472–235.519 |
| c64 mixed PUT/SET mean us | 363.691 | 368.443 | 126.897 |
| c1 mixed combined calls/s | 18,832.735 | 18,791.621 | 172,507.698 |

C64 GET throughput falls **2.390%** and its mean rises **2.450%**. Both complete
repetitions have lower throughput, higher mean and a worse p99 bucket than their
CRC control. C1 GET throughput also falls in both repetitions: pooled **-0.572%**,
with **+0.578%** mean; its p99 ties in the first repetition and worsens in the
second. C64 mixed throughput falls **0.990%**, with higher GET and PUT means in
both repetitions. Its mixed GET p99 worsens in the first and ties in the second;
pooled equality is not evidence that both repetitions have equal tails.
C1 mixed changes direction across repetitions.

All four cells and both run orders are retained in the
[complete readout](global-queue-performance-v1/READOUT.md) and
[per-repeat tables](global-queue-performance-v1/PER-REPEAT.md). Rates use summed
calls over complete cohort duration; means use integer duration sums/counts;
quantiles merge the original histogram bins. No percentile is averaged and no
partial or favorable subset is substituted for the complete result. Two short
repetitions do not establish statistical significance or sustained capacity.

## Fixed experiment and acceptance

Candidate [3338650](https://github.com/c4pt0r/kv9/commit/33386507d0c068d0243451d44c70b0110e314b4e)
sets `.global_queue_interval(8)` on the existing two-worker RPC executor. The
event interval remains eight. Timed builds have **default features**: the
optional read-stage observer and testing seams are absent. The earlier
[instrumented stage diagnosis](READ-STAGE-RESULTS.md) motivated the hypothesis;
its measurements are not mixed into this uninstrumented screen.

The protocol is unchanged: **12 smoke and 24 timed cohorts**, c1/c64 point GET
and 50% GET/PUT, two opposite ten-second run orders, fixed v3 clients, 4,096
data keys plus sentinel and 128-byte values. Clients use CPUs 0–1, three voters
or Redis use 2–5, and helpers/owned background containers use 6–15/22–31.
The host is shared. KV9 keeps ordinary three-voter quorum and synchronous WAL
calls on **tmpfs**; Redis is standalone with persistence and pipelining disabled.
These systems do not provide equal durability. This is not a real-disk,
cross-host, independent-machine failure or sustained-capacity comparison.

All **49,504,037 measured calls** succeed in one attempt, with zero dropped slots.
Full phase accounting retains initialization routing attempts. The independent
audit accepts **80 exited lifetimes**, **48 fresh drains**, **48 writer/listener
bindings**, **4,678 resource samples**, and **636 retained files / 4,507,888,898
bytes**. Source/process/namespace identities, complete values, all outcomes,
CPU placement and exact restoration pass. No cohort is repeated or omitted.
Seven driver/binding and 17 auditor contracts, plus the unchanged statistics
contract, pass before their respective runtime/derivation gates.

[Local correctness and recovery](GLOBAL-QUEUE-VALIDATION.md) pass 435 default
tests/doctests (one existing ignored), formatting/Clippy and complete ordinary
streaming/unary histories with **363 operations: 330 OK / 33 unknown**. All 627
checked source files match the original retained release after first-party cache
invalidation. Candidate Chaos Mesh and broader API gates were not run; the
performance result already rejects this candidate.

Root reclaimed only **10,999** precisely selected obsolete debug intermediates
from 12 inactive old worktrees, recovering **6,508,384,256 available bytes**.
Exact metadata/hashes, live process references and cache locks were checked;
all original executable artifacts, WAL, histories, sources and locks remain.
None of the previous cleanup's 16,785 paths is repeated. Storage guards remain
32/96 GiB before each cohort and 16/64 GiB during execution. Reclamation is
resource preparation, not a performance optimization.

## Decision and next work

The [read-stage diagnosis](READ-STAGE-RESULTS.md) still establishes a large c1
quorum interval and c64 completion/resumption interval. This experiment does
**not** establish that the global injection queue caused that interval, nor that
more frequent polling is beneficial. Do not stack this rejected setting onto
future candidates or spend candidate Chaos qualification on it without a new
reason.

Next, inspect the point-stream per-request task/wakeup path and the quorum
message round trip. A bounded experiment that changes actual task handoffs or
transport work must retain all cancellation, deadline, backpressure, sealed
ReadIndex, successful pump/apply/view and durable acknowledgement semantics.
Use the selected CRC configuration as the control. The notification candidate
`42e0117` remains a separate modest loaded-read experiment with its existing
qualification limits; this screen does not compare it directly with `3338650`.

The read target remains open before dynamic multi-Raft and automatic range
splits. Full implementation proofs, actual Chaos/host failure and bounded
storage requirements remain open. All validation is local; no hosted CI was
dispatched. The [evidence bundle](global-queue-performance-v1/README.md) preserves
original source, recovery, protocol, runtime, failed hypothesis and statistics.
