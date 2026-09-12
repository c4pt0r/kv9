# Reuse the drained Raft inbox vector

This isolated candidate starts from selected CRC runtime behavior. It does not
include the held 64-KiB Append configuration. The sole runtime edit is in
`GrpcTransport::drain`: return the inbox's already allocated vector instead of
moving each message into another newly allocated vector. In testing builds,
stable in-place filtering preserves the partition mask.

## Equivalence argument

Let `I = [m0, ..., mn]` be the vector returned by the one unchanged call to
`RaftInbox::drain`. Its bounded prefix selection, byte accounting, queue lock
and remaining-work notification all happen before the modified code.

In the default build the old loop constructs `O = []` and appends each element
of `I` in order. By induction, after k iterations `O` contains exactly the first
k moved elements; after n iterations it is `I` element-for-element. The new
code returns the owned vector directly. Neither implementation clones a Raft
message or changes its contents. The change removes the second vector's
allocation and element moves at the source level; an optimized-binary speedup
must be measured, since compiler transformations may affect realized costs.

With `test` or `testing`, the partition refresh remains before the inbox drain.
The old loop evaluates `is_masked(from)` once for each message in order, drops
masked messages and appends the rest in order. `Vec::retain` evaluates the same
predicate once per element in original order and preserves retained order.
For the same sequence of mask observations, both produce the same stable
subsequence. Message destruction has no consensus side effect. No additional
mask refresh, admission or notification occurs.

This is an ownership/representation equivalence under normal allocation and
non-panicking predicate execution. It is not a new consensus algorithm or a
whole-implementation machine proof. No read authorization, Ready processing,
persistence, commit/apply fence, deadline, routing generation or wakeup changes.
The inbox's count/encoded-byte bounds and drain limits are unchanged. The
source-level allocation reduction does not establish a process-memory bound.

## Validation and acceptance

Run formatting and existing Raft/server tests using the shared retained-build
lock. Compile/check the default and explicit testing-feature paths, including
the existing inbound/outbound partition regressions. The source change has no
new input or protocol transition requiring a duplicate implementation-shaped
test. Preserve actual test counts, ignored cases, command exits and source
identity separately.

Before a performance claim, build and retain a clean candidate executable,
check ordinary recovery, then compare with the original CRC and fixed v3 Redis
controls using complete c1/c64 point GET/mixed cohorts. Retain both repetitions,
GET-only means/tails and all outcomes; never pool incomplete attempts. Record
cache-space repair separately if necessary. A useful candidate still needs
applicable full point/batch, proof-composition and actual Chaos Mesh gates before
promotion. No such performance result or promotion is established by this doc.
