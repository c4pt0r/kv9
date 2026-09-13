# Experimental vectored writes for segmented WAL frames

This isolated change starts from selected `11113f6`. `WalSegment::append` writes
the existing header, payload and checksum through one vectored operation when
the writer accepts the full frame. The old path called `write_all` separately
for each of those three buffers. No combined frame allocation is introduced.
The CRC and frame-buffer experiments remain separate.

The helper uses stable [`Write::write_vectored`](https://doc.rust-lang.org/std/io/trait.Write.html#method.write_vectored)
and [`IoSlice::advance_slices`](https://doc.rust-lang.org/std/io/struct.IoSlice.html#method.advance_slices).
It advances only by a successful write's returned byte count, including writes
ending inside a buffer. Interrupted calls preserve the suffix; zero progress
returns `WriteZero`; other errors return immediately. The caller retains its
existing poison fence and performs `sync_all` only after the complete frame
write succeeds. It publishes the new segment summary only after sync succeeds.
Header, payload encoding, CRC inputs, replay, position checks, size limits,
Raft commit and acknowledgment logic are unchanged.

## Stream refinement argument

Let `F = header || payload || checksum`, with length `L`. The loop invariant is
that exactly `F[0..k)` has been emitted and the concatenation of remaining
slices is `F[k..L)`, where `0 <= k <= L`. Initially `k = 0`; removing empty
slices does not change their concatenation. Under the standard writer contract,
a successful call returns `0 < n <= L-k` and emits exactly the next `n` bytes.
Advancing the slices by `n` therefore preserves the invariant with `k' = k+n`.
Three universal SMT queries check emitted-byte equality, cursor bounds/progress,
and complete-frame success. Three incorrect variants must produce countermodels:
skip a byte, over-advance the cursor and omit the checksum.

An interrupted call emits no bytes under the writer contract and leaves the
invariant unchanged. Other errors cannot return success. When no slices remain,
`k = L`, so success implies the same complete frame as the former three writes.
Each successful nonzero step decreases `L-k`; termination assumes calls return
and interruptions do not continue forever, as with the former `write_all` path.
Failure may leave an uncertain prefix, which is handled by the unchanged writer
poison/recovery rules. A single vectored write does not make a frame atomic or
durable by itself.

The proof assumes the Rust `Write` and `IoSlice` contracts, slice memory safety,
compiler correctness and the underlying file implementation. It is an abstract
stream refinement proof, not a whole-Rust or whole-Raft proof. The checker binds
the helper and requires every other byte of the original segment module,
including sync/publication/error handling and existing tests, to be unchanged.

## Local qualification and remaining gates

The three universal SMT checks and three countermodels pass. Local qualification
passes **714 workspace tests and doctests**, formatting and Clippy, with 23
existing tests ignored. Five new tests exercise two short-write boundaries,
interruptions, zero progress, terminal errors, empty buffers and scalar-writer
fallback. Existing tests cover real write/sync failures, poisoned handles,
recovery, checksums and positions. The first check stopped on test formatting;
its failure is retained, and only that formatting changed before qualification.

A separate `strace` observation of the existing three-append file/replay test
records exactly **three `writev` calls**, each consuming all three buffers and
immediately followed by a successful `fsync` of the same file. The writes are
54, 56 and 47 bytes. Segment creation, sealing and recovery retain their other
syncs. This repeats one of the 714 tests under tracing; it is not an additional
distinct test or a performance benchmark.

[Original evidence](wal-segment-vectored-source-v1/README.md) retains both source
check attempts, proof queries/results and syscall output. Source checks use the
existing BuildCache lock and first-party invalidation, four offline jobs, helper
CPUs, a 96 GiB preflight, 80 GiB runtime floor, 16 GiB consumption guard,
five-second samples, 900-second command limits and a 1,200-second outer limit.
Exact release, ordinary recovery, actual Chaos Mesh and matched throughput/
latency with full regressions remain required before promotion. No new QPS or
latency result is claimed.
