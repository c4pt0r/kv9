# Current CRC main write CPU profile

The exact selected CRC main release now has accepted CPU attribution for point
Put and BatchPut(64). The clearest isolated batch hotspot is the legacy Raft WAL
FNV loop: **365 / 3,106 selected samples (11.751%)**. The point workload spends
**166 / 3,259 samples (5.094%)** in the applied-receipt linear search. These are
on-CPU observations, not latency fractions or predicted database speedups.

The [frame-buffer experiment](WRITE-FRAME-BUFFER-CRC-PERFORMANCE.md) remains
unselected. The runtime remains `bd42e60f84657e22e36e34924a5c80a08eac623a`.
This diagnostic changes no runtime code, consistency rule, storage format or
read policy. It closes no original industrial work package.

## Exact inputs and protocol

The original clean default ThinLTO server is SHA-256
`106f517c84fabe790ea139f3c18661e14447abd9e257fdd6ba82bfabe6796eb9`;
its original build manifest is
`9005c064129e4a70b9de881e46074c4b7b5ec493e482095dd6fa7884fccdfb4a`.
Fresh preflight verifies all 859 declared server source files, the original
build command and default-feature Cargo artifacts. The unchanged native v3
client is revision `0be806d9671e2c50701a64aa7889c8859b7648ba`, binary
`8da9af469f962a938027d1970141bbe4622f7d42b2b795f720e288fb3f8d5957`,
with all 581 declared client sources checked. No rebuild is substituted.

Each API uses a fresh three-voter cluster, 64 workers, 4,096 keys plus sentinel,
128-byte values, seed 71, 128 warmups and a five-second measurement window.
The fixed client keeps its 1,500 ms deadline, call accounting and validation.
Clients use CPUs 0–1, voters 2–5, helpers 6–15 and 22–31. WAL is volatile
tmpfs; normal Raft quorum, synchronization and commit/apply response fences
remain. This is a shared-host CPU diagnostic without a new container-isolation
protocol, a disk-durability result or a cross-host performance claim.

Each owned-PID `perf` recording lasts 20 seconds, uses `cpu-clock` at 199 Hz,
DWARF call graphs with 16 KiB stacks, mono clock and a 128 MiB cap. The retained
read-only prefix issues 128 paced absent-key RawGet calls before each measured
workload. Its completion does not substitute for actual sample coverage.
The prefix, independent decoder and cleanup helpers are byte-identical to the
previous accepted mechanism; their original controls remain bound. The older
`ca0002c7` byte-table CRC profile is historical and is not relabeled as this
slicing-by-eight release.

## Accepted observations

| Observation | Point Put | BatchPut(64) |
| --- | ---: | ---: |
| Selected CPU samples | 3,259 | 3,106 |
| Recorded CPU samples | 3,510 | 3,156 |
| Successful measured calls | 649,437 | 80,677 |
| Other measured outcomes | 0 | 0 |
| Sample loss | 0 | 0 |
| Clock-anchor offset spread | 225 ns | 424 ns |
| Raw perf bytes | 58,662,664 | 52,702,772 |
| Retained database bytes | 751,201,387 | 5,136,700,693 |

Both original decodes pass nominal-interval containment, a 1 ms exclusion at
each edge, all 32 aggregate bins, samples within 50 ms of each retained edge,
identical sample periods, owned PID scope and raw-file identity. Each voter
also happens to cover all 32 bins. Strict workload/dataset checks, retained
database hashes, scratch removal and owned-process cleanup pass. The 256
priming reads are separate from measured calls. The four fixture/client and
two observed profiler lifetimes per API exit, as do both stage supervisors and
their children. No workload or decoder retry was needed.

| Sample view | Point Put | BatchPut(64) |
| --- | ---: | ---: |
| Raft WAL FNV loop, exact instruction ranges | 64 / 1.964% | 365 / 11.751% |
| Engine frame CRC function | 32 / 0.982% | 152 / 4.894% |
| Applied-receipt linear search, exact instruction range | 166 / 5.094% | 32 / 1.030% |
| Persistent ordered-map category | 102 / 3.130% | 429 / 13.812% |
| Allocation/copy/compare category | 460 / 14.115% | 735 / 23.664% |
| RPC/framing/serialization category | 430 / 13.194% | 76 / 2.447% |
| Kernel network category | 115 / 3.529% | 64 / 2.061% |

