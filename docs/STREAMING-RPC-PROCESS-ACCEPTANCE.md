# Default streaming process acceptance

The clean integration candidate `e52e72b4d1020eb078a818f71854d8c179f3dd27`
passed the first local ordinary-WAL process test using its standalone default
server and workload binaries. Both omitted-input/default streaming and explicit
unary use each voter's sole ordinary advertised listener. The workload records
its effective transport in report version 2.

| Transport | Complete logical operations | Successful | Unknown |
| --- | ---: | ---: | ---: |
| Default streaming | 145 | 133 | 12 |
| Explicit unary | 149 | 137 | 12 |
| Total | 294 | 270 | 24 |

Both full histories passed the independent linearizability checker. Each arm
kills its current leader, confirms successful reads/writes against the survivor
leader, restarts the original data directory and confirms further successful
reads/writes. Streaming had 12 successful GETs and 5 PUTs contained in its
voter-loss window; unary had 13 GETs and 5 PUTs. Unknown outcomes remain in the
histories and are not relabeled as refusals or discarded. These are correctness
runs, not throughput or latency samples.

The run owned five voter incarnations and two workload processes, all exited.
Each final voter capture retains two serial successful status-export advances
after the workload exit, stable process/boot/start identity, zero public/read/apply
ledgers, and non-stopped read/apply owners. Ordinary listener identity and
executable hashes are bound to the retained process lifetimes.

## Retained build and run

- Build: `/tmp/kv9-normal-stream-build-first` (debug, default features).
- Server SHA-256: `c31f8c57085ab24b902ed52b31bf36bc600fa53eb04470c25a9dddf7e0e0c9a3`.
- Workload SHA-256: `dcf6f88aa8aa943d8be71af329094bc8015f26a8844ed2b34032972bc4b4bef7`.
- Source inventory SHA-256: `5314c023764ad3cc8ff7e9d5d5dc27c6df7f83d94896e676d406634a87332cbd`.
- Run: `/tmp/kv9-normal-stream-e2e-first`.
- Run summary SHA-256: `10f31c8cfd9c0adfb81647acae61788f956770351301175ba8cfb8f6c6687add`.
- Fifteen v2/v1 transport and artifact corruption controls: `/tmp/kv9-normal-stream-v2-controls-first/summary.json`.
- Controls SHA-256: `5f41d03ebf4da608006b47d665dd50a985d1816dd5eb912d27d8a7635bccc646`.

The controls reject missing/unknown/malformed transport, malformed version,
missing/duplicate/non-executable artifacts, malformed features, tarpc without
both command and artifact feature evidence, relabeled v1 results and weakened
legacy feature evidence. Original genuine v1/v2 reports and full histories were
revalidated and their bytes remained unchanged.

Build, process test and corruption controls each completed on their first
attempt. Earlier implementation compile/import failures and the incompatible
historical control preparation remain recorded in
[the local checkpoint](STREAMING-RPC-INTEGRATION.md).

## Independent readback

A separate read-only audit passed on its first attempt. It checked all 548
source hashes against clean `e52e72b`, both normal-build Cargo feature sets,
six runtime helper bindings, both complete histories, all four loss/restart
windows, ordinary listener ownership, and both fresh three-voter drains.
All seven owned child lifetimes were absent at readback. The audit read
82 artifacts totaling 500,834,758 bytes without modifying retained evidence.

Audit: `/tmp/kv9-normal-stream-independent-first/audit.json`.
SHA-256: `9b9e2c0e4a5f056c842ac9c02b472d21317562ac21764c06bcda80cb0bf17b0e`.

## Scope and next gates

This establishes point GET/PUT/DELETE histories for the normal-port integration
on one local three-process WAL cluster. It does not establish native-batch fault
histories, actual Chaos Mesh acceptance, performance improvement, cross-host or
power-loss behavior, or complete formal verification. Native batch API unit and
HTTP/2 adapter tests are described in [the API contract](RAW-BATCH-CLIENT.md).

The candidate remains on `codex/streaming-rpc-main-integration`; main is not
promoted by this process checkpoint. The fresh point Chaos matrix, atomic batch
history/checker/fault matrix, and batch-size throughput/tail-latency comparisons
remain outstanding. No hosted CI was triggered and no original #9 roadmap item
became complete from this increment.
