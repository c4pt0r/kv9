# ThinLTO improves point-read and mixed performance

Candidate `02d0c01024b65a84b220c6948ff2224bfa7900bc` improves throughput,
mean latency and p99 in all four measured cells and both run orders. Its change
from parent `40f014f` is release code generation: ThinLTO and one codegen unit.
The measured CRC control predates that parent's opt-in read-stage observer;
the observer is disabled in both production builds. The result belongs to the
complete pinned candidates, without an instruction-level equivalence claim for
their default builds. Consensus semantics and the fixed v3 clients are unchanged.

This is a favorable experimental checkpoint. Selected runtime remains CRC
`ca0002c7` pending broader point/batch performance qualification. The exact build
now passes the [21-window actual Chaos matrix](RELEASE-THIN-LTO-CHAOS.md); that
correctness run adds no new throughput or latency measurement.
The read-performance milestone remains open before dynamic multi-Raft and
automatic range splits. [Source and recovery validation](RELEASE-THIN-LTO-VALIDATION.md)
has passed within its stated scope.

## Complete same-run comparison

Both ten-second repetitions are pooled by actual counts and elapsed time.
Quantiles are intervals from merged raw histogram buckets, not averages of
percentiles. Mixed throughput counts GET and PUT together.

| Metric | Selected CRC | ThinLTO | Redis |
| --- | ---: | ---: | ---: |
| c1 GET calls/s | 26,591.615 | **28,293.988** | 174,022.103 |
| c1 GET mean us | 37.485 | **35.226** | 5.671 |
| c1 GET p99 us | 50.176–50.687 | **45.056–45.567** | 7.424–7.487 |
| c64 GET calls/s | 346,695.956 | **376,202.163** | 510,898.184 |
| c64 GET mean us | 184.474 | **169.998** | 125.158 |
| c64 GET p99 us | 352.256–356.351 | **323.584–327.679** | 229.376–231.423 |
| c1 mixed calls/s | 18,968.689 | **21,603.659** | 172,140.336 |
| c64 mixed calls/s | 170,992.289 | **188,817.154** | 500,132.085 |
| c64 mixed GET mean us | 386.171 | **350.297** | 127.850 |
| c64 mixed GET p99 us | 630.784–638.975 | **565.248–573.439** | 233.472–235.519 |
| c64 mixed PUT/SET mean us | 362.143 | **327.347** | 127.810 |
| c64 mixed PUT/SET p99 us | 598.016–606.207 | **540.672–548.863** | 233.472–235.519 |

Against CRC, c1 GET throughput rises **6.402%** and mean falls **6.025%**;
c64 GET throughput rises **8.511%** and mean falls **7.847%**. Mixed throughput
rises **13.891% at c1** and **10.424% at c64**. Separate mixed GET and PUT means
and p99 improve in both repetitions; no mixed-operation regression is concealed
by combined throughput. The complete per-repeat tables accompany the evidence.

ThinLTO reaches **73.635% of Redis c64 GET throughput**, with a **35.826% higher
mean**. Isolated GET mean is still approximately **6.21 times Redis**. This
improvement does not establish Redis parity, sustained capacity or statistical
significance from two short repetitions.

## Method and acceptance

The premeasurement contract fixes 12 separate two-second smoke cohorts followed
by all 24 timed cohorts: c1/c64, point read100/read50, CRC/ThinLTO/Redis, two
opposite orders. The workload uses 4,096 keys and 128-byte values. Default
production builds have no read-stage instrumentation; timed native and Redis
clients remain the original `0be806d9` v3 artifacts. No candidate-trained or
recompiled client participates.

Clients use CPUs 0–1, voters 2–5 and helpers 6–15,22–31. Compilation, compression
and other runtime work finish before timing. This is a shared-host loopback
comparison: three-voter KV9 retains ordinary quorum and sync calls on **tmpfs
WAL**, while standalone Redis has persistence and pipelining disabled. It is
not equal durability, physical-disk latency, independent-host failure or NIC
capacity evidence. DPDK is not implicated by this result.

All 12 smoke and 24 timed cohorts complete on their original executions.
The independent audit accepts **50,708,100 measured calls**, all successful
with one attempt and zero dropped slots. Initialization routing attempts remain
separately accounted. It checks **80 exited process lifetimes**, **48 fresh
drains and voter bindings**, **4,681 resource samples**, and **642 retained
files / 4,786,432,855 bytes**. Configured/effective CPU settings and all three
owned container namespaces are restored. No incomplete cohort is pooled.

