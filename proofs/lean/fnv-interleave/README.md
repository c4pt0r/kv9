# Four-lane FNV computation equivalence

This proof qualifies the standalone kernel in
`crates/raft/src/storage/fnv.rs`. It is not connected to the Raft writer yet.
The selected writer uses 32-bit FNV-1a over each record body, initialized at
`0x811c9dc5`, with byte XOR followed by wrapping multiplication by `0x01000193`.
All checksum bits and byte order must remain identical.

The kernel borrows four slices, computes their minimum length, interleaves four
independent accumulators over that common prefix, then finishes each scalar
tail. It allocates no memory: four input descriptors, four accumulators and
fixed loop bookkeeping suffice, independent of body size. A zero-length lane
makes the common prefix empty and every lane uses the scalar continuation.
This avoids per-byte lane-presence branches in the common loop. Highly unequal
inputs can retain most of the original serial cost; performance must be measured.

## Universal proof and source mapping

`Interleave.lean` proves scheduling equivalence for an arbitrary state type,
byte type and transition function, then specializes it to `BitVec 32` FNV.
The generic induction avoids forcing the kernel to normalize large concrete
bit-vector arithmetic during list-pattern equality checks.

| Theorem | Statement |
| --- | --- |
| `scalar_append` | A prefix state continued through a tail equals the complete sequential fold. |
| `four_equivalence` | For arbitrary four lists and initial states, all four returned values equal their independent scalar folds. |
| `split_equivalence` | Independently fragmented lanes preserve the same complete checksums. |
| `lane_independence` | Other input lanes and their states cannot affect lane zero; complete tuple equivalence covers all lanes. |
| `common_length_bound` | The common-prefix bound does not exceed any input length. |
| `checksum_equivalence` | Instantiation with the legacy FNV step and offset equals four complete legacy checksums. |

The model consumes four heads while all lists are nonempty, then scalar tails.
The reviewed Rust mapping is that `0..common` visits exactly those common
prefix positions, slice indexing returns that lane's byte, and `[common..]`
is its remaining suffix. `u32::from(u8)`, XOR and `wrapping_mul` map to zero
extension, bit-vector XOR and multiplication modulo 2^32. The bound theorem
supports safe indexing, while Rust slice/iterator semantics and rustc remain
explicit premises. This is not a formal Rust operational semantics, a whole
Raft proof, a stronger hash or a writer/persistence proof.

`source-contract.json` pins the complete module and proof, their function and
constant declarations, the original committed `bd42e60` writer checksum and
six theorem names. The checker compiles the exact frozen Rust module alongside
the actual extracted original function. Four tests cover short and large
bodies, all shortest-lane positions, empty/ragged tails, arbitrary initial
states, continuation boundaries, lane permutation and original-code equality;
all four run in both debug and optimized standalone builds.

The semantic axiom gate reads every transitive `#print axioms` report. Only
`propext`, `Quot.sound` and `Classical.choice` are allowed. There is no proof
hole, custom axiom, native evaluation or bit-vector decision axiom. Six
controls reject a cross-lane model, proof hole, injected false axiom, changed
prime, changed Rust lane and changed tail. These controls validate rejection
mechanisms; they are not additional positive theorem statements.

## Reproduction and first results

```sh
env PYTHONOPTIMIZE=0 PYTHONDONTWRITEBYTECODE=1 taskset -c 6-15,22-31 \
  python3 scripts/check-fnv-interleave-proof.py --source-root . \
  --output /path/to/fresh/fnv-proof \
  --lean /path/to/lean-4.33.1-linux/bin/lean \
  --rustc /path/to/installed/toolchain/bin/rustc
```

The checker refuses an existing output path and retains source snapshots,
commands, actual child identities/exits, logs, axiom reports and a hashed
inventory. Lean uses a 1,024 MiB limit and each invocation is time-bounded.
Standalone compilation and tests run sequentially; no database or profiler
is launched.

The first complete gate passes at
`/tmp/kv9-raft-fnv-interleave-proof-20260914-first`: actual session 81919,
terminal 6e45e3/0; six distinct Lean statements, eight Rust test passes across
four distinct tests, and six rejected controls. The source hash remains
`a836a03afbf5080eea9d35e040896dd823b66954ed5db356a7de88cfc0d1945d`.

Earlier development evidence stays under
`/tmp/kv9-raft-fnv-interleave-kernel-20260914-first`. The initial concrete-state
proof elaboration was explicitly stopped after excessive memory growth; actual
session 46615 ends at 500e42/143. A bounded diagnostic 57740 ends at bd255d/1,
identifying kernel normalization and an invalid local name. The generic-state
proof and corrected name pass at 1cc8ad/0 before the full qualification above.
These failures were not erased or counted as passing proof runs.

Kernel measurements, bounded writer integration, ordinary recovery, actual
Chaos Mesh and database throughput/latency acceptance remain separate gates.
