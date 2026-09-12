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

| Change / source | Recorded decision and evidence |
| --- | --- |
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

The ThinLTO candidate `02d0c01` now passes full workspace checks (709 tests,
23 existing ignored, formatting/Clippy), a source-bound default production build,
359-operation ordinary recovery, the earlier 24-cohort screen, the new complete
72-cohort point/batch comparison and the 21-window actual Chaos matrix with
9,923 complete history operations. Full72 accepts 81,648,272 measured successes,
240 exited lifetimes and 144 drains/bindings. Throughput and mean improve in all
12 cells, with a loaded BatchPut p99 tradeoff. Keep the exact source and artifacts
frozen. Main integration `11113f6` now passes fresh release and observer checks,
a separate default build and 291-operation recovery (262 OK / 29 unknown).
The new server/client reproduce the original executable hashes. Investigate
the write tail at a [common offered load](BATCH-WRITE-TAIL-NEXT.md) without
discarding the closed-loop result. A combination with notification candidate
`42e0117` requires its own matched qualification; historical percentages cannot
be added.

After this qualification, use the [quorum-path plan](QUORUM-LATENCY-NEXT.md)
to localize isolated-read latency. Reuse the existing body-handoff measurement;
new observation must resolve a remaining boundary before another rewrite.

Before revisiting any row, state the new evidence and changed variable that
could invalidate its old conclusion. A fresh branch, renamed helper or repeated
latency symptom is insufficient on its own. Check all branch history and the
linked original report before implementation; preserve original failures and
separate new evidence from old acceptance. Scheduling rewrites are paused unless
a new concrete cause warrants one. Core proof, actual Chaos, no service-critical
SPOF except object storage, Redis read parity and subsequent dynamic multi-Raft/
automatic splits remain open. CI stays local.
