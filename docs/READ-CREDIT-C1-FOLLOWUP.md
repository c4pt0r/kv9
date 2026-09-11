# Read-credit follow-up: longer c1 measurements and completed local fault gate

Candidate `57ff6851e40ed63c837189d6eb0a11190704725a` completes the separately
predeclared four-repeat c1 comparison. All **53,783,436 issued logical calls
succeed in exactly 53,783,436 attempts**. All 24 cohorts pass the first unchanged
independent audit. Pooled GET throughput increases **0.424%** and its mean
decreases **0.443%** against control `5ee897a`; GET p99 improves in two repeats
and worsens in two. These observations do not establish statistical significance
or a c1 no-regression result.

The earlier [five-second c1/c64 screen](READ-GROUP-CREDIT-SCREENING.md) remains
separate, including both small c1 GET regressions and its roughly 8% c64 gains.
The longer experiment does not replace those observations. Exact-source
[eleven-window local Chaos acceptance](READ-CREDIT-CHAOS-ACCEPTANCE.md) also
passes. Keep the candidate for further performance work; **`5ee897a` remains
the general baseline** pending broader load and semantic gates.

## Four paired repetitions

All latency values are whole logical calls in microseconds. Report each
original p99 histogram interval without averaging percentiles.

| Repeat | API | Control calls/s | Candidate calls/s | Change | Mean us, control -> candidate | p99 interval us, control -> candidate |
| ---: | --- | ---: | ---: | ---: | --- | --- |
| 0 | GET | 26,462.704 | 26,487.456 | +0.094% | 37.668 -> 37.623 | 51.712-52.223 -> 52.224-52.735 |
| 1 | GET | 26,382.713 | 26,387.281 | +0.017% | 37.781 -> 37.773 | 52.224-52.735 -> 51.712-52.223 |
| 2 | GET | 26,204.691 | 26,496.583 | +1.114% | 38.035 -> 37.606 | 52.736-53.247 -> 51.712-52.223 |
| 3 | GET | 26,292.640 | 26,417.996 | +0.477% | 37.909 -> 37.721 | 52.224-52.735 -> 53.248-53.759 |
| 0 | BatchGet(1) | 26,282.217 | 26,285.631 | +0.013% | 37.919 -> 37.916 | 52.224-52.735 -> 52.224-52.735 |
| 1 | BatchGet(1) | 26,156.824 | 26,575.532 | +1.601% | 38.103 -> 37.488 | 53.248-53.759 -> 50.176-50.687 |
| 2 | BatchGet(1) | 26,059.263 | 26,311.673 | +0.969% | 38.251 -> 37.879 | 52.736-53.247 -> 52.224-52.735 |
| 3 | BatchGet(1) | 26,118.957 | 26,166.714 | +0.183% | 38.141 -> 38.072 | 53.248-53.759 -> 52.736-53.247 |

Pooled QPS is total calls divided by total observed cohort seconds. Pooled
mean is total whole-call latency divided by total call count. Each arm has
four separate 30-second windows; this is not a continuous 120-second run.

| Arm | Total calls | Pooled calls/s | Pooled mean us |
| --- | ---: | ---: | ---: |
| Control GET | 3,160,285 | 26,335.687 | 37.848132 |
| Candidate GET | 3,173,681 | 26,447.329 | 37.680573 |
| Control BatchGet(1) | 3,138,519 | 26,154.315 | 38.103270 |
| Candidate BatchGet(1) | 3,160,189 | 26,334.887 | 37.837581 |
| Redis GET | 20,701,084 | 172,509.024 | 5.721245 |
| Redis MGET(1) | 20,449,678 | 170,413.961 | 5.788401 |

Candidate GET still has approximately **6.6x Redis's mean latency**. Every
Redis control and its p99 interval is retained in the
[complete readout](../scripts/redis-reference/read-credit-followup-v1/c1-long/READOUT.md).
Larger read groups help amortize confirmation at c64; c1 has no overlapping
readers to share that work. Shortening the individual confirmation path is
therefore still necessary. No causal attribution or intrinsic latency floor
is inferred from these endpoint results.

## Protocol, validation and isolation

Protocol `kv9-read-credit-c1-long-v1` fixes six arms and four repetitions in
forward/reverse/reverse/forward order. Only the duration and c1 schedule change
from the prior study. Sources, binaries, payload, public/SDK admission,
deadlines, connection behavior and isolation settings remain fixed. Native
measurement source is `03c1c776`, Redis reference source `b8ec38f`.

Six driver and twelve independent auditor contracts pass before timing. The
24 descriptors match the driver, independently defined auditor, test fixture
and frozen protocol. Source preflight verifies 584 control, 594 candidate,
579 native-client and 580 Redis-client files. The prior correctness-only smoke
used these exact releases; it is excluded from performance. No runtime smoke
is repeated for this duration/schedule-only follow-up.

Root timing session **52530 exits 0**, and independent audit session **3452
exits 0**. No build, test, fault, profile or independent audit overlaps timing.
The preceding Chaos fixture and all its helper commands are terminal first.
All refusals, unknown writes, read failures, client rejections, non-success
reasons, connection failures and dropped slots are zero.

The audit checks 80 exited owned lifetimes, 48 qualifying drains and writer
bindings, 14,096 resource samples, 2,337 role/source checks and 528 retained
files / 88,754,298 bytes. All three owned container cpusets restore exactly to
configured/effective `0-31`, and historical namespace identities are preserved.

This is a shared-host, one-client-worker, three-voter volatile tmpfs-WAL
diagnostic against standalone Redis 7.0.15 with persistence disabled, one I/O
thread and no pipelining. It does not establish equal durability, cross-host
latency, sustained service capacity or performance under writes and mixed load.

Exact preparation, contracts, readout and accepted audit are retained in the
[evidence directory](../scripts/redis-reference/read-credit-followup-v1/README.md).
The original raw recording remains
`/tmp/kv9-read-credit-c1-long-diagnostic-attempt1`. Audit SHA-256:
`420b680b147f3a9a2bc69a7840d5550e20c45d21a848b02b04ce43a35a87f660`.

## Next implementation

Candidate [c3131800](https://github.com/c4pt0r/kv9/blob/c3131800b665f6f160a11c804f542237e77ab83a/docs/RAFT-INBOX-DRAIN.md)
builds on `57ff6851` and directly transfers the owned vector returned by the
Raft inbox, removing a second production vector allocation and message move.
The testing partition filter preserves its existing predicate and order. The
local sequence-equivalence argument leaves queue bounds, protocol messages,
read confirmation, apply fencing and write acknowledgment unchanged.

Its 218-test Raft gate, format check and warnings-denied Clippy pass; the clean
default release builds successfully. Process E2E and its unchanged independent
audit pass **364 calls: 335 OK, 29 unknown, zero refused**, including streaming
and unary leader loss/original-directory restart. All four progress windows,
six fresh drains and seven exited lifetimes pass. All 595 source files and
default release/runtime bindings remain unchanged. This is one-host process
evidence; it does not transfer `57ff6851`'s Chaos acceptance to this revision.
The separate matched performance screen is next. All current performance
numbers belong to `57ff6851`; no gain is claimed for `c3131800` yet.

Continue with measured handoff/buffer costs and sustained/write/mixed/large-batch
checks. Whole grouped-read/Ready/Rust proof composition, the broader industrial
fault matrix, Redis-class read/write performance and automatic splits remain
open. All work uses local checks; hosted CI is not triggered.
