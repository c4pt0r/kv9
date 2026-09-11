# Point and batch write CPU: engine CRC is the first target

The exact general baseline `5ee897a` spends a substantial sampled CPU population
in the engine WAL's bitwise IEEE CRC loop: **7.807% for point PUT and 41.903%
for BatchPut(64)**. This changes the next implementation priority from generic
copy/scheduling work to an equivalent faster checksum computation. The
[full workload comparison](V3-WORKLOAD-PERFORMANCE.md) remains the uninstrumented
throughput/latency evidence; this profile does not establish a speedup.

## Exact recording and retained population

The server is the unchanged default release at `5ee897a2f58c57bdf17ea1757c96224adf0f0dbb`,
SHA-256 `0d5ffa081482945d213b88aef12222afab44a44ed03bf46cf27bd292db7b1711`.
Both runs use the same v3 native client `0be806d9671e2c50701a64aa7889c8859b7648ba`,
SHA-256 `8da9af469f962a938027d1970141bbe4622f7d42b2b795f720e288fb3f8d5957`.
All 584 server and 581 client source files and original Cargo/build records
match before and after recording. The original v3 fixture performs startup,
bounded public admission, fresh drains, final deterministic readback and owned
process/storage cleanup without production source instrumentation.

Each separate cohort uses c64, 4,096 keys plus sentinel, 128-byte values,
128 warmup calls, seed 71, a 1,500-ms deadline, the ten-million-call cap and
five seconds of write-only measurement. Point calls use actual RawPut; batch
calls contain 64 keys. Clients run on CPUs 0-1, the three voters share 2-5 and
the profiler runs on 6-15,22-31. This is a shared host; background containers
are not isolated for these CPU diagnostics. Normal Raft quorum and sync calls
remain enabled with tmpfs WAL data. No equal-durability, real-NIC, production
capacity or candidate-performance claim follows.

Perf attaches only to the three owned voter PIDs. Each recording is bounded
to twenty seconds and 128 MiB, with `cpu-clock`, 199 Hz, monotonic timestamps
and DWARF call graphs with a 16 KiB stack window. Analysis selects only the
client's nominal five-second measurement window, excluding one millisecond at
each edge. Setup, warmup and drain samples are excluded. The root recording
session and decoder both exit zero; all fixtures and profiler lifetimes exit.

| Observation | Point PUT | BatchPut(64) |
| --- | ---: | ---: |
| Successful measured calls | 562,638 | 48,524 |
| Other measured outcomes | 0 | 0 |
| Recorded CPU samples | 3,544 | 3,159 |
| Selected measurement samples | 3,292 | 3,100 |
| Reported lost samples | 0 | 0 |
| Raw perf bytes | 59,182,700 | 52,718,556 |
| Clock-anchor offset spread, ns | 306 | 522 |
| CRC-loop leaf samples | 257 | 1,299 |
| CRC-loop share of selected samples | 7.807% | 41.903% |

Aggregate samples and each of the three voters cover all 32 equal bins of the
selected interval. Four fixture lifetimes and two observed profiler-launcher
lifetimes exit per cohort. Tmpfs scratch is removed after retaining and
rehashing 48 files / 651,312,282 bytes for point PUT and 123 files /
3,094,418,848 bytes for batch PUT. These retained files do not turn volatile
execution into a power-loss test.

## Source and instruction attribution

`WalSegment::append` starts at `0x8cf690` in this exact executable, with symbol
size `0x918`. The half-open relative instruction range `[0x55b, 0x5eb)` reads
the next input byte and performs its eight reflected polynomial steps. Exact
disassembly shows the IEEE polynomial `0xedb88320`; the retained full-symbol
disassembly also provides the surrounding loop and write/sync operations.
The counts above select only leaf samples in this range. No instruction offset
from an older executable is used without checking this executable.

