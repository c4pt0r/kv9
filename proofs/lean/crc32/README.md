# CRC-32 computation equivalence

This proof covers the replacement of the reflected IEEE CRC bit loop by the
256-entry byte table in `crates/engine/src/wal.rs`. It does not prove WAL recovery,
Rust compilation, operating-system persistence, CRC collision resistance, or
whole-Raft refinement.

Run the fresh gate from the repository root, with a **new** output directory:

```sh
taskset -c 6-15,22-31 env PYTHONOPTIMIZE=0 PYTHONDONTWRITEBYTECODE=1 \
  python3 scripts/check-crc32-proof.py \
  --lean /path/to/lean-4.33.1-linux/bin/lean \
  --output /path/to/new-crc32-evidence
```

The version is checked against `proofs/lean/lean-toolchain`. A standalone
`rustc` is required; no Cargo build, database, or benchmark runs. The output
retains all commands, stdout/stderr, exit statuses, timestamps, process IDs,
CPU affinity, sources, generated Lean files, Rust executable, const-evaluated
table, theorem axiom reports, result, and hashed inventory. An existing output
directory is refused. Failure keeps the original attempt and is not a pass.

## Statements and source map

`CRC32.lean` models a Rust `u32` as `BitVec 32` and `u8` as `BitVec 8`:

| Source operation | Model / obligation |
| --- | --- |
| `(crc & 1).wrapping_neg()` | Modular 32-bit negation in `bitStep` |
| `(crc >> 1) ^ (0xEDB8_8320 & mask)` | Unsigned shift/XOR/AND in `bitStep` |
| Original eight iterations after XOR with the byte | `oldByte`, using `bits 8` |
| Eight iterations starting at the table index | `tableEntry` |
| `((crc ^ u32::from(byte)) & 0xff) as usize` | `masked_index`: the zero-extended 8-bit index, hence 0..255 |
| `(crc >> 8) ^ CRC32_TABLE[index]` | `tableByte`, then generated `compiledByte` |
| Byte iteration | Arbitrary-list induction in `list_equivalence` |
| Old flattened parts vs new nested loops | `parts_flatten` and `compiled_parts_equivalence` |
| Initial `0xFFFF_FFFF`, final `!crc` | `checksum_equivalence` and `compiled_checksum_equivalence` |
| Arbitrary fragment boundaries, including empty parts | `fragmentation_invariant` and its compiled-table counterpart |
| `crc32(bytes)` wrapper | `singleton_equivalence` and exact source binding |

The byte theorem quantifies over **every 32-bit state and every 8-bit byte**.
It uses XOR linearity of a polynomial step, induction over the number of steps,
and the fact that the high 24 state bits simply shift eight places before they
can reach the feedback tap. List/fragment lengths have no finite model bound.

`source-contract.json` contains the exact old declarations from pinned baseline
`5ee897a2f58c57bdf17ea1757c96224adf0f0dbb` and the reviewed replacement
declarations. Only whitespace may differ. The checker independently extracts
and compares the current definitions and the old committed definitions. It
compiles the **extracted current constant table and CRC functions**, emits all
256 entries, and substitutes those actual values into `RustTable.lean.in`.
Kernel reduction then proves every entry equals `tableEntry`, followed by
arbitrary-byte, list, parts and final-checksum equivalence for that emitted table.

The source-to-model premise is the ordinary semantics of Rust unsigned
32-bit bitwise operations, wrapping negation, byte widening, bounded index
conversion and ordered slice iteration. This is an explicit source refinement
map, not a machine-checked Rust operational semantics or proof of rustc itself.
The extracted-program output binds the observed compiler's constant evaluation;
it is not a claim about a separately built release binary. Entire source and
compiler identities are retained, and all input hashes must remain unchanged
through the gate. There are no assumptions about CRC state, input values,
list length, or fragmentation in the equivalence theorems.

## Trust boundary and controls

The only allowed transitive theorem axioms are the repository's existing Lean
foundations: `propext`, `Classical.choice`, and `Quot.sound`. `sorryAx`, custom
axioms and native-evaluation axioms are rejected. Lean warnings are errors.

The final proof uses direct BitVec lemmas, Boolean case analysis and induction.
It uses neither `bv_decide` nor native evaluation. The high-mask helper checks
the 32 bit positions of the fixed constant, not possible CRC states. The table
proof uses ordinary kernel reduction (`decide`) for all 256 entries. The gate
checks transitive axiom dependencies of all 22 exported statements, including
the generated table and final fragment theorem. The shared-branch unused-simp
linter setting has no effect on theorem checking or the axiom allowlist.

Earlier development attempted the native-evaluation bit-vector tactic, which
the strict gate rejected, and direct certificate reduction, which exceeded the
unchanged command timeout. Those attempts are not accepted evidence. The final
direct proof removes both approaches instead of broadening the trust boundary
or raising the timeout.

The gate also requires rejection of:

1. A **compiled** table with CRC-32C's polynomial substituted for IEEE's;
   Rust compilation must succeed and the finite Lean entry proof must fail.
2. A proof hole in the universal byte theorem.
3. A custom false axiom substituted for that proof.
4. An unreviewed seven-bit source shift.
5. A missing final source complement.

These controls never edit the working source. Their inputs and failures remain
in separate evidence directories. Development attempts and any failed full
gates must be retained separately from the final successful gate.
