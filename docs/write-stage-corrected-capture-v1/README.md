# Corrected write-stage capture: original evidence

The new matching `3f4d4cf` releases complete all four two-second BatchPut(64),
c64 cohorts. Both instrumented cohorts have zero recording loss. The full
runtime and independent audit pass 131,737 measured calls / 8,431,168 items,
12 fresh drains, 16 exited owned lifetimes, 48 raw snapshots and exact original
container CPU restoration. See the [results and limitations](../WRITE-STAGE-CAPTURE-RESULTS.md).

The combined tracing/diagnostics overhead is -3.666% / -1.852% throughput by
order, -2.770% pooled. Per-order client p99 bounds remain separate. This is an
accepted short observer comparison, not a selected-runtime speedup, full
performance-selection gate, Redis comparison, new Chaos acceptance or complete
linearizability history. CRC remains selected.

## Actual executions

| Step | Actual terminal |
| --- | --- |
| New matching default/instrumented release pair | `13353 / 4bd6ff / 0` |
| Independent source/ELF/Cargo/codegen readback | `070a11/0` |
| Six new path/source/feature/client/terminal-binding checks | `ca973a/0` |
| Complete four-row capture and restoration | `78595 / ca7099 / 0` |
| Independent runtime metadata audit | `66bc82/0` |
| Retained-stage and observer-overhead analysis | `1ee97e/0` |
| Independent 12-input analysis review | `9ce9ff/0` |
| Supplemental existing WAL metric differences | `eb02fd/0` |
| Archive assembly and every-member readback | `75701 / a0619a / 0` |

The previous eight capture controls are inherited unchanged, not replayed.
The path-only wrapper adaptation preserves original helpers and diffs. Current
source has no additional runtime correction beyond the previously tested pump
gate fix; Rust tests and unchanged theorem/model suites were not rerun here.

The audit passes on its first runtime execution. A later metadata-only attempt
to create its receipt inside the root-owned audit result directory was denied
before creation (`651217/1`). The receipt was instead written in the existing
owned parent. That event did not rerun or change the successful audit; its
terminal notes remain retained.

The independent stage review verifies order/denominator mapping, summed-work
over elapsed-time rates, latency sums over calls, local group deduplication and
sampled-position weighting. The separate WAL supplement is root-calculated;
its input hashes and arithmetic method are included. Unlike populations and
snapshot boundaries are not subtracted into an invented residual stage.

## Coverage

Each instrumented leader retains 512 unique compatible pairs. Followers have
512 sampled group rows and no terminal waiter inspections. Every instrumented
ring and counter is unchanged through post-client, drain and final readback.
The earlier 1,534 / 1,524 overwritten samples per populated ring remain missing.
Deterministic modulo-16 tails and represented-group deduplication are not an
unbiased all-group distribution. Internal stage quantiles cannot identify or
be added into a native-client p99 request.

Original retention receipts cover 8,418,027,444 database payload bytes copied
and fully read back before tmpfs scratch removal. The later independent audit
checks receipts and file metadata, not another payload decode/hash. These bytes
remain under `/mnt/data/kv9-work`. Retained tmpfs data does not establish disk
or power-loss durability.

## Portable metadata

[runs.tar.gz](runs.tar.gz) contains **533 regular members / 53,476,119 logical
bytes**, including both release metadata sets, original source inventories,
preparation and identity checks, all four runtime metadata sets and raw boundary
files, independent audit/review, analysis and the packet assembler. Its size is
7,950,031 bytes, SHA-256
`b935fc96de707a9bfe710af31066afbb0e0fbc64b945ceec223cbb6bcca84315`.
Every member was decoded, hashed and compared with its original bytes:
[readback](readback.json), [inventory](inventory.json).

Large database payloads and both server executables are explicitly excluded.
Compiler caches and complete offline build dependencies are not supplied.
Absolute original paths and terminal records retain their original identities;
historical `tool-live.json` entries do not mean those completed sessions still run.
No old evidence was moved or deleted, and no hosted workflow was dispatched.

The next investigation targets CPU work within batch lowering, WAL
encoding/checksum and resident-index publication, using this accepted evidence
and preserving previously rejected experiments. The first lossy attempt stays
separate in [its original packet](../write-stage-capture-v1/README.md).
