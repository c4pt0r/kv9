# ThinLTO exact-build Chaos Mesh acceptance

The unchanged candidate `02d0c01024b65a84b220c6948ff2224bfa7900bc` passes the
21-window voter/storage/endpoint matrix with actual Chaos Mesh injection and
independently checked complete CLI, point and native atomic batch histories.
This completes a correctness gate for the favorable
[performance screen](RELEASE-THIN-LTO-PERFORMANCE.md). Broader point/batch
performance qualification remains pending; CRC `ca0002c7` remains selected.

## Tested source and execution

The server is the original production ThinLTO binary, SHA-256
`dd028cb2f61633dda133b05d814a6d173a79a83145d8ced2f81dc0cf8e0f33bc`.
All 627 source files match the retained clean release. Its default feature
graph, compiler commands and source/build identity are independently verified;
the [source validation report](RELEASE-THIN-LTO-VALIDATION.md) retains those
details. Ordinary-port streaming point and native batch clients use their
original retained builds. A remote admission-pressure example has an explicitly
checked test-only feature graph; this does not change the server feature graph.

Both attempts use image
`sha256:91b5db374e49e817d4a9bb05c340490ab7303722e4e42f6f84f9a6919fa9f3b3`.
The image contains the existing server/client binaries, with no server rebuild
inside Docker. The second attempt changes only the delay observation selector
described below and its fresh run paths/identities.

Original corrected execution: **42651/0**, receipt `b399b7`; observer exit 0.
Independent terminal audit: **32590/0**, receipt `bcb717`, audit SHA-256
`6ac402a96e8234b7705d4befb390ca93f5e4b550eda837482aef116dcb4dc295`.
No database fixture was restarted to hide a failed operation or fault window.

## Complete histories and fault effects

| History | Operations | OK | Refused | Unknown |
| --- | ---: | ---: | ---: | ---: |
| CLI RawKV and catalog | 5,778 | 5,286 | 14 | 478 |
| Persistent streaming point | 1,489 | 1,468 | 8 | 13 |
| Native point and atomic batch | 2,656 | 2,617 | 13 | 26 |
| Total | **9,923** | **9,371** | **35** | **517** |

Each history has a matching return for every invocation. Unknown outcomes remain
in the checker's input. Native history includes 932 BatchGet and 929 BatchPut
calls, plus 795 point calls; item counts are not relabeled as RPC throughput.
The native checker covers the normal baseline and all 21 fault windows.

The windows cover registration seed blackholing, each of three voter Pod
failures, partition, public admission pressure, delay, six WRITE errors
(EIO/ENOSPC on every voter), three missing-log refusals, three independently
prepared replacement-PVC refusals, and pending/recovered endpoint migration.
The original formation recovery and container restart checks also complete.
Positive injected effects, successful work during the specified windows,
writer/process identity, fresh progress, inline completion, apply/prefix fences
and four final replicas with two fresh drained statuses pass independent checks.
The observer retains 488 batches and 1,614 accepted runtime samples.

These are bounded single-host Kind histories. This run does not add the
separate dedicated client-link/quorum or FSYNC-specific matrices, independent
host loss, device power loss, sustained performance or whole-implementation
proofs. The compiler remains a correctness premise; no Raft algorithm changes
or new algorithm proof claims are introduced by this code-generation setting.

## Preserved first failure and selector repair

The first execution **59532/1**, receipt `d4adfd`, stopped at the delay gate's
unchanged 20-second deadline. It did not execute the later disk/PVC/migration
stages. The gate froze the first nonempty fresh subset: nodes 3 and 4, both with
zero inline-read completions. Node 2 was the leader; its counter advanced from
10,361 to 10,452, but it had been excluded from the frozen comparison map.
The retained history contains 32 complete successful delay GETs after the
recorded progress threshold. That evidence diagnoses a collector-selection bug;
it does not turn the failed execution into accepted runtime evidence.

`ServerFreshness.observe` now returns a nonempty map only when the current fresh
keys equal all originally seeded identities. Each must match its identity and
have two serial metric-export advances. Missing/replaced/nonadvancing processes
still cause conservative failure. The timeout, active-fault requirement, two
client progress advances, complete successful GET and inline-growth predicates
are unchanged. Seven affected original controls and eight new regressions pass
once (`657cac/0`); unchanged controls retain their original results.

