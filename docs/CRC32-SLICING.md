# Slicing-by-eight IEEE CRC experiment

Current reapplication: [selected ThinLTO write candidate](WRITE-CRC-SLICING8.md).
The historical design below retains its original source/performance scope.


This candidate starts from selected source `ca0002c7` and changes only engine
WAL checksum computation. It is not selected and has no end-to-end performance
or recovery acceptance yet. The new [source-bound slicing proof](../proofs/lean/crc32-slicing8/README.md)
passes alongside local source tests. The historical byte-table checker remains
unchanged and rejects the new function; use the explicit slicing gate.

The accepted post-CRC CPU diagnostic places 561 of 3,103 selected batch-write
samples (18.079%) in `frame_crc`. Receipt lookup accounts for 27 (0.870%).
These are sampled CPU populations, not request latency fractions or a speedup
prediction. The diagnostic includes read-only priming before measurement and
is not replacement throughput evidence.

For the reflected IEEE polynomial, let L be one zero-byte CRC transition.
Linearity over XOR yields tables T[n][b] = L^(n+1)(zero_extend(b)). For an
eight-byte chunk, XOR the incoming state into the little-endian first four
bytes. Each resulting byte contributes T[7-i][byte_i] to the next state.
This is the same polynomial state after eight serial byte transitions.
The tail retains the existing byte transition. State carries across fragments;
initial state, final complement, byte coverage and log format are unchanged.

The implementation uses safe slice iteration and explicit little-endian
decoding. Table indices are bounded bytes, with no alignment requirement,
architecture-specific instruction, unsafe access or additional allocation.
Eight tables occupy 8 KiB and may trade cache capacity for shorter dependency
chains. Short fragments use the original scalar remainder path.

Qualification proves the universal eight-byte transition and binds all 2,048
compiled table entries under an explicit loop/source mapping. Boundary and
fragmentation tests also pass. Ordinary-WAL recovery, exact-source Chaos Mesh
and paired throughput/latency measurements remain independent gates. No Raft,
sync, admission, fencing, memory representation or dual-WAL recovery semantics
change.

## Local prototype evidence

The first local engine run passes 145 tests/doctests (22 ignored); the workspace
run passes 710 (23 ignored). Clippy with warnings denied and formatting pass.
The additional regression checks every start alignment 0..7, input length
0..128 and every split point against the independent bitwise oracle. Existing
known vectors, all 65,536 two-byte inputs, long payloads, fragmented inputs,
torn-tail and corruption tests also remain in the engine run.

A first standalone function screen extracts the exact old/new declarations,
uses a safe unaligned slice, four alternating old/new orders and includes every
recorded result. Warm-cache per-call times are:

| Input bytes | Byte table ns | Slicing ns | Old/new ratio |
| ---: | ---: | ---: | ---: |
| 1 | 1.242 | 1.320 | 0.941 |
| 7 | 3.406 | 3.584 | 0.951 |
| 8 | 4.004 | 1.610 | 2.487 |
| 16 | 10.291 | 2.599 | 3.959 |
| 28 | 22.203 | 5.836 | 3.805 |
| 64 | 73.271 | 12.182 | 6.015 |
| 128 | 169.837 | 32.594 | 5.211 |
| 8,192 | 12,373.798 | 2,761.393 | 4.481 |
| 16,384 | 24,358.695 | 5,344.677 | 4.558 |

These short single-function measurements indicate that the new dependency
structure is worth end-to-end qualification. They are not a database speedup,
capacity limit, statistical significance claim or formal equivalence proof.
Small inputs expose call/loop overhead and do not improve. The local original
records are `/tmp/kv9-wal-crc32-slicing8-validation-first` and
`/tmp/kv9-slicing8-kernel-screen-first`; this experiment remains outside the
selected source and its measured release bindings.

## Formal qualification

The prepared controlled gate and the repository-integrated default-path gate
both pass. Each checks 47 distinct statements and a fresh 47-statement
restoration, including all 256 fallback and 2,048 slicing entries compiled from
the exact Rust declarations. All eight compiled-table, block-row, proof-hole,
custom-axiom and source-contract controls fail for their intended reasons.
The final integrated recording is `/tmp/kv9-slicing8-proof-integration-first`.

Earlier elaboration, finite-index rewriting and two checker-control failures
remain retained separately. The checker corrections recognize indented theorem
declarations and scope an endian mutation to the bound checksum function;
neither changes a theorem, source contract or permitted axiom. The accepted
proof uses only the existing Lean foundations and ordinary kernel reduction.
Rust primitive and standard-library iterator semantics remain explicitly
reviewed premises; this is not a whole-Rust, WAL recovery or Raft proof.

## Performance priority

Durable Raft writes must pay persistence, majority replication and network
confirmation costs. The current memory experiments retain normal sync calls on
tmpfs, while the standalone Redis reference disables persistence; their write
figures do not establish equal durability. This bounded checksum experiment
targets measured removable CPU work. After its qualification, development
returns to GET throughput and single-request latency, retaining linearizable
ReadIndex and apply/view checks. No database gain follows from the standalone
checksum timings or the proof alone.
