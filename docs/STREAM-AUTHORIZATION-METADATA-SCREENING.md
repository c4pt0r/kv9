# Stream authorization metadata: performance screening

Keep `94d8b9fe1b59c6b441267de7dc4352207dd7079c` for broader correctness validation. Single GET improves **4.425% / 3.641%** against the same-run f2 system-allocator control, with better whole-call mean and p99 latency in both repeats. This is a useful incremental screening result, not Redis parity or production promotion. The [exact-candidate eleven-window Chaos acceptance](STREAM-AUTHORIZATION-METADATA-ACCEPTANCE.md) now passes with complete histories and positive fault effects.

The candidate reaches **296,785–298,481 single GET calls/s**, with mean **214.266–215.492 us** and p99 buckets **376.832–385.023 us**. Same-run Redis GET reaches 499,825–507,619 calls/s. BatchGet(1) improves **1.104% / 2.892%** to 284,519–290,686 calls/s. These batch-one rates do not establish larger-batch performance.

This experiment starts from f2 and changes only immutable per-stream authorization metadata reuse; it retains fresh authentication for every frame. It does not combine the separate [jemalloc candidate](JEMALLOC-SERVER-PERFORMANCE.md), which remains the best retained single-GET result at 304,863–305,903 calls/s. Differences between these isolated candidates are not before/after regressions or measured combined gains. The [source contract](https://github.com/c4pt0r/kv9/blob/94d8b9fe1b59c6b441267de7dc4352207dd7079c/docs/STREAM-AUTHORIZATION-METADATA.md) states metadata contents and lifetime boundaries.

## All twelve original cohorts

Latency covers the whole client call. Quantiles are histogram bucket intervals. RSS is the sum of sampled per-voter means for KV9, or the standalone Redis process; it is not an interval peak or a memory-pressure test.

| Repeat | Arm | Successful calls/s | Mean us | p50 us | p95 us | p99 us | RSS MiB |
|---:|---|---:|---:|---|---|---|---:|
| 1 | old-point | 285,832.4 | 223.756 | 210.944–212.991 | 348.160–352.255 | 401.408–405.503 | 42.792 |
| 1 | old-batch1 | 281,411.0 | 227.238 | 215.040–217.087 | 352.256–356.351 | 401.408–405.503 | 43.494 |
| 1 | new-point | 298,481.3 | 214.266 | 204.800–206.847 | 323.584–327.679 | 376.832–380.927 | 43.570 |
| 1 | new-batch1 | 284,519.1 | 224.758 | 212.992–215.039 | 348.160–352.255 | 397.312–401.407 | 43.352 |
| 1 | redis-mget1 | 499,970.1 | 127.862 | 118.784–119.807 | 176.128–178.175 | 231.424–233.471 | 14.714 |
| 1 | redis-get1 | 507,619.3 | 125.951 | 118.784–119.807 | 161.792–163.839 | 233.472–235.519 | 14.575 |
| 2 | redis-get1 | 499,825.4 | 127.903 | 118.784–119.807 | 167.936–169.983 | 231.424–233.471 | 14.540 |
| 2 | redis-mget1 | 492,613.5 | 129.780 | 119.808–120.831 | 172.032–174.079 | 231.424–233.471 | 14.708 |
| 2 | new-batch1 | 290,686.3 | 219.984 | 210.944–212.991 | 335.872–339.967 | 389.120–393.215 | 43.375 |
| 2 | new-point | 296,785.0 | 215.492 | 206.848–208.895 | 327.680–331.775 | 380.928–385.023 | 43.179 |
| 2 | old-batch1 | 282,515.8 | 226.347 | 215.040–217.087 | 352.256–356.351 | 405.504–409.599 | 43.232 |
| 2 | old-point | 286,359.1 | 223.338 | 210.944–212.991 | 344.064–348.159 | 397.312–401.407 | 42.776 |

Single-GET aggregate RSS increases by 0.779 / 0.402 MiB in the paired repeats; batch-one RSS changes by -0.141 / +0.143 MiB. This small dataset does not establish long-running reclamation or fragmentation behavior.

## Local screening evidence

Full post-screening workspace checks now pass: **711 default and 721 experimental tests**, with **23 ignored in each configuration**; these are overlapping configurations, not distinct totals. All-target workspace experimental Clippy passes with warnings denied. Logs: `/tmp/kv9-stream-auth-metadata-workspace-first`. Exact-candidate Chaos has now completed; its scope and evidence are linked above.

- Focused checks: 37 point-related tests, 31 gRPC tests and 10 optional RPC tests pass; filters overlap. All-target server experimental Clippy passes with warnings denied. The initial new role-change test expected Unauthenticated instead of the unchanged PermissionDenied; only the test expectation was corrected, and the failed attempt remains recorded.
- Actual default-feature stream/unary leader-kill and original-directory-restart E2E: **366 calls, 335 OK and 31 unknown**. Both complete atomic histories pass; unknown writes are not blindly replayed. All owned lifetimes and cleanup pass.
- Six-arm correctness smoke: **2,872,640 successful calls**. This is a guard, not a performance claim.
- Matched timing session **50073 exited 0**. The separately frozen auditor passed its first execution: **6,460,852 measured calls all succeed**, with zero refused, unknown-write, failed or client-rejected outcomes, no extra transport attempts and no safety-cap hit.
- All 40 owned lifetimes exited; 2,323 source-file comparisons, 355 resource samples, 24 fresh drains, 24 voter/writer/listener bindings and 264 retained files / 44,378,045 bytes pass. The wrapper restores all three owned container CPU configurations and preserves historical namespace identities.

The inherited protocol is unchanged: fixed native03 and Redis b8 clients, 64 closed-loop workers, 4,096 keys plus a sentinel, 128-byte values, 128 warmup calls, 1,500-ms measured windows, original 10-million-call cap and deadlines, forward/reverse arm order. Client CPUs 0–1 and all measured servers 2–5; owned background containers use 6–15,22–31 during timing. Builds, tests, fault work, profiling and audits are held outside timing. Other host services remain unconstrained.

KV9 retains three Raft voters, normal read barriers and WAL sync calls on volatile tmpfs. Redis is standalone with persistence disabled. This short shared-host diagnostic does not compare equal fault tolerance or power-loss durability, establish sustained capacity, or demonstrate write/mixed performance. No hosted CI was dispatched.

## Next acceptance and bottleneck work

The exact-source link/quorum Chaos increment now passes. Retain the candidate independently while completing the remaining broader acceptance obligations before promotion. In parallel, the [read-barrier waiting diagnostic](READ-BARRIER-WAIT-DIAGNOSTIC.md) identifies a more substantial elapsed-time path to investigate. Measure registration, sealed-group submission, quorum observation, apply catch-up and completion notification separately. Preserve normal quorum confirmation and the applied-index fence. No lease, stale read or relaxed write acknowledgement is proposed.

## Retained bindings

- Source/release: `/tmp/kv9-stream-auth-metadata`, `/tmp/kv9-stream-auth-metadata-release-first`.
- Server SHA-256: `00cddad3ef82de36a3950286224c89a641ae40350e68cade06dc7b4685a010c3`; build manifest `e7bfc773a8135ce73af83b7f6906f6d20f39fd3ccec051f893726c0f520eea60`.
- Driver SHA-256: `941962e20ed5702fb4ddf21513cc23304bc3758de86d6e338fba75df136b8214`; auditor `7a2fb187cb60e323e2d03e48424673742a5005093d983d774c3510fcb0439347`.
- Raw timing: `/tmp/kv9-stream-auth-matched-diagnostic-attempt1/cohorts`.
- Independent reader: `/tmp/kv9-stream-auth-comparison-preparation/results-first`; audit SHA-256 `02a0a28d78715b17f470f1971502e8cd8ff29706aafdee2301cc7cf377b1b225`.
- Process E2E summary SHA-256: `7c1259d185d195f23abcf63d2bd5f55d200dbce6f8b65152b060003378e3a13a`.
