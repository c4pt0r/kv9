# Transfer the inbound Raft vector without a second collection

Tracking: #9, #13 and #20. This isolated candidate derives from read-credit
source `57ff6851e40ed63c837189d6eb0a11190704725a`. The parent has a promising
c64 result but remains pending general promotion; see its
[screening report](https://github.com/c4pt0r/kv9/blob/455907437a8035c685164ebc637e4b3f25a2bce9/docs/READ-GROUP-CREDIT-SCREENING.md).
This additional change has no performance measurement or promotion yet.

## Representation change

`RaftInbox::drain` already returns an owned `Vec<Message>`. Its bounded queue
removal accounts for bytes, preserves FIFO order and notifies the owner when
work remains. `GrpcTransport::drain` previously created another empty vector
and pushed every returned message into it. The default production build now
returns the original owned vector directly, avoiding that second allocation,
growth and message movement.

The testing-only partition seam still refreshes the mask before draining and
filters messages by source node in the same order. That path remains compiled
only under unit tests or the explicit `testing` feature. No queue, admission,
wire format, route-generation, confirmation, apply or wakeup policy changes.

## Sequence-preservation argument

Let `I` be the finite sequence returned by the unchanged inbox drain.

In a production build, the old wrapper starts with an empty output and appends
each element of `I` once in order. Induction over the consumed prefix establishes
that its output is exactly `I`. The new wrapper transfers the vector containing
`I`; `NodeDriver::step_inner` consumes its elements in order and does not inspect
vector capacity. Thus the driver's input sequence is identical, including the
empty case. Buffer allocation/capacity is a representation detail at this seam.

For a testing build, both wrappers invoke the same source-mask predicate once
per input element, in input order, and keep exactly the unmasked subsequence.
For corresponding predicate observations, the consumed sequence is identical.
This argument does not freeze concurrent changes to the testing mask. Mask
refresh, inbox bounds, byte accounting and notification still execute in their
original locations.

The change creates no new Raft state transition or successful-read authority.
This local sequence argument does not complete the outstanding whole-protocol
formal refinement; the inherited consensus/admission proof scope is unchanged.

## Validation and next gate

The first local gate passes **218 Raft tests/doctests**, zero failures and zero
ignored tests, plus formatting and all-target Raft Clippy with warnings denied.
The ordinary library build checks the direct-vector production path; unit
tests exercise the testing-mask path. Existing transport delivery, stale-route,
partition, batching, bounds and driver tests remain unchanged. No test that
merely asserts a particular allocation implementation was added.

Logs and frozen Cargo/crate source inventories are retained at
`/tmp/kv9-inbox-drain-validation-first`; root session 44879 exits 0. These checks
do not establish a throughput/latency improvement, process E2E or source-scoped
Chaos acceptance for this new candidate. Keep the already frozen long-c1
comparison on the parent source. Any later performance screen must compare
this change against that exact parent, preserving both c1 and c64 populations
and all correctness requirements.
