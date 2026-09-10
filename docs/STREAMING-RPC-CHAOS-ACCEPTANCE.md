# Default-stream point Chaos acceptance

The exact normal-port streaming integration at `f0eaf23f6cd50b5a315e51d9cc12914a0f4e9d3b` passed the full local 21-window actual Chaos Mesh matrix on 2026-09-10. Its runtime is `e52e72b4d1020eb078a818f71854d8c179f3dd27`; the three intervening changed files are Markdown documentation. The standalone server and persistent client use default Cargo features, ordinary WAL persistence, and the usual advertised port 20160. The persistent workload explicitly records `tonic_stream` in version-2 configuration/report evidence; no experimental listener, RPC-experiment feature, or relay was used.

This acceptance covers point GET/PUT/DELETE and the unchanged accompanying CLI/catalog history. Native atomic `batch_get`/`batch_put` require their separate history model and fault matrix. This is local correctness evidence on one shared kind host, not latency/throughput, cross-host, power-loss, or formal proof-composition acceptance.

| Full history | Invocations / returns | Successful | Unknown | Refused |
| --- | ---: | ---: | ---: | ---: |
| Persistent default stream | 1,292 / 1,292 | 1,275 | 13 | 4 |
| Concurrent CLI/catalog | 5,090 / 5,090 | 4,595 | 487 | 8 |

Both complete histories independently validate; unknown results retain their uncertainty. Their SHA-256 values are `d412f3e2945f123997d09fe2cc1170d0598d6115ffbab88350ab3f4551f8e3e5` (persistent) and `5801fe7bfd368e6b3775ab4841772295bb89b093625ecf62ba26bb5262a863ac` (CLI). Every original window contains successful persistent and concurrent writes and reads: registration-seed blackhole; sustained failure of each of three voters; partition; admission overload; delay; six voter/errno combinations (EIO and ENOSPC); three missing-log restarts; three independent replacement-PVC refusals; and retained-PVC endpoint migration while pending and recovered. The original initial all-voter preformation crash/replacement, container recovery, and collector-removal independence checks also pass.

The original fault/effect predicates, whole-history checks, checker time budgets, and negative evidence controls were preserved. Independent copies pass formation, missing-store, replacement-store, endpoint migration, latency-metrics, and admission-pressure validators, including 26 corrupt-evidence rejections. The inherited boot/start identity, field bounds, delay qualification, and serial freshness controls pass all 43 preflight cases. The additional delay observation gate retains its 20-second bound and completed in 6.214623105 seconds, with two fully contained successful persistent GETs after fresh server and client progress observations and exact active-fault identity checks.

The independent observer reparsed 1,232 exact executable/status observations across 31 runtime lifetimes. It retained 154 rejected/transient samples rather than treating them as positive evidence. Fifteen fault/lifetime envelopes show positive completed-inline growth, including the required partition, delay, and recovered endpoint-migration phases; this is sampled envelope activity and does not attribute an individual response to a stack path. The final four healed replicas each have eight exact-writer drained observations after the final full-history verdict, including seven serial export advances. Public admission, async-read, and async-apply ledgers are drained, and no final fatal/stopped state is present.

The standalone production server was built before the pressure example, then rehashed unchanged afterward; the standalone workload was built separately with its own retained Cargo JSON. Server, engine, Raft, server-library, and workload production artifacts have empty feature lists and `profile.test=false`. The same retained workload binary and manifest are copied to the fixture and image. A live measurement-stage workload capture binds Pod/container identity, boot/PID/start ticks, executing binary, exact configuration, and established sockets to ordinary port 20160. Docker config ID and Kubernetes manifest digest are joined through the retained CRI image metadata.

| Identity | SHA-256 / value |
| --- | --- |
| Server executable | `b848dfdddbd1a848f897dd77883e82c072bcbb2017287fab2c6dd3cdde2e4212` |
| Persistent client executable | `a20f688d6ee3660c1e3ae3058e0b6c76df50afd664faf029ceb4c762a45135ec` |
| Image config | `sha256:5f9e033556047f12aec0266a5293e0d4aaf5b8c11eed9f85a345a51930962fbd` |
| Imported image manifest | `sha256:8e9e414eb8d4d21618c269032a1eee37dd1f409dbe82817ca7efda106f279d71` |
| Source inventory | 549 files; tree `4c8fb068f76870c3c10103b4897f55ca1ff6b30b2e71553e37e33e10a71a61d9` |
| Frozen ready plan | `a6bc2724a04438abe123415bdfc100a4a8d3642cb4a22ef4ebc15f9ae87aa2f4` |
| Independent acceptance result | `f3eb0f797050aa06a259f436ac38c84eafccb8078b4ed9d9536cae735ff78537` |

The owned namespace `kv9-chaos-1789066736-1191747` was removed after success. All eight preexisting namespace UIDs remain unchanged, including historical failed fixtures. The owned fixture, observer, and supplemental collector host processes exited. Host helpers/builds were pinned to CPUs 6–31; observed Pods retained CPUs 0–31. No exclusive-host timing claim is made.

All unsuccessful audit attempts remain indexed. The first client capture confused an image config digest with a manifest digest and timed out; its replacement validates the exact CRI mapping. A supplemental final-drain collector started after fixture cleanup and obtained no accepted samples; it was stopped without changing the fixture, and the continuous observer independently supplies the required post-history fresh-drain evidence. The first independent finalizer assumed one uniform preflight result schema, and a second assumed a historical-control row had an expected-verdict field; both failures remain intact. The finalizer checks the actual named schemas and uses the already successful independent validators without repeating them. There was one actual matrix attempt, which passed; none of these audit adapter failures is relabeled as a runtime pass.

Raw evidence is `/tmp/kv9-chaos-e2e.0997sU`; the copied independent evidence is `/tmp/kv9-normal-stream-chaos-independent-first`. The authoritative result is `/tmp/kv9-normal-stream-chaos-audit-first/audit-v3.json`. Preparation and every attempted script/log are under `/tmp/kv9-normal-stream-chaos-preparation` and `/tmp/kv9-normal-stream-chaos-audit-first`. The source snapshot and accepted prior observer sources are retained with this report. Archive name: `/tmp/kv9-normal-stream-chaos-f0eaf23-20260910.tar.gz`; its separate verified archive-result JSON records the final byte count, member inventory, and SHA-256 without making the archive self-referential.

The completed archive is 222,268,529 bytes with SHA-256
`090d994b66846141e76bf7854a5dc463675d5057f7414e15dc7a132bc13ce24f`.
All 3,486 members were read back and verified; all original inputs remained
unchanged. The result is `/tmp/kv9-normal-stream-chaos-archive-result.json`.
