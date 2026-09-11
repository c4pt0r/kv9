# Equivalent byte-table CRC for the engine WAL

This candidate replaces the engine's eight polynomial steps per byte with one
lookup in a 256-entry table computed at compile time. It uses the same reflected
IEEE polynomial `0xedb88320`, initial state `0xffffffff` and final complement.
It changes computation only: WAL versions, record bytes, checksum coverage,
fragment concatenation, write/sync ordering and recovery rules remain the same.
It does not use the different CRC-32C polynomial implemented by x86's CRC32
instruction, introduce unsafe code, or add a dependency.

## Measured reason for revisiting the prototype

The general baseline is `5ee897a`. A separate local CPU diagnostic uses that
exact default-release binary with the published v3 clients `0be806d9`, c64,
4,096 keys, 128-byte values and five seconds each of actual point PUT and
BatchPut(64). Three voters retain quorum and normal sync calls on tmpfs.
The recording is instrumented, on a shared host, and is not a throughput or
durability comparison. Background containers are not isolated for this profile.

All 611,162 measured calls succeed. The decoded measurement windows contain
3,292 point-write and 3,100 batch-write CPU samples, with no reported lost
samples. Exact-binary disassembly places **257 / 3,292 (7.807%)** and
**1,299 / 3,100 (41.903%)** leaf samples inside the inlined CRC loop in
`WalSegment::append`. The symbol starts at `0x8cf690`; its selected relative
instruction interval is `[0x55b, 0x5eb)`. These samples are a subset of WAL/apply
work and are not an additive latency partition or a predicted speedup.

The byte-table idea and its independent bitwise-oracle tests already existed
in unselected prototype `89b9755`. This candidate reapplies that isolated
computation to the current baseline because the broader workload now identifies
CRC as substantial batch-write work. It does not combine read-credit, owned
mutation movement, a new RPC transport or a different persistence contract.

## Equivalence argument

Let `S(c) = (c >> 1) XOR (P AND -(c AND 1))` on 32-bit words, with
`P = 0xedb88320`. The old byte update is `S^8(c XOR byte)`.
The generated table stores `T[i] = S^8(i)` for every eight-bit index.
Linearity of the reflected polynomial step and the low-byte decomposition give

```text
S^8(c XOR byte) = (c >> 8) XOR T[(c XOR byte) AND 0xff].
```

The right-hand side is the new Rust loop. Equality holds for arbitrary current
state and byte. Induction over bytes establishes equality after any finite
input; induction/concatenation over fragments establishes the same result for
`crc32_parts`, including empty inputs and empty fragments. Identical initial
state and final complement therefore produce identical stored checksums.

The proof concerns the CRC computation and its explicit Rust-operation mapping.
It does not prove the compiler, whole Raft implementation, storage hardware or
crash model. Source-bound machine-checked proof and compiled-table verification
are kept separately from finite regression examples.

## Validation and performance decision

Regression tests preserve the pre-optimization bitwise oracle. They cover all
65,536 two-byte inputs, known IEEE vectors, deterministic data through 65,536
bytes and varied fragment boundaries. Existing engine tests exercise checksum
rejection, torn tails, positioned replay, snapshots and recovery ordering.

Local source gates pass: 144 engine tests (22 ignored), 709 workspace tests
(23 ignored), workspace all-target Clippy with warnings denied, and formatting.
The [source-bound Lean gate](../proofs/lean/crc32/README.md) checks 22 theorems,
including arbitrary-state byte/list/fragment equivalence and all 256 actual
const-evaluated Rust table entries. Its five controls reject a compiled wrong
polynomial, a proof hole, a custom axiom, an unmapped shift and a missing final
complement. Transitive axioms are limited to `propext`, `Classical.choice` and
`Quot.sound`. Earlier failed native-evaluation and certificate-reduction gates
remain retained; the accepted proof uses direct Boolean algebra and induction.
This validates computation equivalence, not end-to-end performance or recovery.

The next screen compares baseline/candidate/Redis for actual point PUT and
BatchPut(64), with the same clients and resource limits. Keep successful QPS,
whole-call mean/p95/p99, all outcomes and original attempts. A CPU sample share
alone cannot select this candidate. General promotion also requires broader
read/mixed workloads and exact-source process and Chaos Mesh acceptance.
Checks run locally; no hosted workflow is dispatched for this experiment.
