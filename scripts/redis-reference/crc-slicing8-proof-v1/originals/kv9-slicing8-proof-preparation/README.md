# Slicing-by-eight CRC proof preparation

This directory contains an independently prepared, source-bound Lean qualification of the frozen slicing-by-eight prototype. It does not modify the candidate worktree and does not establish a performance result or whole-Rust/WAL correctness.

## Reproduction and integration

The new checker is separate from the unchanged byte-table checker. Copy `check-crc32-slicing8-proof.py` to the repository's `scripts/` and the following three files to a new `proofs/lean/crc32-slicing8/` directory:

- `Slicing8.lean`
- `RustSlicing.lean.in`
- `slicing-source-contract.json`

Keep `proofs/lean/crc32/` and `scripts/check-crc32-proof.py` byte-for-byte unchanged. The new checker imports the existing strict checker utilities and pins their exact hash; an intentional future utility change requires an explicitly reviewed source-contract update. Route the repository formal gate explicitly to the appropriate checker. The historical byte checker **rejects the new `crc32_parts` source contract**, and a passing old byte proof must not be presented as acceptance of the new block algorithm.

The command for this outside-repository preparation is:

```sh
env PYTHONOPTIMIZE=0 PYTHONDONTWRITEBYTECODE=1 taskset -c 6-15,22-31 \
  /usr/bin/python3 /tmp/kv9-slicing8-proof-preparation/check-crc32-slicing8-proof.py \
  --source-root /tmp/kv9-wal-crc32-slicing8 \
  --proof-dir /tmp/kv9-slicing8-proof-preparation \
  --lean /tmp/kv9-p0-tools/lean-4.33.1-linux/bin/lean \
  --rustc /home/dongxu/.cargo/bin/rustc \
  --output /tmp/kv9-slicing8-proof-controlled-NEXT-FRESH
```

The output path must be absent. The checker snapshots inputs, records each actual process PID/argv/exit, compiles the extracted Rust with standalone rustc, and compiles every Lean module from source. It uses no Cargo target and no previously compiled proof artifacts in its positive/restored runs. Negative controls reuse only positive dependency modules to isolate their intended defect.

## Mathematical scope

`Slicing8.lean` proves the block formula for every 32-bit initial state and every choice of eight bytes. Its derivation uses bitwise XOR linearity, state-byte decomposition, and eight applications of the original byte transition. No finite state/byte sampling substitutes for this universal theorem.

The recursive list theorem handles all list lengths: each full eight-byte block followed by an arbitrary trailing zero to seven bytes. Parts/checksum/fragmentation theorems admit arbitrary finite parts, including empty parts, carry the state across part boundaries, and preserve one initial all-ones state and one final complement.

`RustSlicing.lean.in` receives all 2,048 values emitted by compiling the actual `CRC32_SLICING` const declaration. Kernel reduction checks every row/byte entry. The unchanged `RustTable.lean.in` checks the actual 256 `CRC32_TABLE` entries used for tails. The universal compiled-block/list/parts theorems use these checked concrete values. Transitivity through the shared original bitwise specification establishes equivalence with the source-bound committed `ca0002c7f8e9ee6f595efcc9f4151085ccce87cb` byte-table checksum.

Only `propext`, `Classical.choice`, and `Quot.sound` are permitted. Every declared theorem has exactly one parsed Lean axiom report. `decide` is used only for finite concrete table/bounds checks and checked kernel reductions. No native-evaluation axiom, `sorry`, or custom axiom is accepted. The controlled proof-hole and custom-axiom fixtures intentionally contain rejected defects; they are not accepted proof sources.

## Source mapping and boundary

The JSON contract binds the full frozen candidate WAL hash, exact `crc32`, `CRC32_TABLE`, `CRC32_SLICING`, and `crc32_parts` declarations, the committed ca source, and the earlier committed bitwise source. Its mapping explicitly covers:

- little-endian packing of the first four bytes;
- logical 32-bit shifts, XOR, wrapping mask arithmetic, byte conversion and indices;
- consecutive, non-overlapping eight-byte chunks and their exact trailing remainder;
- tail processing with the byte table, state carried across parts, and initial/final complement.

This is a manually reviewed Rust-to-model correspondence under Rust/compiler/standard-library iterator semantics. It is not a verified Rust compiler, proof of `chunks_exact`, machine refinement of the complete function, WAL recovery proof, storage durability proof, or evidence of speed. In particular, fragment equivalence does not imply a cryptographic collision guarantee.

## Retained failures

All prior elaboration attempts remain in `/tmp/kv9-slicing8-proof-attempt1` through `attempt7`. Failures concerned shift rewriting, tuple construction, or bit-index bounds. Successful attempt7 proves all 16 new abstract theorems. The first compiled-table attempt passed every concrete slicing entry but failed its Fin-index rewrite for the compiled block; `/tmp/kv9-slicing8-compiled-attempt2` corrects that rewrite and passes all nine compiled-slicing theorems.

The first controlled gate `/tmp/kv9-slicing8-proof-controlled-first` stopped before any proof run because the new Python inventory reader missed two indented existing theorem declarations and assumed theorem/query ordering. Its frozen checker, inputs, log and terminal are preserved. The correction recognizes indentation and compares exact theorem multisets; no proof statement, source contract, trust rule, or acceptance scope changed. Terminal accepted-gate details are recorded separately in `RESULT.md` once available.

The second controlled gate proved the 47-theorem baseline and rejected all five compiled-table/block/proof/trust mutants, then stopped because its endian source-control anchor searched all of `wal.rs`, which also contains unrelated little-endian decoding. `/tmp/kv9-slicing8-proof-controlled-second` retains that failure and its exact checker. The correction restricts the same endian mutation to the already-bound `crc32_parts` declaration. It does not change the mutant's intended behavior, any theorem, the source contract, or rejection conditions.
