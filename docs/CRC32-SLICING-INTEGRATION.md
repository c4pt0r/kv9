# CRC main integration

The WAL checksum now processes eight-byte blocks with the proven slicing table,
retaining the IEEE polynomial, byte format and scalar tail. This removes serial
checksum dependencies without changing Raft commit, synchronization, durable
apply, response fences or Safe ReadIndex.

Runtime integration source is `bd42e60f84657e22e36e34924a5c80a08eac623a`, based
on main `69101d7`. Six runtime/proof files, including the WAL source, are copied exactly
from the qualified `e748620` experiment; no vectored/frame-buffer optimization
is combined with it. Subsequent reporting edits do not change runtime source.

## Local source, proof and recovery

All 789 workspace tests/doctests pass, with 23 existing ignored tests. Formatting,
warnings-denied Clippy and explicit experimental-lease compilation pass. The
source-bound Lean checker verifies 47 distinct statements, a fresh restoration
of the same statements, all 256 fallback and 2,048 slicing entries, and eight
intentional rejection controls. The historical byte-table checker stays unchanged
and rejects the new CRC definition. Rust primitive, iterator and compiler
semantics remain explicit proof premises; this is not a whole-Rust or Raft proof.

The clean default release uses Rust 1.94.0, ThinLTO, one codegen unit and opt-level 3.
The original BuildCache lock/invalidation checks and independent source readback
cover 859 files. Actual production server, native client and point client feature
sets are empty. The test-only pressure example's Raft dev-dependency includes
`experimental-leader-lease` and `testing`; neither feature enters those production
binaries. Seventeen controls check this exact distinction and reject missing or
extra test features and non-default production features.

Ordinary WAL recovery accepts two complete streaming/unary histories with
353 operations: 325 OK and 28 unknown, six fresh voter drains and seven exited
process lifetimes. Unknown writes remain unknown and are not replayed for success.
Proof, unit-test, ordinary-recovery and Chaos populations are separate.

Server SHA256: `106f517c84fabe790ea139f3c18661e14447abd9e257fdd6ba82bfabe6796eb9`.
Native client SHA256: `ec8002c00253fa6264a2f7bef3814e33dc871c7ef895e6163b7be38be2edffad`.
Point client SHA256: `4ecc204d985b4dd6f1a3a2a2f0c4a37f838d8a17cb22bdda6463d86b8cf998ca`.

[Original source/proof/recovery evidence](https://github.com/c4pt0r/kv9/blob/f0bd3c3ed995866f0d5d25ddded8b64ea73c0b98/docs/crc-main-integration-v1/README.md)
retains 287 files, including generated proof modules, compiler-emitted tables,
full recovery histories and WAL bytes. Its seven parts total 13,473,200 compressed
bytes; all 51,096,826 decoded bytes pass separate published-part verification.

## Actual main Chaos qualification

The exact default-feature main binary passes all 21 actual Chaos Mesh windows
and independent history, fault-effect, source and process checks. The complete
histories contain 11,316 operations:

| History | Operations | OK | Unknown | Refused |
| --- | ---: | ---: | ---: | ---: |
| CLI/catalog | 6,525 | 5,957 | 549 | 19 |
| Persistent point | 1,709 | 1,692 | 15 | 2 |
| Native atomic batch | 3,082 | 3,034 | 38 | 10 |
| Total | 11,316 | 10,683 | 602 | 31 |

Faults cover unavailable first seed, sustained failure of each voter, leader
partition, overload, network delay, EIO and ENOSPC for all three voters,
missing-log refusal and original-store recovery, replacement-PVC refusal and
original-PVC recovery, and retained-PVC endpoint migration. Unknown writes are
not replayed or counted as successful. Four final replica lifetimes publish
fresh drained states, and all 31 observed server lifetimes exit.

All six post phases pass: independent acceptance, cleanup capture, process-tree
capture, complete archive/readback, scoped cleanup and all-lifetime exit checks.
The full local archive contains 4,208 regular files and 1,062,174,968 decoded
bytes; its 100,471,419 compressed bytes have SHA256
`1b2abda397dd8d6505894d0e2f1125bd936b9b6879379e37e3bceedc60fa5935`.
The created namespace and its faults are removed only after archive readback;
all eight historical namespace UIDs remain unchanged. The earlier independent
audit remains intact and is supplemented by subsequent cleanup evidence.

[Portable Chaos evidence](https://github.com/c4pt0r/kv9/blob/a9dc6b0977bdf354b8219f2e35ef823cc7c78bce/docs/crc-main-chaos-v1/README.md)
retains all three complete histories and original faults, observers, build
bindings, feature controls, independent acceptance and cleanup records.
All 2,291 selected files and 499,798,357 decoded bytes pass separate byte
verification; 12 parts total 25,140,016 compressed bytes. Production ELF/WAL
payloads remain local with their catalogs and hashes published. Published-part
verification does not rerun live acceptance or prove excluded payload residency.

The runtime used session 94647, terminal `b9c075/0`; post session 85751 ends
`4ad876/0`. Publication selection, packaging and verification also complete.
The adapter retains a historical `vectored` namespace prefix; the exact source,
image and executable hashes identify CRC main, without a vectored-WAL change.
These checks do not close dedicated client-link/quorum-loss, cross-host,
power-loss, complete protocol-proof or scale-out gates.

## Performance and next work

This integration adds no new throughput result. The [full matched experiment](WRITE-CRC-FULL-REGRESSION-PERFORMANCE.md)
measured the retained e748 binary against selected `11113f6`: loaded BatchPut(64)
1,062,522.902 items/s (+17.119%), whole-call p99 7.406–7.471 ms versus
8.389–8.520 ms; point Put 139,532.275 calls/s (+2.203%). Mixed batch throughput
improves 16.898%; loaded pure GET changes -0.191%. These are shared-host tmpfs
WAL diagnostics for that exact source/binary, not newly measured main capacity
or equal-durability Redis results.

Next apply the frame-buffer experiment to the integrated CRC baseline, preserve
its separate original experiment, and qualify the resulting exact source before
a paired write comparison. Do not assume isolated optimization gains compose.
Use fresh disk observations before reserving the entire retained campaign; all
original resource floors and reserves remain. The isolated vectored screen found
no write gain and stays experimental. Dynamic multi-Raft and automatic splits
follow this write phase. No original industrial roadmap checkbox closes here.
All checks run locally; no hosted CI was dispatched.
