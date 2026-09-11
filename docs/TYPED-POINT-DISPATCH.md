# Typed authenticated point dispatch experiment

This isolated candidate starts at parallel-stream source `f2c4e85`. It does
not include the jemalloc experiment. The hypothesis is that common point
dispatch can avoid temporary tonic Request/extensions allocation, the extra
AuthContext clone from that extension table, and an additional async-trait
future allocation. A matched measurement must determine whether these savings
outweigh changes in future size and code layout.

The five point operations decode into `(AuthContext, protobuf message)` and
call shared inherent asynchronous methods. Unary RPC methods delegate to those
same methods. Optional tarpc uses the common point handler and is affected too;
the unary transport keeps its original outer async-trait boundary. Framing,
payload encoding, response metadata, deadlines, stream scheduling, admission
limits, backend preparation, Raft and storage code are unchanged.

## Source correspondence and assumptions

Let `a` be the trusted result of a successful per-frame authenticator call,
`m` the decoded protobuf message, and `S` the admission/backend state. Project
the old tonic request to `(a, m)` by reading AuthContext from its extensions
and taking its message. The new adapter represents this pair directly.
Allocation counts, incidental Arc counts and destruction time of unused
transport metadata are outside this application-state projection.

For each of the five operations, the following ordered transitions are the
same on both sides of this projection:

1. The stream owner validates frame identity, operation and deadline and owns
   the existing reply reservation and shared stream semaphore grant.
2. Common point decoding checks frame authorization/payload sizes, parses the
   same single authorization metadata entry, invokes the custom authenticator
   for this frame, and decodes protobuf. No authentication result is cached.
3. Stream batch item bounds run before public identity validation. They remain
   in point dispatch, preserving the unary adapter's existing different scope.
4. Public Raw operations require `AuthKind::Client` and `node_id == None`.
   Successful authentication alone does not satisfy this predicate. Unary
   retains its original check and the shared method repeats this pure check.
5. Admission reserves the same work class and `encoded_len(m)`, then validates
   RequestContext. Unknown or duplicate protobuf fields do not change this
   rule into a charge for raw frame bytes. Admission refusal still precedes a
   context error when both apply.
6. The unchanged prepared-read/write body receives the same context, keys,
   values and reservation, and uses the same point/batch observation kind.
   Response construction and WireReply encoding are unchanged.

If any guard fails, both adapters return the same status at the same stage
without taking later application transitions. If guards pass, they reach the
same backend call with equal application inputs and an equivalent reservation.
The unary wrapper's repeated successful role check is a stuttering step under
this projection: it changes no state and cannot introduce a new failure.

Cancellation preserves the existing ownership argument. Read preparation
remains inside the cancellable handler future; a prepared blocking read owns
its reservation. Prepared writes still move the original reservation into the
existing independently owned completion path, so dropping observation cannot
release capacity for a live write. Removing an outer future allocation does
not move the reservation out of that path or create a retry.

This is a source-level correspondence under the existing Rust/library and
backend contracts. It is not a machine-checked refinement of the full runtime,
an arbitrary tonic Request lifetime equivalence, or a new core consensus proof.
Core algorithm sources and their existing proof scopes are unchanged. Broader
implementation refinement, process/Chaos acceptance and default promotion
remain separate requirements.

## Focused regressions

New behavioral tests cover both invalid custom identity forms across all five
operations under saturated admission, stream batch error priority and preserved
unary behavior, decoded-size charging with a legal unknown field, and admission
before context validation. One real HTTP/2 stream changes its custom principal
between requests and then revokes the credential; each frame reauthenticates,
the correct principal reaches the backend, and the revoked write does not.

Existing cancellation, panic, response correlation, batch receipt, malformed
response and uncertain-write no-replay tests remain required, together with
unary/admission and optional tarpc feature coverage. Performance screening uses
the unchanged native and Redis clients with a clean f2 control. The experiment
must pass appropriate full acceptance before selection; this document by
itself makes no throughput, fault-acceptance or promotion claim.
