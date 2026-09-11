# Consume owned buffers while planning a RawKV proposal

This isolated candidate starts from CRC baseline `ca0002c7`. The separate
owned-buffer apply experiment changes final resident-index insertion and was
rejected for performance regressions. This candidate removes copies earlier,
while constructing the proposal. Neither experiment's correctness or
performance evidence transfers to the other source. Local source checks pass;
release recovery, Chaos acceptance and performance measurement remain pending.

## Changes and unchanged protocol boundaries

`WriteBatch::into_mutations` consumes the existing ordered vector.
`Command::fenced_write_from_owned_batch` moves its buffers into the same
`Command::Fenced { inner: FencedInner::Write { .. }, .. }` representation.
The existing borrowed constructors remain available for callers that retain
their plans. Metadata catalog constructors and their wire tags are unchanged.

`RawExecutor::plan_owned_batch_put` takes the already-owned async BatchPut
request. It encodes each physical key in the original order and moves the
value into its plan. The async runtime still validates the complete borrowed
request span before consuming that request. The synchronous borrowed batch
API retains its original planner.

Both runtime proposal paths already own their final plan. They now consume it
into the fenced command before the original submission call. A single command
and a single fence remain retained across the existing replacement policy;
the async path keeps its original `Arc<Command>`. Admission, blocking-worker
ownership, deadlines, cancellation, retry eligibility, quorum confirmation
and exact committed/applied acknowledgement handling are unchanged.

For an async BatchPut pair, this removes the planner's value clone and the
command conversion's key/value clones. A point PUT removes the command
conversion's key/value clones. Physical-key encoding, command serialization,
replication and apply still do their required work. These are source-level
copy counts, not measured allocation counts or a predicted speedup.

## Equivalence argument

Let the byte interpretation of an owned vector be its ordered byte sequence.
Moving a vector and cloning it have the same interpretation. The engine and
command APIs expose bytes, not the original allocation address.

For a mutation `Put(cf, key, value)`, both constructors produce
`KvOp::Put(cf_code(cf), key_bytes, value_bytes)`. For a Delete they produce
`KvOp::Delete(cf_code(cf), key_bytes)`. Both consume or borrow the same vector
in order. Equality of the empty operation list and preservation under each
appended mutation prove equality of the complete operation sequence by
induction. This includes duplicate keys, empty values and all column families.
Both constructors attach the unchanged fence fields and the same inner tag.
The unchanged deterministic encoder therefore produces identical log bytes.

For batch planning, both entry points first reject the same unsupported
options, then call the same physical-key encoder for each original key in
the same order. Replacing `value.clone()` with the owned value preserves
the planned mutation's bytes. Induction over the input pairs yields the same
successful plan. The first encoding error occurs at the same pair and no
partial plan is returned or applied. An empty input encodes no key in either
planner; it retains the original executor-level behavior even for an invalid
keyspace ID. The runtime's context gate still precedes planning.

The conversion owns no storage or driver handle and has no persistent effect.
Only the existing runtime submission path can propose the converted command;
the existing apply-time fence adjudicator still decides its effect. Since
the byte payload and those control-flow boundaries are unchanged, the
optimization introduces no new election, read-confirmation or recovery rule.

This argument relies on Rust's vector ownership semantics and the existing
encoder contract. It is not a new machine-checked whole-Rust/Raft refinement
or a proof of identical allocation-failure behavior.

## Validation plan and current status

The new command regression compares owned and borrowed command values and
encoded bytes, then decodes the result through the existing decoder. Cases
include empty batches, all column families, binary/empty keys, duplicate
Put/Delete/Put order, empty values and extreme fence fields. The generic
lowering path must still refuse the fenced envelope.

Two planner regressions check final engine reads across duplicate keys,
binary/empty keys, empty values and two tenants, and preserve option-error
precedence, invalid-keyspace refusal and empty-batch behavior. Existing runtime
tests must also continue to establish context/fence authorization and the
unchanged proposal-completion/cancellation semantics.

The first focused invocations pass exactly one command regression and two
planner regressions. The first subsequent `cargo test --workspace` invocation
passes 712 tests with 23 ignored and no failures. All-target Clippy with
warnings denied and `cargo fmt --all -- --check` also exit zero. Compilation
and these checks start after the owned-apply timing and independent audit
finish. The four edited source files retain their pre-test SHA-256 values;
only this status document is updated afterward. All checks run locally.

The candidate still requires its own original release, process/Chaos
acceptance and matched throughput/whole-call-latency comparison against the
selected baseline. No performance gain or mainline promotion is claimed yet.
