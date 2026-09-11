# Linux server allocator experiment

The accepted parallel-stream implementation increases useful concurrency. Its
new instrumented profiles still place approximately 23% of selected CPU leaves
in allocation/copy/comparison and 15% in scheduling/synchronization. Allocator
symbols include malloc/free and allocator-internal chunk management. These
observations motivate a controlled allocator experiment; they do not predict
a throughput gain or establish a complete latency breakdown.

On Linux, the `kv9` binary selects `tikv-jemallocator` 0.6.1 as Rust's global
allocator. The dependency is exact-version pinned, with default features off.
Its prefixed jemalloc API serves Rust allocations; C allocator interposition,
profiling and statistics features are not enabled. Background threads are off
by default, but pthread/background-thread support remains compiled on the
measured platform. No allocator environment or configuration-file tuning is
introduced. Record the absence of allocator build/runtime overrides in each
exact build and run; default features alone do not prohibit such overrides.
Other targets retain the existing
allocator. Library crates and the separately built benchmark clients do not
declare a global allocator and remain independent of this binary selection.

The [upstream allocator implementation](https://github.com/tikv/jemallocator/tree/0.6.1)
maps Rust's `GlobalAlloc` contract to jemalloc, including requested alignment,
zeroed allocation and reallocation. The experiment assumes that implementation
meets the allocator contract. It is not a new formal proof of the allocator.
Allocator exhaustion and process failure remain outside the successful-memory
allocation assumptions of the existing algorithm proofs.

The request framing, authentication, stream task/response bounds, public
admission, metadata validation, Raft quorum/read barriers, exact apply receipts,
WAL sync operations and retry/unknown rules are unchanged. No allocator-specific
object is stored in durable state. Scheduling and failure timing may change,
so source correspondence alone is insufficient for runtime acceptance.

## Experiment gates

The existing binary tests cover basic allocation and CLI behavior; library test
executables do not inherit the binary's allocator. Use existing actual-process
histories for concurrent allocation under stream/unary traffic, leader loss and
original-directory restart, then run the matched diagnostic with
the accepted parallel-stream server as control. Keep the native and Redis
clients fixed. Report whole-call latency, successful throughput, outcomes and
per-process memory together. A lower CPU cost does not imply lower retained
memory: allocator arenas and caches can increase RSS.

The dependency adds native allocator code and a build step; retain its exact
lockfile, emitted allocator features and the actual release build script's
configuration/archive identity. Only a useful measured candidate proceeds to the
remaining workspace, process and actual Chaos Mesh acceptance gates before
selection or default promotion. Preserve rejected experiments and all original
artifacts. This branch alone makes no performance or production-readiness claim.
