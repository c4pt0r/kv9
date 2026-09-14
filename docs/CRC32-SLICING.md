# Slicing-by-eight IEEE CRC

The engine processes each full eight-byte fragment block through eight
precomputed tables and keeps the byte-table transition for its tail. It uses
safe slice iteration and explicit little-endian decoding, with no alignment
requirement, unsafe access, architecture-specific instruction or allocation.
The tables occupy 8 KiB.

For the reflected IEEE polynomial, let L be one zero-byte CRC transition.
Linearity over XOR gives T[n][b] = L^(n+1)(zero_extend(b)). XOR the incoming
32-bit state into the first four bytes in little-endian order, then combine
eight table contributions T[7-i][byte_i]. This produces the same state as eight
serial byte transitions. State carries across fragments. The initial state,
final complement, checksum polynomial, WAL bytes, recovery format, Raft commit,
synchronization, durable apply and response fences remain unchanged.

## Proof and tests

The [source-bound proof](../proofs/lean/crc32-slicing8/README.md) establishes
arbitrary-state, length and fragmentation equivalence. It binds all 256 fallback
and 2,048 slicing entries emitted by standalone rustc and checks 47 distinct
Lean theorem statements. Only the established Lean foundations are permitted;
eight source/proof/table rejection controls prevent silently weakened checks.
Rust primitive, iterator and compiler semantics are explicit premises, not a
machine-checked refinement of the whole implementation.

Run the explicit slicing checker from that README. The historical byte-table
checker stays unchanged and intentionally rejects the changed CRC function.
Unit tests compare every start alignment 0..7, input length 0..128 and split
point with an independent bitwise oracle. Existing vectors, all two-byte
inputs, longer fragmented inputs, WAL corruption and torn-tail tests remain.

## Integration and measured scope

This integration copies the exact runtime/proof files from experimental source
`e748620a7b0ba326ac5f4fa8e3a6b1ff48553c6a`. Its [full regression result](WRITE-CRC-FULL-REGRESSION-PERFORMANCE.md)
passes all 24 smokes and 48 timed cohorts; loaded batch writes improve 17.119%
and mixed batch throughput 16.898%, with lower relevant p99 intervals.
Its [actual Chaos Mesh qualification](WRITE-CRC-CHAOS.md) remains separate.

Those measurements belong to the retained e748 executable, using three voters
on one shared host with volatile tmpfs WAL. The current main integration needs
its own source/build and ordinary recovery acceptance before publication;
its new binary does not inherit the measured QPS by assertion. All validation
runs locally. See the [write development order](WRITE-PERFORMANCE-NEXT.md).
