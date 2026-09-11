# Slicing-by-eight CRC qualification

Candidate [e5662bb](https://github.com/c4pt0r/kv9/commit/e5662bbce1443e6907e3d832a7b300881796340c)
adds an equivalent eight-byte engine WAL checksum update to selected source
`ca0002c7`. Its local source tests and source-bound Lean proof pass. It remains
experimental: no new database throughput or latency result is established,
and it is not selected on master.

The implementation preserves the reflected IEEE polynomial, initial state,
final complement, checksummed bytes and log format. Full eight-byte blocks
use eight independent table contributions; remaining bytes use the existing
byte table. Safe slice iteration and explicit little-endian decoding support
unaligned inputs without hardware CRC assumptions, allocations or unsafe code.
Raft quorum, write/sync ordering, fencing and dual-WAL recovery are unchanged.

## Correctness evidence

Local runs pass **145 engine tests/doctests (22 ignored)** and **710 workspace
tests/doctests (23 ignored)**, plus warnings-denied Clippy and formatting. The
new regression checks every alignment 0..7, length 0..128 and split point
against the independent bitwise oracle. Existing known-vector, exhaustive
two-byte, long-input, fragmentation, torn-tail and corruption tests remain.

The [new formal gate](https://github.com/c4pt0r/kv9/blob/e5662bbce1443e6907e3d832a7b300881796340c/proofs/lean/crc32-slicing8/README.md)
passes in both its prepared and repository-integrated forms. Each invocation
freshly checks **47 distinct theorem statements**, then the same 47 in a
restored run. **94 counts repeated checks**, including dimension and table-entry
lemmas. The universal statements cover every 32-bit state, eight input bytes,
arbitrary finite input lengths, all tails and arbitrary fragment boundaries.
All **256 fallback and 2,048 slicing entries** are emitted by compiling the
actual extracted Rust constants, checked by kernel reduction, and used by the
universal compiled-table theorems.

All eight controls are rejected for their intended cause: a compiled wrong
recurrence, a compiled wrong polynomial, a wrong block row, a proof hole, a
custom false axiom, big-endian decoding, a wrong tail shift and a missing final
complement. The custom-axiom source compiles but fails the semantic axiom audit.
The source controls fail before compilation. The allowed transitive axioms
remain `propext`, `Classical.choice` and `Quot.sound`; no native-evaluation axiom
or unchecked proof is admitted.

The historical byte-table checker remains unchanged and rejects the changed
function. The new gate binds both committed predecessors and explicitly checks
the new block. Rust primitive and standard-library iterator semantics remain
reviewed premises. This is CRC arithmetic equivalence, not a whole-Rust,
compiler, WAL recovery, persistence or Raft proof. Earlier failed elaborations,
the finite-index rewrite failure and two checker-control failures are retained.

The [evidence index](../scripts/redis-reference/crc-slicing8-proof-v1/index.json)
publishes **478 exact files / 2,999,228 bytes**: source and test logs, proof
attempts, command/exit records, emitted table values, semantic controls and the
integrated gate. Compiled native executables and Lean object files stay local;
the publication is a reproducible selection, not a complete environment image.

## Original release and process recovery

The original default-feature release binds all **596 source files** to clean
`e5662bb`. The server SHA-256 is
`1478689c90c191d728f646a60de9b3b5f22174c3833bd5b4baa7abbad08bf3ea`;
the separately copied workload is
`9f7e40de023e21b1e5b289a924e64c4ebee848a97699a184b7a0b1ace6ec8f42`.
Both original Cargo invocations use the release profile and default features.

The three-voter ordinary-WAL fixture and unchanged independent auditor pass
complete atomic batch histories with overlapping point operations, leader
termination and original-directory restart:

| Transport | Complete calls | Successful | Unknown outcomes |
| --- | ---: | ---: | ---: |
| Tonic streaming | 183 | 169 | 14 |
| Explicit tonic unary | 177 | 164 | 13 |
| Total | 360 | 333 | 27 |

Unknown outcomes remain in the checked histories and are not relabeled as
successful or failed writes. Both cases show progress during voter loss and
after restart, with fresh Serving/empty-drain observations for all three voters.
All five server lifetimes and two client lifetimes exit. The source, copied
binaries, command/process identities, full histories and original input hashes
are checked again independently.

The [process evidence index](../scripts/redis-reference/crc-slicing8-process-v1/original-path-index.json)
retains exact build/provenance, full histories, runtime observations, independent
checks and terminal records. Original WAL directories and executables remain
local. This is same-host process-fault recovery evidence, not Chaos Mesh,
power-loss durability, a cross-host test or a performance measurement.

## Performance interpretation and next work

The [post-CRC CPU profile](POST-CRC-CPU-PROFILE.md) attributes 18.079% of selected
batch-write leaf samples to engine `frame_crc`. Its separate Raft FNV loop is
not changed by this experiment. A [warm-cache standalone checksum screen](../scripts/redis-reference/crc-slicing8-kernel-v1/index.json) shows
about 4.48x / 4.56x lower computation time for 8/16-KiB inputs, while very short
inputs slightly worsen. Those function timings are not database QPS, latency,
sustained capacity or an equal-durability comparison.

Durable Raft writes necessarily pay persistence, majority replication and
network confirmation costs. Current memory benchmarks use tmpfs with normal
sync calls, while standalone Redis disables persistence. Its write throughput
is a reference, not a requirement to match after dropping those guarantees.
After bounded CRC qualification, the development priority returns to GET
throughput and single-request latency with linearizable ReadIndex and apply/view
checks. Exact-source Chaos Mesh and paired c1/c64 read/write/mixed measurements
remain required before general selection. Checks run locally; no hosted CI
was dispatched.
