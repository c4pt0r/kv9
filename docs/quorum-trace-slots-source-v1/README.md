# Immutable-slot trace source checks

The predecessor recorder lost 1,083 selected observations to its shared observer
mutex in the original four-cohort campaign. This source changes only the opt-in
recorder storage to preallocated, uniquely reserved immutable cells. It retains
the existing reader/schema, sampling, capacity, capture cut and all Raft fences.
See [the source argument](../QUORUM-MESSAGE-TRACE.md#safety-correspondence).

The original local source gate passed formatting, diagnostic compilation,
17 focused tests, 455 diagnostic Raft/server tests (one existing ignored), and
Clippy with warnings denied. Test populations overlap. Root session 90236
terminated with exit 0 (`ae1c03`). Full command logs and source/cache bindings
are retained here; original bytes are not normalized. The default/standalone
read-stage source paths have no changed compiled code; prior results retain
only their original scope. Production release and actual capture are separate.