For context, the entire append symbol has 260 point and 1,301 batch leaf
samples. The broader engine/storage leaf category is 10.571% / 51.871%; the CRC
counts are a subset, not an extra category. Generic allocation/copy/comparison
is 13.335% / 13.000%, persistent-map leaf symbols 2.825% / 7.516%, and RPC
framing/serialization/buffers 13.396% / 2.258%. The full category rules and
unclassified/unsymbolized populations are retained in the analysis artifacts.

On-CPU samples omit blocked time; these percentages are not end-to-end latency
fractions. Optimized unwinding and async boundaries limit caller recovery.
Inclusive recovered stack populations overlap and must not be added. This
single instrumented recording per workload provides a concrete target, not
statistical significance or a predicted throughput multiplier.

## Implementation and remaining acceptance

Revisit the previously unselected byte-table prototype `89b9755` on the current
baseline. Replace eight bit steps per byte with a compile-time 256-entry table,
keeping IEEE polynomial, initial state, final complement, fragmentation,
stored format and checksum coverage identical. Preserve every write/sync,
commit/apply and recovery condition. Check arbitrary-state byte equivalence,
arbitrary-length/fragment induction and the actual Rust-generated table, with
strict proof-axiom auditing. Known vectors and exhaustive two-byte examples
are useful regressions but do not replace the general equivalence argument.

Candidate [ca0002c7](https://github.com/c4pt0r/kv9/blob/ca0002c7f8e9ee6f595efcc9f4151085ccce87cb/docs/CRC32-TABLE.md)
now implements this isolated change. Local validation passes 144 engine tests,
709 workspace tests, all-target Clippy with warnings denied and formatting;
22/23 engine/workspace tests remain ignored. The strict Lean gate accepts
22 theorems, all 256 compiled Rust table entries and five intended rejection
controls using only the three allowed foundation axioms. Earlier failed gates
remain retained. This proves scoped computation equivalence with the explicit
source map, not whole Rust/Raft refinement or crash recovery. The clean default
release has 590 source files and SHA-256
`b33b5d302f901aac5cf6a95449b9bb5dd68473747e011220dbed7aed2e591d13`.
Its original build manifest SHA-256 is
`8c2ea115afc1ec8b6c82224dc6449d1d2439f5f27b5758e45090589752d186e8`.

A separate c64 baseline/candidate/Redis screen will compare point PUT and
BatchPut(64) in twelve forward/reverse ten-second cohorts after six independent
two-second smoke cells. It retains complete outcomes, whole-call mean/p95/p99,
calls/s, keys/s and resource/storage guards. A useful write result still needs
broader read/mixed checks and exact-source process/Chaos Mesh acceptance before
general promotion. The owned-buffer and persistent-map hypotheses remain
separate follow-ons. No GitHub CI is dispatched.

The [completed first write screen](CRC-WRITE-SCREENING.md) now measures
**+4.476% point PUT and +38.486% BatchPut(64) pooled throughput**, with improved
mean/p99 in both repeats. Independent accounting, exact-source process
recovery and the [eleven-window Chaos fixture](CRC-CHAOS-ACCEPTANCE.md) pass;
broader workloads remain open.

The original recordings, source bindings, exact decoder and instruction
attribution remain under `/tmp/kv9-write-apply-profile-first`. Large raw WAL
and perf data are retained locally. The write-stage endpoint means from the
earlier matrix have a wider setup/readback envelope; they are not merged into
this profile's measurement-only sample population.

The [compact evidence index](../scripts/redis-reference/write-crc-profile-v1/index.json)
binds 54 exact copies (5,943,949 bytes): recording/decoding helpers, selected
samples, instruction attribution, source/build bindings, fixture summaries,
source-test logs and the accepted proof readback. Large raw recordings and
WALs remain local; the index does not claim to include every runtime input.
`root-stage-gate-first.json` is the original pre-proof record and correctly
retains its then-pending proof status; `proof/result.json` records the later
accepted gate. No historical artifact is rewritten as a later pass.
