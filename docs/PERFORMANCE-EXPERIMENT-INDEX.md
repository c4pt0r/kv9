# Performance experiment index

Consult this index and `git log --all` before proposing another experiment.
Several completed reports live on evidence branches rather than the current
main tree. Searching only current files missed prior rejected scheduling work.
That omission caused unnecessary implementation/check work in the latest turn;
the repeated prototypes are retained below and stopped before new timing.

Main now selects [ThinLTO `11113f6`](RELEASE-THIN-LTO-MAIN-INTEGRATION.md), with
the same executable bytes as qualified `02d0c01`. Each result below applies only to the
source, client, workload and duration recorded in its linked report. Historical
percentages must not be relabeled as current measurements. Accepted evidence
does not itself imply a selected candidate or full industrial qualification.

The current priority is [write performance against Redis with one primary and
two replicas](WRITE-PERFORMANCE-NEXT.md). The [matched baseline](WRITE-REDIS3-BASELINE.md)
now passes 12 smoke/24 timed cohorts and independent readback. At c64, selected
KV9 reaches 136,519.558 point writes/s and 887,520.285 BatchPut(64) items/s;
Redis WAIT 1/2 reach 229,760.166 / 232,465.084 point writes/s and about four
million batch items/s. Preserve KV9's 9.437–9.568-ms loaded batch p99 and the
volatile-storage/durability limits. This is a baseline, not a selected speedup.
The CRC reapplication passes its [actual 21-window Chaos campaign](WRITE-CRC-CHAOS.md)
and independent audit/cleanup: 9,833 complete operations. Its [matched write
comparison](WRITE-CRC-PERFORMANCE.md) now passes eight smokes and sixteen timed
cohorts: loaded BatchPut(64) improves 18.807% to 1,063,493.134 items/s, with p99
of 6.947–7.012 ms; loaded point writes improve 2.786% to 139,402.831/s.
Full point/batch and mixed-read regressions remain before default promotion.
The separate frame-buffer candidate now passes source, release, ordinary
recovery and [actual Chaos qualification](WRITE-RAFT-FRAME-BUFFER-CHAOS.md);
its [comparison preparation](WRITE-RAFT-FRAME-BUFFER-PERFORMANCE-PLAN.md) passes
28 local controls, but performance is still unmeasured pending capacity.
Read optimization remains held; no lease performance result is established.

The [leader-lease proof](LEADER-LEASE-PROOF.md) is a new design checkpoint:
eight TLAPS lemmas / 23 obligations plus real-clock containment show how to
remove per-read quorum RTT under explicit additional premises. The subsequent
[transition proof](LEASE-AUTHORITY-MODEL.md) adds 27 theorems / 348 obligations,
finite fault-model evidence and the fail-closed expiration contract. There is no
selected lease-enabled runtime or performance result yet. Do not relabel earlier Safe
ReadIndex timings as lease performance; preserve the implementation/proof/Chaos
gates and the selected `11113f6` baseline.
The subsequent [Rust controller](LEASE-CONTROLLER.md) passes local source/fault
controls and integer-timing proofs; its feature does not enable a server read path.
The [voting adapter](LEASE-VOTE-BINDING.md) adds durable policy/epochs and actual
election gates. The [renewal adapter](LEASE-RENEWAL-PUBLICATION.md) now adds exact
renewal/grant envelopes and whole-pump certificate publication. The
[read-view integration](LEASE-READ-VIEW.md) now passes local tests and source
fault controls for experimental GET/BatchGet over one retained applied view,
including real unary and streaming RPCs. Default startup remains Safe ReadIndex;
clock qualification and actual lease Chaos remain open. There is still no lease
performance measurement; existing selected-versus-Redis results remain the baseline.
The [explicit Linux clock](LEASE-CLOCK.md) now passes sampled-time arithmetic,
593 library tests and actual process-pause expiration. Its declared bounds still
require physical platform qualification and lease-enabled Chaos; this checkpoint
adds no throughput, latency or Redis comparison result.

