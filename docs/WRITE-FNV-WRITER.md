# Bounded FNV writer integration: source qualification

The experimental Raft writer now uses the proven four-lane checksum kernel
for up to four already available entry bodies, with a **64 KiB total body
budget**. Source `12f44d35590ede5f89337fe731dd950162865154` passes the composition
proof and local workspace checks. It remains an isolated candidate; there is
**no new database QPS or latency result**, and CRC main remains selected.

## Implementation and correctness

Each existing `write_entries_unsynced` call stages bodies in input order.
Four bodies use independent checksum accumulators. Incomplete groups use scalar
checksums; a budget boundary flushes previous bodies, and an oversized body
takes the original single-record path. A final flush completes every call.
There is no cross-Ready queue or wait for future requests.

Frame encoding and individual ordered frame writes stay unchanged. A group
releases its body allocations as it writes; it retains no old body capacity
between groups. The budget covers logical/requested body bytes, including kind
bytes, independently of Ready length. Original protobuf serialization and one
frame allocation still exist; allocator overhead is outside that logical bound.

The [source-mapped Lean gate](https://github.com/c4pt0r/kv9/blob/37a491eea64a8cd4acc425939335d21c42f1b392/proofs/lean/fnv-writer/README.md) passes **15 writer
statements plus six kernel statements**, with **seven writer and six kernel
rejection controls**. Four kernel equivalence tests also pass in debug and
optimized standalone builds. The writer model proves ordering, complete-stream
and checksum/frame equivalence, staging bounds and legal short-write prefixes.
It compares the existing writer-failure, append, Ready sync/publication and
replay declarations exactly against committed CRC main. Rust primitives,
protobuf, file and compiler semantics remain explicit premises; this is not a
proof of the entire Rust implementation or Raft protocol.

The first development proof had tactic/type errors. Two full checker attempts
then found control-construction errors; both are retained with their exact
inputs and nonzero exits. The final corrected checker passes. Production code
did not change to accommodate these controls.

## Local validation

The clean `12f44d3` checkout passes all seven source-gate commands. Focused Raft
storage checks pass **35 tests**; the subsequent full workspace run passes
**797 tests/doctests, with 23 existing ignored**. The focused population is a
subset and must not be added to the workspace total. Formatting, warnings-denied
Clippy and explicit experimental-lease compilation also pass. No source changes
occur during qualification; first-party artifact invalidation and actual
recompilation are recorded under the existing shared-cache lock.

Five new persistence tests cover:

- 70 complete Ready byte-stream comparisons against an independent legacy
  encoder, with immediate loss-of-unsynced recovery. Cases include zero through
  thirteen entries, exact four-body capacity, three-body budget flushes, mixed
  lengths and oversized scalar fallback.
- Body-inclusive budget admission, large-length rejection and allocation
  capacity release after flush.
- 632 mixed-size Ready write/sync/short-write/error/crash combinations, including
  short headers and bodies, no memory publication, writer poisoning and stable
  reopened prefixes.
- 828 replacement-suffix failure/recovery combinations crossing a four-entry
  group and scalar tail, preserving committed prefixes and suffix identity.
- 52 failures through the actual raft-rs follower Ready loop, verifying that a
  failed seven-entry group cannot return append acknowledgments.

The **1,512 new failure combinations are deterministic filesystem-model tests**,
not actual Chaos Mesh events. Existing Ready, lease and recovery tests remain.

The source supervisor starts with 114,609,889,280 available bytes and observes
a minimum of 113,493,303,296 bytes, above its **96 GiB floor** and within its
**8 GiB consumption reservation**. Actual session `16443` ends at `3e6304/0`;
the supervisor is reaped. These observations do not reserve the next campaign.

[Portable original evidence and failed attempts](https://github.com/c4pt0r/kv9/blob/37a491eea64a8cd4acc425939335d21c42f1b392/docs/fnv-writer-source-v1/README.md)
include every source/proof/checker record and an independent full archive
readback. Hosted CI remains manual and was not dispatched.

## Next acceptance gates

1. Build a clean exact-source default release and independently verify source,
   feature, compiler and binary bindings.
2. Run ordinary recovery and the actual 21-window Chaos Mesh campaign, retaining
   complete histories, unknown outcomes, original failures and all owned-process
   exits. Previous candidates' Chaos results do not qualify this binary.
3. Run the matched point Put and BatchPut(64), c1/c64, opposite-order throughput
   and latency screen against selected CRC main. Preserve all quorum, sync,
   apply and response fences and capacity checks.
4. Select only from database results. The standalone equal-body checksum speedup
   and CPU profile do not establish Ready-group frequency or database gains.

Latest accepted database results remain **139,188.639 point Put/s** and
**1,065,680.142 BatchPut(64) items/s** at c64, with respective p99 intervals
**737.280–745.471 us** and **6.947–7.012 ms**. These are the existing shared-host
volatile-tmpfs panel; Redis was not rerun, and equal durability is not claimed.
This source checkpoint closes no original industrial roadmap work package.
