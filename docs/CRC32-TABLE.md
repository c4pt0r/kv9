# WAL CRC byte-table candidate

This experiment is based on `23bc58b` and changes only the WAL CRC calculation.
The current `a00e39f` PUT CPU recording attributes 182 of 3,242 selected samples
(5.61%) to the exact executable's inlined bitwise CRC loop inside
`WalSegment::append`. This observation is a CPU diagnostic, not a throughput
claim. RPC framework comparison takes priority over timing this candidate.

## Equivalence argument

Let `P = 0xEDB88320` and let the original one-bit transition on a 32-bit word be
`T(x) = (x >> 1) XOR (P if x & 1 else 0)`. All operations here are unsigned
32-bit bit-vector operations. `T` is linear over XOR: right shift is linear,
and multiplication of the low bit by the fixed vector `P` is linear.
Therefore eight compositions `T^8` are also linear.

For any state `c` and input byte `b`, let `x = c XOR b`, with `b` zero-extended.
Write `x = h XOR l`, where `h = x & 0xFFFFFF00` and `l = x & 0xFF`.
The low eight bits of `h` are zero, so its first eight transitions perform only
right shifts: `T^8(h) = h >> 8 = c >> 8`. The compile-time table contains
exactly `table[l] = T^8(l)`, computed by the original eight-step recurrence for
every value 0 through 255. Thus

```text
T^8(c XOR b) = (c >> 8) XOR table[(c XOR b) & 0xFF].
```

That is the new byte transition. Induction on any finite byte sequence gives
identical internal states, starting from the same `0xFFFFFFFF`. Both functions
return the same final complement. Empty slices perform no transitions;
partitioning input into slices preserves the byte sequence and carries the
same state across every boundary. The table has exactly 256 entries and the
masked index is always in bounds. It uses 1 KiB of static storage, no heap
allocation, unsafe code, architecture-specific instructions or dependencies.

This is a written algebraic equivalence argument, not a new machine-checked
consensus proof. It does not change the checksum polynomial, frame layout,
corruption/refusal rules, writes, synchronization calls or publication order.
No hardware CRC32C instruction is substituted for the existing polynomial.

## Local validation and current disposition

The focused CRC tests passed: existing known vectors, all 65,536 two-byte
inputs against the retained pre-change bitwise oracle, and deterministic
inputs through 65,536 bytes with empty, split and multi-chunk boundaries.
Raw log: `/tmp/kv9-table-crc-focused-first.log` (three selected tests passed).

Broader storage/restart tests and a repeated client-visible throughput bracket
remain outstanding. This candidate is retained on an experiment branch and is
not selected or promoted to main. RPC framework experiments are the next main
performance task; all comparisons must preserve Raft consistency.
