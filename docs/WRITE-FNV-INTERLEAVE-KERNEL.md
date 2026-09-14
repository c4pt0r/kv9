# Four-lane legacy FNV kernel results

The standalone four-lane kernel passes source-bound equivalence proof and
tests. On this AMD Ryzen 9 9950X, four equal 10,601-byte bodies take **7.630 us**
instead of **30.286 us** for four serial checksums, a **3.969x kernel speedup**.
Four equal 206-byte bodies improve **3.342x**. Both execution orders agree.
This qualifies a bounded Raft-writer integration experiment; it is not a
database QPS improvement or a selected runtime change.

The source prototype is commit `c00b57452360464d8d67acd055fb92280ba834b9`.
`crates/raft/src/storage/fnv.rs` remains unlinked to the database writer in
this checkpoint. It borrows four slices, uses four independent accumulators
and scalar tails, and allocates no memory. Existing Raft storage bytes, reads,
quorum confirmation and synchronization are unaffected by this standalone test.

## Correctness

The [source-bound Lean gate](https://github.com/c4pt0r/kv9/blob/2fe34ba33eabe74d6a2468aefacd8a2774886597/proofs/lean/fnv-interleave/README.md) passes
six distinct universal statements, covering arbitrary initial states, empty
and unequal inputs, lane independence, continuation boundaries and the common
prefix bound. Four actual Rust equivalence tests pass in both debug and
optimized standalone builds: eight passes, four distinct tests. They include
the checksum function extracted from the committed original writer.

Six controls reject cross-lane consumption, a proof hole, a custom false axiom,
and incorrect Rust prime/lane/tail expressions. The source contract binds
the exact kernel and model. Rust primitive, indexing, iteration and compiler
semantics remain explicit premises. This proves computation equivalence under
that mapping; writer failure behavior and the whole Raft protocol require
their separate composition and runtime gates.

Actual complete proof session **81919**, terminal **6e45e3/0**. Earlier concrete
bit-vector proof normalization exceeded a reasonable development memory budget
and was stopped; a bounded diagnostic also found an invalid local name. Both
failures remain retained. Proving generic fold scheduling first and specializing
to FNV resolves the normalization cost without weakening the statement. The
full source/axiom/test/control gate then passes on its first invocation.

## Fixed kernel measurement

The [benchmark and runner](https://github.com/c4pt0r/kv9/blob/2fe34ba33eabe74d6a2468aefacd8a2774886597/experiments/fnv-interleave/README.md) include the
exact module, use matching non-inlined serial/interleaved wrappers and black-box
the function pointer, inputs and returned hashes. Deterministic buffers are
allocated before timing. Each row has 32 warmup groups, then a fixed group
count targeting 64 MiB, bounded to 256–1,000,000 groups. Zero-byte groups use
1,000,000 iterations. Three repetitions in each complete opposite order give
108 rows. No count is chosen from an observed speed and no row is omitted.

The fresh standalone build uses rustc 1.94.0, opt-level 3, ThinLTO, one codegen
unit and unwind panics, with no CPU ISA override. Benchmark binary SHA-256:
`98a6db582606ebe1b99a5704c4e1acb31e4b5d3d7c9ed2bc8393621c7bdaf884`.
Compilation terminates before measurement; the kernel is pinned to helper CPU 6.
All five recorded child processes exit. This is one shared-host hot-buffer
matrix, not a cross-host or statistical-significance claim.

| Four body lengths (bytes) | Serial ns/group | Interleaved ns/group | Pooled speedup | Forward / reverse speedup |
| --- | ---: | ---: | ---: | ---: |
| 0, 0, 0, 0 | 4.050 | 4.694 | 0.863x | 0.876x / 0.849x |
| 32, 32, 32, 32 | 38.381 | 22.598 | 1.698x | 1.702x / 1.695x |
| 205, 205, 205, 205 | 497.044 | 148.502 | 3.347x | 3.351x / 3.343x |
| 206, 206, 206, 206 | 499.899 | 149.600 | 3.342x | 3.353x / 3.330x |
| 10600, 10600, 10600, 10600 | 30281.282 | 7675.344 | 3.945x | 3.939x / 3.952x |
| 10601, 10601, 10601, 10601 | 30286.130 | 7629.821 | 3.969x | 3.966x / 3.973x |
| 65536, 65536, 65536, 65536 | 187653.812 | 47158.150 | 3.979x | 3.990x / 3.969x |
| 205, 205, 10600, 205 | 7930.438 | 7573.782 | 1.047x | 1.047x / 1.048x |
| 0, 205, 10600, 32 | 7684.505 | 7685.716 | 1.000x | 1.000x / 1.000x |

Totals are pooled by work/time, not averages of rates or ratios. Raw rows retain
all byte counts, elapsed times, order and repetitions. Zero-byte throughput in
MiB/s is unavailable. `ns/group` is elapsed cost per four-frame kernel call;
there is no individual-request latency histogram or p99 claim.

The initial retained-WAL header prefix sample emphasized 205/10600-byte bodies.
A subsequent complete header traversal finds mature 206/10601-byte bodies
after the log-index width changes; both boundaries are represented. Those
header observations include setup/warmup/drain records and do not identify
measured-only distribution or the frequency of four-entry Ready batches.
No original WAL body is used as microbenchmark data.

Fresh disassembly shows four independent scalar multiply chains in the common
loop. A very uneven group spends most of its work in the scalar tail and gains
only 1.047x; a zero-length lane removes common-prefix work and is effectively
unchanged in the skewed case. Empty groups are slower by about 0.64 ns. These
cases support a bounded grouping policy and scalar fallback, not an assumption
that every write can achieve the equal-body kernel speedup.

## Writer integration contract

Next add staging for at most four existing entry bodies with a **64 KiB total
body budget** inside one `write_entries_unsynced` call. Flush at the existing
call boundary; do not wait for future requests. Four staged bodies use the
proven kernel. Incomplete groups use scalar checksums, and oversized bodies
flush pending work before the original single-record path. Additional staged
body memory must stay bounded independently of the Ready length.

Preserve the exact frame encoding, ordered frame writes, existing Ready
synchronization and memory-publication boundary. A failed append/sync must
continue to poison the writer and forbid success acknowledgment. Encoding
ahead can change how much unacknowledged data precedes a failure; prove that
every such persisted prefix is still a legal prefix of the original frame
stream, without inventing a committed result.

Required next checks: complete byte-stream equivalence across group/budget
boundaries, failure/short-write/sync poisoning and recovery tests, source-mapped
composition proof, clean default release, actual recovery and Chaos Mesh,
then matched point/batch c1/c64 throughput and latency. The current CPU profile
does not measure four-entry Ready frequency, and this kernel result supplies
no database speedup. Keep that uncertainty explicit through selection.

## Evidence

[Original proof, tests, failed development attempts and all kernel rows](https://github.com/c4pt0r/kv9/blob/2fe34ba33eabe74d6a2468aefacd8a2774886597/docs/fnv-interleave-kernel-v1/README.md)
are retained in a portable archive, with local executable hashes recorded.
Actual microbenchmark session **17991**, terminal **326c39/0**, includes a
successful compile followed by the one fixed matrix. Source/compiler/binary
hashes remain unchanged. No database workload, Chaos campaign or hosted CI
was run for this standalone checkpoint. It closes no original industrial
work-package checkbox.
