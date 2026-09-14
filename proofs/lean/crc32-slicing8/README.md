# Slicing-by-eight CRC computation equivalence

This gate extends the historical [byte-table proof](../crc32/README.md) to the
engine's eight-byte CRC update. It proves computation equivalence for arbitrary
32-bit states, bytes, input lengths and fragment boundaries, under the explicit
Rust operation and iterator mapping below. It does not prove whole-Rust
semantics, WAL recovery, persistence, Raft, collision resistance or rustc.

Run from the candidate repository with a fresh output directory:

```sh
taskset -c 6-15,22-31 env PYTHONOPTIMIZE=0 PYTHONDONTWRITEBYTECODE=1 \
  python3 scripts/check-crc32-slicing8-proof.py \
  --source-root . \
  --lean /path/to/lean-4.33.1-linux/bin/lean \
  --output /path/to/new-slicing8-evidence
```

The pinned Lean version comes from `proofs/lean/lean-toolchain`. The gate uses
standalone rustc; it does not invoke Cargo or a database. Every invocation keeps
its sources, compiler identities, commands, timestamps, process IDs, exits,
stdout/stderr, emitted tables, generated proof modules, axiom reports, result
and hashed inventory. Existing output directories are refused. A failed attempt
must remain separate from a later successful invocation.

## Statements and implementation mapping

`Slicing8.lean` derives the eight-byte transition from the unchanged IEEE
bit-step and byte-table lemmas. The zero-byte transition is linear over XOR.
Expanding eight successive byte updates and decomposing the incoming 32-bit
state into four bytes yields the eight independent table contributions used by
the Rust implementation. List induction handles all full blocks and every tail
length; another induction handles arbitrary fragments, including empty ones.

`RustSlicing.lean.in` binds those universal statements to all actual compiled
table values. The gate extracts both Rust const declarations and checksum
functions, compiles that extracted program, and records its 256 fallback and
2,048 slicing entries. Ordinary Lean kernel reduction proves every entry and
table dimension, then checks the arbitrary-state, list and fragment theorems
for the emitted tables.

| Rust operation | Model and source premise |
| --- | --- |
| `u32` shifts, XOR and OR | `BitVec 32` logical operations |
| `u32::from_le_bytes` | `packed`; `packed_bytes` proves the byte positions |
| Masks and shifts selecting a table index | `byteAt`; each value is an eight-bit index, including the unmasked `low >> 24` |
| `CRC32_SLICING[row][byte]` | `compiled_slice_entry`, for each of eight rows and all 256 bytes |
| Eight table contributions | `compiled_block_transition`, for every state and eight bytes |
| `chunks_exact(8)` iteration | Ordered, disjoint full blocks; `compiledSlicingFold` consumes eight elements per step |
| `chunks.remainder()` | Exactly the remaining zero to seven bytes, once and in order, using the actual fallback table |
| Outer fragment loop | `compiledSlicingParts` carries one accumulator across all parts |
| Initial state and final complement | `compiled_slicing_checksum_equivalence` |
| Different fragment boundaries | `compiled_slicing_fragmentation_invariant` |
| Single-slice wrapper | Unchanged wrapper and inherited singleton equivalence |

The Rust standard library's chunk/remainder ordering and primitive operation
semantics are reviewed premises, not a formal Rust operational semantics. The
source contract compares exact declarations, allowing only whitespace changes,
and also pins the entire candidate WAL source. It binds the committed byte-table
parent `ca0002c7` and the earlier bitwise source `5ee897a2`. The emitted table
proof identifies this standalone compiler invocation; a separately built release
binary still needs its own source/build provenance and runtime validation.

## Trust and rejection controls

The gate freshly checks 47 theorem statements: 22 inherited byte-table
statements, 16 slicing statements and nine compiled-slicing statements. A fresh
restored run checks the same 47 again; these are 94 checks, not 94 distinct
theorems. Table dimensions and finite-entry lemmas are included in those counts.

The existing semantic axiom checker is imported unchanged and hash-pinned.
Only `propext`, `Classical.choice` and `Quot.sound` are permitted transitively.
Proof holes, custom axioms and native-evaluation axioms are rejected. The proof
uses algebra, induction and ordinary kernel reduction, with no `native_decide`
or `bv_decide`. Warnings remain errors.

Eight controls must fail for their intended reason:

1. A compiled seven-bit table recurrence must fail the finite slicing-entry proof.
2. A compiled CRC-32C polynomial must fail the inherited IEEE table proof.
3. A wrong block lookup row must fail the universal compiled-block theorem.
4. A hole in the universal slicing proof must fail Lean's warning gate.
5. A custom false axiom must fail the semantic axiom allowlist.
6. Big-endian source decoding must fail the source contract.
7. A seven-bit tail shift must fail the source contract.
8. A missing final complement must fail the source contract.

The historical `scripts/check-crc32-proof.py` remains a byte-table gate. It must
reject the changed `crc32_parts` definition; it is not silently made to accept
slicing through its old contract. Use the explicit slicing gate above for this
candidate. That gate retains the historical rejection and freshly rechecks the
inherited proof alongside the new proof.

Source tests, ordinary-WAL recovery, exact-source Chaos Mesh and paired database
throughput/latency measurements remain independent qualification requirements.