| Change / source | Recorded decision and evidence |
| --- | --- |
| Segmented WAL vectored writes `cfd9c92` | Experimental on selected ThinLTO, separate from CRC and the Raft frame-buffer candidate. Three universal SMT checks, three countermodels, 714 workspace tests/doctests (23 existing ignored), formatting and Clippy pass. An actual three-append file/replay probe records one `writev` plus the existing `fsync` per frame. Release/recovery/actual Chaos and performance remain pending. [Source qualification](https://github.com/c4pt0r/kv9/blob/cfd9c927f8ecd33974100f696e6b08b227d25a41/docs/WRITE-SEGMENT-VECTORED.md). |
| Single-buffer Raft WAL frame `01d128f` | Experimental on selected ThinLTO, separate from CRC. Removes one allocation/body copy. Three source-bound universal SMT checks, three counterexample controls, 710 workspace tests/doctests (23 existing ignored), formatting and Clippy pass. New compatibility test writes/replays 3,840 frames. [Exact release and 353-operation recovery](WRITE-RAFT-FRAME-BUFFER-RECOVERY.md) and [actual 21-window Chaos](WRITE-RAFT-FRAME-BUFFER-CHAOS.md) pass: 9,818 complete Chaos operations, four final drains and all 34 recorded server lifetimes exited. Performance remains unmeasured. [Source qualification](https://github.com/c4pt0r/kv9/blob/01d128fd771dfbf0e6826ee5b6821411afac1fec/docs/WRITE-RAFT-FRAME-BUFFER.md). |
| Slicing-by-eight engine CRC `e5662bb`, reapplied as `e748620` | Experimental on selected ThinLTO. Fresh 47-theorem proof, 710 workspace tests/doctests (23 existing ignored), release, 363-operation ordinary recovery and [actual 21-window Chaos](WRITE-CRC-CHAOS.md) pass. [Matched write comparison](WRITE-CRC-PERFORMANCE.md) passes: loaded BatchPut(64) +18.807% with lower pooled p99; loaded point writes +2.786%. Full read/mixed regression remains pending capacity before promotion. [Current qualification](write-reference-qualification-v1/README.md); [historical qualification](https://github.com/c4pt0r/kv9/blob/65511010e2fda8adba04efd831a39bcdca1979a4/docs/CRC32-SLICING-QUALIFICATION.md). |
| Independent per-request stream tasks `f2c4e85` | Retained; older c64 GET +38.15–38.93%, with better mean/p99. [Original report](https://github.com/c4pt0r/kv9/blob/cf5c87e/docs/PARALLEL-STREAM-GET-PERFORMANCE.md). |
| Linux jemalloc `629bee4` | Held: older c64 GET +4.47% / +6.24% and better mean/p99, with aggregate mean voter RSS +23.1% / +25.9%. Larger working sets and mixed/write qualification remain necessary. [Report](https://github.com/c4pt0r/kv9/blob/d79ea48/docs/JEMALLOC-SERVER-PERFORMANCE.md). |
| One RPC runtime worker `711631b` | Rejected for throughput/latency regression. [Report](https://github.com/c4pt0r/kv9/blob/99993db/docs/RPC-WORKER-SCREENING.md). |
| Two RPC runtime workers `5ee897a` | Retained in CRC ancestry, with its original validation scope. [Report](https://github.com/c4pt0r/kv9/blob/f155219/docs/RPC-WORKER-PAIR-PERFORMANCE.md). |
| Fixed global queue interval eight `c893834` | Already rejected on two RPC workers/event8: c64 GET -3.217% / -2.614%, higher means. [Report](https://github.com/c4pt0r/kv9/blob/c3e4f81/docs/RPC-PAIR-GLOBAL-QUEUE-SCREENING.md). |
| Two persistent stream handler workers `9b74274` | Already rejected: c64 GET -4.503% / -4.808%, higher means despite improved p99. [Report](https://github.com/c4pt0r/kv9/blob/b5f140e/docs/STREAM-WORKER-PAIR-SCREENING.md). |
| Direct peer request body `6707bcc` | Rejected at both c1/c64; all eight throughput comparisons regress. [Report](https://github.com/c4pt0r/kv9/blob/1052d73/docs/DIRECT-PEER-BODY-SCREENING.md). |
| Idle peer watchdog `f62c08e` | Rejected; improved tails but seven of eight throughput comparisons regress. [Report](https://github.com/c4pt0r/kv9/blob/206c1ba/docs/PEER-IDLE-WATCHDOG-SCREENING.md). |
| Immediate peer batch enqueue `ea5f498` | Rejected matched latency screen. [Report](https://github.com/c4pt0r/kv9/blob/d76c3df/docs/PEER-BATCH-ENQUEUE-SCREENING.md). |
| Two concurrent ReadIndex contexts `5654ea5` | Held after mixed-read regression; preserve existing confirmation/fence semantics. [Report](https://github.com/c4pt0r/kv9/blob/119a49c/docs/READ-WINDOW-SCREEN.md). |
| CRC plus immutable stream metadata `bb13e43` | Held: pure c64 GET essentially unchanged, mixed throughput -0.453%. [Report](https://github.com/c4pt0r/kv9/blob/3641d0f/docs/STREAM-METADATA-CRC-PERFORMANCE.md). |
| 64-KiB Append payload target `74b958a` | Held; intended mixed-load benefit not established. [Report](https://github.com/c4pt0r/kv9/blob/381d097/docs/RAFT-APPEND-PAYLOAD-PERFORMANCE.md). |
| Inbox vector reuse `1045755` | Held; small mixed gain with c1/pure-read tradeoffs. [Report](https://github.com/c4pt0r/kv9/blob/51efc65/docs/INBOX-VECTOR-REUSE-PERFORMANCE.md). |
| RPC event interval one `5bed688` | Rejected. [Report](https://github.com/c4pt0r/kv9/blob/a7d6ccf/docs/RPC-EVENT-ONE-PERFORMANCE.md). |
| Separate outbound peer executor `36ae89a` | Rejected as the next increment; retain complete latency/throughput tradeoffs. [Report](https://github.com/c4pt0r/kv9/blob/3ed8661/docs/PEER-EXECUTOR-ISOLATION-PERFORMANCE.md). |
| Coalesced owner notifications `42e0117` | Experimental modest loaded-read/mixed improvement; isolated GET does not improve, broader API/Chaos gates remain. [Report](COALESCED-OWNER-PERFORMANCE.md). |
| Fixed global queue interval eight on CRC `3338650` | Revisit of `c893834` under newer c1/c64 GET/mixed scope; rejected again, c64 GET -2.390%. [Complete report](GLOBAL-QUEUE-PERFORMANCE.md). |
| ThinLTO and one release codegen unit `02d0c01` | Complete [full72 comparison](RELEASE-THIN-LTO-FULL72.md): all 12 point/batch cells improve throughput/mean in both orders; c1/c64 GET +6.806%/+8.465%. Loaded BatchPut pooled p99 worsens, explicitly retained. Exact-build [21-window Chaos](RELEASE-THIN-LTO-CHAOS.md) accepted; [fresh main integration `11113f6`](RELEASE-THIN-LTO-MAIN-INTEGRATION.md) now passes. [Earlier screen](RELEASE-THIN-LTO-PERFORMANCE.md) remains separately scoped. |
| Fixed-rate BatchPut tail diagnosis on CRC / selected `11113f6` | [Eight accepted cohorts](BATCH-WRITE-FIXED-RATE-RESULTS.md): ThinLTO whole-call and scheduled p99 remain higher at 8k/12k offered calls/s in both orders. All 797,956 issued calls succeed; 2,044 dropped slots prevent strict equal-work attribution. No runtime change or performance rerun. |
| Bounded 32-us owner polling `2ca5fcc` | Rejected by the [complete 24-cohort screen](OWNER-POLL-PERFORMANCE.md): all eight matched workload/order comparisons regress QPS, mean and p99; all pooled workloads use more server CPU. C1/c64 GET QPS -18.963% / -25.251%; c1 CPU 1.6084 → 3.3527 estimated cores. Source/proof and ordinary recovery remain separately scoped. No retune, cohort rerun, promotion or new Chaos run. |

## Latest unmeasured prototypes

`d94cdee` returned handler polling to the stream owner. It passed 436 local
Raft/server tests/doctests (one existing ignored), formatting/Clippy, a retained
default release and ordinary streaming/unary recovery: 348 operations, 312 OK
and 36 unknown, all retained in complete histories. Its performance setup was
frozen but no smoke/timing campaign was executed. The earlier independent-task
gain makes lost synchronous parallelism a material concern.

A subsequent two-persistent-worker draft passed 438 local tests/doctests, with
one existing ignored, formatting/Clippy. Review located the earlier `9b74274`
experiment. This draft duplicates that rejected architecture and is stopped
before release/timing. Its source patch and unapplied queue-specific test
proposals remain local. Neither prototype supplies new QPS or latency data;
neither changes selected runtime or qualifies any original checklist item.

## Current direction and reuse rule

The [quorum-path capture](QUORUM-TRACE-RESULTS.md) is now complete. Recorder
`1875e74` lost 1,083 observations; bounded immutable-slot repair `7d45612`
records all selected events in six prefixes and passes the unchanged reader.
The observed leader-local round-trip means are 17.286 / 17.102 us, with local
follower inbox admission-to-drain means 1.752–1.817 us. These are sampled
lifecycle populations, not pure network or parked-thread durations.

The isolated 32-us owner-poll candidate `2ca5fcc` is now rejected by its
[complete screen](OWNER-POLL-PERFORMANCE.md). All eight workload/order comparisons
lose throughput and worsen mean/p99, with more server CPU in every pooled cell.
The original predicate, notifications, tick deadline and all Raft fences remain
unchanged; successful source/proof/recovery gates do not predict performance.
No poll-budget sweep, coalescing combination or losing-cohort replacement follows.

Next reuse the existing traces and all-branch profiles before a bounded CPU/
call-stack observation on the exact selected build, outside timing. Identify a
concrete cost before another scheduling/transport change. The higher CPU use is
consistent with shared-core contention but does not itself locate a hot stack.

The ThinLTO candidate `02d0c01` now passes full workspace checks (709 tests,
23 existing ignored, formatting/Clippy), a source-bound default production build,
359-operation ordinary recovery, the earlier 24-cohort screen, the new complete
72-cohort point/batch comparison and the 21-window actual Chaos matrix with
9,923 complete history operations. Full72 accepts 81,648,272 measured successes,
240 exited lifetimes and 144 drains/bindings. Throughput and mean improve in all
12 cells, with a loaded BatchPut p99 tradeoff. Keep the exact source and artifacts
frozen. Main integration `11113f6` now passes fresh release and observer checks,
a separate default build and 291-operation recovery (262 OK / 29 unknown).
The new server/client reproduce the original executable hashes. The completed
[fixed-rate write diagnosis](BATCH-WRITE-FIXED-RATE-RESULTS.md) preserves higher
tails at both common offered rates and client-drop/scheduling limitations.
Keep the original closed-loop result; the active next phase is the
[three-copy write comparison and optimization](WRITE-PERFORMANCE-NEXT.md). A combination with notification candidate
`42e0117` requires its own matched qualification; historical percentages cannot
be added.

Use the [quorum-path plan](QUORUM-LATENCY-NEXT.md) and its completed captures
before adding another observation. Existing body-handoff measurements remain
reusable; new evidence must resolve a remaining cost before another rewrite.

Before revisiting any row, state the new evidence and changed variable that
could invalidate its old conclusion. A fresh branch, renamed helper or repeated
latency symptom is insufficient on its own. Check all branch history and the
linked original report before implementation; preserve original failures and
separate new evidence from old acceptance. Scheduling rewrites are paused unless
a new concrete cause warrants one. Core proof, actual Chaos, no service-critical
SPOF except object storage, Redis read parity and subsequent dynamic multi-Raft/
automatic splits remain open. CI stays local.
