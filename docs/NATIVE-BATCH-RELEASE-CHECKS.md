# Native batch clean release checks

The standalone default-feature release server, native batch measurement client
and Redis MGET/MSET reference at
`3bd1751bddf9a85b07447b64a12adf28ed0df8a9` completed finite local correctness
checks. These runs validate release behavior and report accounting; they are
not paired throughput/latency measurements or additional linearizability proofs.

All three builds used empty feature lists, optimization level 3 and a clean
source tree. The server is byte-identical to the preceding native measurement
release. No runtime or
validator changes were needed for these checks.

| Check group | Cases | Logical calls | Result |
| --- | ---: | ---: | --- |
| Native batch client on three ordinary WAL voters | 9 | 19,527 | All intended predicates passed |
| Redis reference against fresh Redis and controlled RESP peers | 19 | 482,259 | All intended predicates passed |

Native cases cover batch sizes 1, 4, 16, 64, 128 and 256, an operation cap,
fewer scheduled slots than workers, and two-voter unavailability/recovery.
The unavailable-voter case retained 344 successful calls, 24 refusals,
517 unknown writes and 519 read failures. Final data verification and existing
aggregate report validation passed. These reports do not replace full
per-operation histories or independently prove uncertain-write non-replay.

The unchanged Redis cases cover the same batch sizes, read/write mixes,
fixed-rate scheduling and shedding, cap/idle-worker boundaries, fragmented
responses, malformed cardinality and lengths, server errors, and consumed
writes followed by EOF or timeout. The two deliberate malformed-read cases
correctly produced incomplete reports and exit code 1. Both consumed-write
controls observed one application, no replay of that nonce, then a distinct
new nonce on a new connection. Real-Redis final data and sentinels passed;
the two fatal protocol cases intentionally skipped client final verification.

The healthy Redis batch-1 case reached its original 100,000-call cap, and
correctly reports `timing_eligible=false`. It was retained without changing
the cap or rerunning. Redis fixed shedding conserved 1,242 issued plus
23,758 dropped slots, exactly 25,000 offered. Release duration eligibility
is a report predicate, not evidence that these correctness runs are suitable
performance measurements.

All 52 owned process lifetimes exited and cleanup passed. The native and
Redis drivers ran sequentially on CPUs 6–31. This placement does not isolate
physical cores from other host activity. Redis persistence and replication
settings do not match three-voter KV9 durability.

## Reproduction identity and retained evidence

The native driver changed only the build/output paths and expected revision.
A separate Redis adapter changed the old debug-only hash/profile check to the
exact clean release identity and the existing complete/duration/release/clean
eligibility predicate. All 19 case configurations, controlled peer behavior
and maintained validators stayed unchanged. The original debug driver and
its evidence remain intact.

| Artifact | SHA-256 |
| --- | --- |
| Release server | `82cf715e6d8d1ea8898db1ad3624431958a21213c50ebc00ae5320943cc03991` |
| Native measurement client | `0fcf10b9d644ce1b8323b5736510b0826f5bb89aae3d2c72f2071c5e0b3f5ad2` |
| Redis reference client | `d5c2069f00787a37a14b1cb25848aad05c3d2c684d9609b914becbafc0ec9e2b` |
| Combined release build manifest | `80a0987ba19e24812e6c554bde7d0df2193d2702823dfb0430449e9d2b87d645` |
| Correctness handoff JSON | `3ecc1165edc1c86ae1df51c7a1c7b1c2bd8d2a089d466af1414863d49679e791` |

Retained local roots:

- `/tmp/kv9-native-batch-paired-release-build-first`
- `/tmp/kv9-native-batch-paired-release-smoke-first`
- `/tmp/kv9-redis-batch-reference-release-correctness-attempt1`
- `/tmp/kv9-native-batch-paired-release-correctness-handoff`

Batch performance, remaining adapter-proof and Chaos obligations, and main
promotion remain open in issue #50. Daily CI remains local.