Original terminals: smoke `4074/0`, timing `79984/0`, audit `73017/0`;
statistics receipt `0f5422/0`. Seven driver/binding, 17 auditor and the finite
statistics preparation contract pass. The unchanged statistics core binds all
24 reports to the accepted inventory and validates histograms before pooling.

## Interpretation and next work

The result supports this release-build candidate. Smaller generated code and
cross-unit optimization are plausible mechanisms; no new CPU profile isolates
their contributions from the source ancestry described above. The retained server is
16,141,024 bytes versus 20,136,472 for CRC. This is not a cold-build-time or
runtime-memory comparison. Compiler correctness remains a premise.

1. Freeze this source, binary and complete result. Extend the existing matched
   protocol to the broader point/batch API matrix, retaining separate operation
   counts, means, p99 and equivalent acknowledgement boundaries.
2. The exact-build [21-window Chaos gate](RELEASE-THIN-LTO-CHAOS.md) now passes:
   9,923 complete history operations, positive effects, fresh drains and owned
   cleanup. Preserve this scoped result and the original failed delay-selection
   attempt. Dedicated link/FSYNC and broader industrial fault obligations remain
   separate; ordinary process recovery does not substitute for actual injection.
3. If those gates pass, promote the code-generation setting as its own change;
   evaluate combination with `42e0117` separately. Its historical notification
   improvement cannot be added to this percentage.
4. Continue reducing isolated read latency with the
   [quorum-path investigation](QUORUM-LATENCY-NEXT.md), using the selected
   implementation. Consult the [experiment index](PERFORMANCE-EXPERIMENT-INDEX.md)
   before another scheduling experiment. Preserve fresh Safe ReadIndex, sealed
   groups, exact contexts, successful whole-pump completion, apply/view fences,
   durable ACKs and bounded cancellation/deadline/admission ownership.

Core implementation proofs, bounded storage, full actual fault acceptance and
no service-critical singleton except object storage remain open. CI ran locally;
no hosted workflow was dispatched.

## Immutable evidence

[Original qualification and performance bundle](https://github.com/c4pt0r/kv9/blob/fb82a3b67def2d7cfa67071391ba07682d23d584/docs/release-thin-lto-performance-v1/README.md):
1,243 original files / 212,101,156 decoded bytes / five bounded archive parts.
The integrity-only verifier passes; [pooled tables](https://github.com/c4pt0r/kv9/blob/fb82a3b67def2d7cfa67071391ba07682d23d584/docs/release-thin-lto-performance-v1/READOUT.md)
and [every repetition](https://github.com/c4pt0r/kv9/blob/fb82a3b67def2d7cfa67071391ba07682d23d584/docs/release-thin-lto-performance-v1/PER-REPEAT.md) are the
unchanged accepted statistical output. Runtime binaries/WALs and bulky host
observations remain local under their original inventories.

[Historical WAL cold-retention overlay](https://github.com/c4pt0r/kv9/blob/fb82a3b67def2d7cfa67071391ba07682d23d584/docs/benchmark-cold-retention-v1/README.md):
552 WAL files / 12,241,339,982 logical bytes are cold, with an exact-byte pilot
restore verified. Net allocated savings are about 4.00 GiB, excluding transaction
metadata. Cold original paths must be rehydrated before old full audits; this
supersedes earlier residency descriptions. The [publication notes](https://github.com/c4pt0r/kv9/blob/fb82a3b67def2d7cfa67071391ba07682d23d584/docs/benchmark-cold-retention-v1/PUBLICATION-NOTES.md)
retain metadata failures and subsequent bounded cache reclamation. Current
ThinLTO originals remain resident. No benchmark predicate or storage floor was
lowered; no old payload was discarded.

[Further CRC72 retention and large-file qualification](https://github.com/c4pt0r/kv9/blob/7d869009602919748a8cd8e920dade305e3072fb/docs/thin-lto-capacity-v1/README.md)
records 554 additional cold files, the seven-file real restore pilot and actual
1 GiB-plus synthetic qualification. Those old CRC72 full audits now require
rehydration; current ThinLTO timing evidence remains resident. This environment
work supplies no new performance measurement and does not establish full-matrix
storage capacity.

Evidence lives on a separate immutable commit so subsequent main source
snapshots do not repeatedly absorb another archive. Reports and the experiment
index keep the cross-branch evidence discoverable.