Both failed and successful attempts are retained independently. The first
failure's immediate report copies include missing/empty-copy errors; later
final-client collection is explicitly supplemental. Its point/native final
clients exited successfully, but no full-matrix acceptance is claimed. Early
metadata/feature-role checks, a pre-freeze normalization failure and their
bounded corrections are retained as well.

## Archive and owned cleanup

The accepted attempt's pre-cleanup archive contains 4,467 members and
942,647,571 input bytes; its 92,324,837 compressed bytes have SHA-256
`916b3148468a31f84ce0dfda826256077f64c8be89d2f3605b62237078ae94a5`.
All members are read back and all source files rehashed before cleanup
(`71600/0`, receipt `6a2f39`). This archives collected files and build inputs,
not a complete PVC or device image.

Owned namespace `kv9-chaos-1789204188-2187533`, UID
`af3878c4-e3b0-44e9-8ee1-29d7050ff396`, is removed only after that archive.
Cleanup **82779/0**, receipt `e19242`, verifies eight current process trees,
five current containers, original fixture/observer exits and the unchanged
eight historical namespace identities. The subsequent lifetime readback
(`55e9eb/0`) confirms **33 observed server lifetimes / 25 containers** exited
or were removed. Its first invocation (`aac352/1`) was premature: root called
it before cleanup reached terminal, and it refused immediately because the
required cleanup summary did not yet exist. That failed invocation is preserved;
the unchanged helper ran after cleanup completed.

The failed attempt has its own independently read-back archive: 1,504 members,
209,044,901 input bytes, 39,656,354 compressed bytes, SHA-256
`c63d0a46f3933908792dc452be6d2c0ee34ed1cfed23fb84d3011bb3f7e1d06e`.
Its owned namespace/processes were separately cleaned, preserving the same
historical namespaces. An unrelated historical PodChaos resource remains
untouched; it is not described as recovered by either run.

## Next performance gate

Keep the original CRC/ThinLTO servers and fixed v3 clients for **36 smoke and
72 timed cohorts**, covering c1/c64, point/batch64, read/write/mixed, and both
orders. Report separate operation counts, throughput, mean and histogram-based
p99. The current 24-cohort performance result remains unchanged.

The new post-process compressed-retention preparation passes 53 tiny/helper
controls: seven driver, 15 auditor and 31 retention controls. It compresses only
after writer exit, independently decodes exact original bytes and verifies a
combined smoke/timing capacity cap. Its actual **1,073,741,841-byte** synthetic
compression/independent-decode/fresh-restore qualification now passes
(`70393/0`, `c603b7`). [Capacity and restore evidence](https://github.com/c4pt0r/kv9/blob/7d869009602919748a8cd8e920dade305e3072fb/docs/thin-lto-capacity-v1/README.md)
retains 38 completed historical CRC transactions and an independently rehashed
seven-file real WAL restore. The remaining 554 files / 8,482,763,491 logical
bytes are cold; net payload allocation saving is 2,442,665,984 bytes, excluding
metadata. Original full-input audits require rehydration first. The exact native
cache cleanup also preserves binaries and invalidates affected Cargo build-script
success markers before removing outputs. Further capacity remains necessary for
the full performance runtime. No workload or storage floor is reduced.

Full proof, fault, bounded-storage, host-availability and Redis-read-parity gates
remain open before dynamic multi-Raft and automatic splits. All checks are local;
no hosted workflow was dispatched. The tracker is
[#9](https://github.com/c4pt0r/kv9/issues/9).

## Reporting evidence

[Immutable reporting evidence](https://github.com/c4pt0r/kv9/blob/50933c8b9b8a9bca941e1ed14ac2ab8189bf758e/docs/release-thin-lto-chaos-v1/README.md)
retains 6,489 original files / 819,666,702 decoded bytes in 17 bounded archive
parts. Its integrity verifier passes (`9f0c31/0`). Explicitly omitted executable
payloads, bulky observations, symlinks and original archives remain local.
The reporting bundle is not a complete replay image. Original runtime evidence remains under
`/tmp/kv9-chaos-e2e.7SKaLy` and `/tmp/kv9-chaos-e2e.WD2lZP`; preparation, audit,
archive and cleanup receipts remain under their respective
`/tmp/kv9-release-thin-lto-full-chaos-*-first` directories.