The two instruction views use fresh disassembly of this exact binary.
`write_record_unsynced` starts at `0x71e810`; its FNV loops occupy relative
half-open ranges `[0x120, 0x199)` and `[0x1b0, 0x1c7)`.
`inspect_applied` starts at `0x70feb0`; the linear search occupies
`[0x120, 0x137)`. Every selected offset in these views maps to a decoded
instruction. The batch frame-writing function has 366 leaf samples overall;
365 fall in its FNV loop. The compiler already unrolls that loop eight bytes
at a time, while successive hash values remain dependent.

The original category rules are retained. They do not collect every jemalloc
internal symbol under allocation: a separate explicit named-allocator view
finds 233 point samples (7.149%) and 481 batch samples (15.486%). That view
overlaps the original categories and must not be added to them. Recovered
inclusive stacks overlap, optimized/async unwinding is incomplete, and
`cpu-clock` instruction sampling can skid. These limits prohibit interpreting
the table as an exact breakdown of request latency or causal speedup.

## Next implementation

The subsequent [standalone FNV interleaving experiment](WRITE-FNV-INTERLEAVE-KERNEL.md)
now passes its computation proof, Rust equivalence tests and fixed kernel
measurements. Its [database writer integration and matched screen](WRITE-FNV-WRITER-PERFORMANCE.md)
have since completed: loaded batch throughput improves 4.131%, but pooled p99
worsens, so CRC main remains selected. The following preserves the hypothesis
established by this CPU checkpoint. The current next experiment is the separate
[receipt tail hint](WRITE-RECEIPT-TAIL-HINT.md), whose proof, source, release,
recovery and actual Chaos qualification pass; its matched timing remains pending.

Start with a bounded experiment that interleaves independent legacy FNV states
for entries already present in one Raft Ready. Establish a finite memory bound
and a scalar fallback for short or oversized groups. Prove each lane computes
the identical bytewise FNV recurrence before touching the database writer.
Plain unrolling is insufficient because the current compiler already does it.

Any subsequent writer integration must preserve frame bytes, record order,
the existing Ready synchronization/publication boundary and failure poisoning.
It needs focused compatibility/failure tests, the required actual recovery and
Chaos Mesh histories, and a matched throughput **and latency** screen before
selection. Kernel-only gains would not establish database improvement. No such
candidate or performance gain is claimed by this profile.

Receipt lookup and persistent-map ownership remain secondary targets. The
earlier indexed/deque receipt experiment had mixed batch results; simply
repeating that implementation is not justified. Existing engine/Ready group
commit must not be reimplemented. DPDK still requires cross-host/NIC evidence.
The [development order](WRITE-PERFORMANCE-NEXT.md) retains dynamic multi-Raft,
routing, membership, splits and no-singleton requirements after the write phase.

## Evidence and execution

[Portable source, samples, reports and execution receipts](https://github.com/c4pt0r/kv9/blob/13cdb54175e170eed6493d4187dab746a95c9ee8/docs/crc-main-current-cpu-v1/README.md)
are published on a separate evidence branch. Independent streaming readback
checks all 803 archived files: 36,908,139 decoded bytes, 4,117,081 compressed
bytes in two parts. Inventory SHA-256:
`fa32fdc7d5894129c396fee58a7b44bdcaf0af74417043347d100a0c970befe3`.
Actual package terminal is **3d7283/0**; independent readback is **f6df3d/0**.
Portable-byte verification is separate from the original runtime/decode gates.

Original local recording and decode:
`/tmp/kv9-crc-main-current-profile-preparation-20260914-first`.
Root preflight, exact-binary disassembly, reproducible instruction attribution
and actual tool receipts:
`/tmp/kv9-crc-main-current-profile-root-20260914-first`.
The preparation inventory is
`930f7224e89bcda753bacc6690f75af8214f4b2cf4d2abde0f8a3b5aae019b2a`.

Actual profile session **49065**, terminal **16d5c2/0**; independent decoder
session **13979**, terminal **a9d140/0**. Fresh disassembly **9e1ce0/0** and
instruction attribution **3c36fa/0** follow those terminals. Source hashes are
unchanged through both supervised stages. Preflight observes 121,114,349,568
disk bytes available; post-analysis observes 115,073,114,112. Raw recordings,
database payloads and executable originals remain local. No additional cleanup
or hosted CI was performed for this diagnostic.
