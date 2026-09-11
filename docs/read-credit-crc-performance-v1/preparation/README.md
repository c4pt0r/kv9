# Read-credit on CRC: frozen full workload preparation

**Ready for root review; no runtime launched.** The candidate is clean `de37c71009e8199859b931e0037818f490e8d3f6`. The only candidate build used is `/tmp/kv9-read-credit-crc-release-rebuilt-first`: server SHA `5464adee210f7cebbce7cd57d6e01bfe5a1ace42936edad24f57e63ff9cfbdd7`, original manifest SHA `164b7999386b042ad2df814fea44fe93724ae15396e9f0cae0356a926fc3d1de`. Its 600-file source tree is `59924d6dfcc38dd3640866b424659cfc8319e886cf8d32745841faca4384e0bb`.

Root's accepted rebuild record is copied verbatim as `root-release-rebuilt-readback.json`; it records session 5651 / exit 0 / receipt 512c3d and the actual project rebuild. The rejected `/tmp/kv9-read-credit-crc-release-first` remains retained and is absent from all executable/configuration inputs. It is not relabeled as acceptance.

## Inherited protocol and scope

This preparation derives from `/tmp/kv9-owned-batch-broad-workloads-preparation`. Only candidate identity/build path, owned paths, protocol/report label, and derived driver hash bindings change. Adjacent `.diff` files show the complete parent delta; `.final.diff` shows the final pin/path substitution from the retained `pending-first/` draft. The wrapper and statistical arithmetic core are byte-identical.

The fixed old role is selected CRC `ca0002c7f8e9ee6f595efcc9f4151085ccce87cb` from `/tmp/kv9-wal-crc32-table` and original `/tmp/kv9-wal-crc32-release-first`. Native and Redis clients remain unchanged v3 `0be806d9671e2c50701a64aa7889c8859b7648ba`, from `/tmp/kv9-point-write-measurement-v3` and `/tmp/kv9-point-write-v3-release-first/{native,redis}`. The candidate correctness workload is source-bound in its `workload/` subdirectory; measurement uses the fixed 0be client.

Protocol: `kv9-read-credit-crc-broad-workloads-c1-c64-v1`. All descriptors exactly equal the accepted parent's workload inventory:

- 72 timed cohorts: c1/c64 × point1/batch64 × read percentages 0/50/100 × old/new/Redis × two repetitions. Repetition 1 reverses the entire first 36-cohort list. Each timed cohort is 10 seconds.
- Separate correctness smoke: the first 36 descriptors, with the inherited two-second duration and background placement.
- Closed loop, 4,096 keys plus sentinel, 128-byte values, seed 71, 128 warmups, 10-million-call cap, 1,500-ms deadline, configured max-attempts 6, native max-in-flight c1/c64.
- Actual API pairs GET/PUT ↔ GET/SET and BatchGet/BatchPut(64) ↔ MGET/MSET(64). Setup and verification use their original batch APIs.
- Public request capacity 64, matched-fixture encoded-byte capacity 16 MiB, async reads 128. The 16 MiB value is the established fixture override, not the 64 MiB production source default.

All outcomes/attempts, pure/mixed operation populations, deterministic mutable values/write-key membership, full paginated scan/sentinel, process/start/boot/executable/listener identity, drained publications, resource observation, WAL retention, and cleanup predicates remain unchanged. Aggregate reports do not establish exact issued write-nonce membership or concurrent linearizability history. Per-operation and whole-call raw histogram populations remain separate; quantiles are merged from raw counts, never averaged, and batch latency is never divided by keys.

Timed CPUs remain client 0–1, servers 2–5, and helper/owned-container 6–15,22–31. Root owns the exact container identity/restriction/restoration checks. No Docker/Kubernetes mutation was performed here. Three-voter KV9 quorum/sync on tmpfs and standalone memory Redis do not have equal write durability. This full matrix will assess GET throughput and c1/p99 while retaining every declared workload; preparation makes no result or promotion claim.

## Final verification

All **7 driver contracts and 15 auditor contracts pass**, including the previously deferred actual source/build-role check. Full role readback binds 2,352 source-file instances: CRC 590, candidate 600, and the same 581-source native/Redis client twice. Every final Python source and independent 72/36 inventory is checked. The four-role source/build bindings are in `roles-final-first.json`; original default-release Cargo profile/feature and client pairing checks are unchanged.

`READINESS.json` binds **41 preparation inputs** and **21 external runtime/helper inputs**, plus the full source maps. `inventory-final-first.json` and `external-inputs-first.json` contain exact paths, sizes and hashes. All owned preparation commands are terminal. No fixture, real-result audit, statistics derivation, Cargo, benchmark, profile or fault has run.

The initial pin metadata reader correctly bound the server but assumed the correctness workload lived at release root. It failed with `FileNotFoundError` before tests. The exact first writer and `pin-readback-first-failure.json` remain retained; the corrected metadata reader uses `workload/kv9-batch-workload`. No runtime helper, predicate, build or measurement was rerun to address this ancillary path mistake.

## Storage and launch prerequisites

The earlier space observation was about 202.20 GiB retention and 61.72 GiB tmpfs; `space-readiness-final-first.json` records the fresh post-rebuild sample. The previous same 72-cohort matrix retained exactly 94,449,175,837 bytes (87.97 GiB). This is historical consumption, not a future upper bound. A duration-only smoke extrapolation adds about 17.6 GiB, but initialization, throughput and other retained work make that uncertain.

The original thresholds remain: pre-cohort tmpfs ≥32 GiB / retention ≥96 GiB; observed runtime tmpfs ≥16 GiB / retention ≥64 GiB. Earlier endpoint guards remain too. A guard intervention retains incomplete/invalid timing, with no cap change, omitted cohort or automatic rerun. No old WAL, history or other evidence was removed. Root must recheck actual free space and owned-container identities before launch.

`commands.json` provides exact argv for:

1. Root smoke: `/tmp/kv9-read-credit-crc-performance-smoke-first`.
2. Root isolated timing: `/tmp/kv9-read-credit-crc-performance-timing-first`.
3. One frozen independent audit into this directory's fresh `results-first`, replacing only `ROOT_TERMINAL_TIMING_SESSION` with the actual released terminal session.
4. Frozen statistics after accepted audit and root's `/tmp/kv9-read-credit-crc-performance-root-preparation/audit-terminal-first.json`.

Root process recovery, readiness review, separate smoke and terminal-gated timing release remain required. The future session placeholder is never represented as an actual invocation.
