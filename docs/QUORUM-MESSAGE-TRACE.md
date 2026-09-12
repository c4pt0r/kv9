# Quorum-message trace source checkpoint

The next single-GET investigation now has an implemented opt-in recorder and
offline reader on [diagnostic source `1875e74`](https://github.com/c4pt0r/kv9/blob/1875e74141753cc6a55f025384b480b71ee84c1a/docs/QUORUM-MESSAGE-TRACE.md).
It passes source checks and a clean production diagnostic release build, with
no actual trace or new performance result yet.
Main continues to select the previously qualified ThinLTO runtime `11113f6`.

The feature records exact existing ReadIndex contexts across group submission,
peer queue admission/dequeue, inbound validation, follower/leader inbox and
Raft step, exact quorum confirmation and successful apply-covered eligibility.
It adds no protocol fields and preserves fresh Safe ReadIndex, sealed groups,
whole-pump completion, apply/view fences and durable write acknowledgements.
The [source guide](https://github.com/c4pt0r/kv9/blob/1875e74141753cc6a55f025384b480b71ee84c1a/docs/QUORUM-MESSAGE-TRACE.md) states the observational
correspondence argument; it is not a completed proof of core Raft.

Storage is bounded to 65,536 events per process, selected by context sequence
modulo 256. One observer try-lock records each selected boundary or an explicit
loss. Checked local tickets distinguish queue entries; retaining the actual
route allocation prevents reused addresses from joining different routes.
Periodic heartbeats may reuse a context, so repeated messages remain ambiguous.
Cancellation, queue rejection, abandoned tickets and prefix truncation remain
visible. Missing or reversed spans are never assigned zero latency.

A one-shot local marker triggers export after timed clients exit. The file binds
node, PID, start ticks, boot identity and exporter construction time. Timestamps
are process-local; the reader never subtracts clocks across replicas. Missing
intermediate queue stages leave a context incomplete even if a measured endpoint
span remains available. Any observation loss disables complete-context/message
attribution; exact queue-ticket spans may remain explicitly partial.

## Local acceptance

| Configuration | Passed | Existing ignored |
| --- | ---: | ---: |
| Default workspace tests/doctests | 709 | 23 |
| Quorum-trace raft/server tests | 453 | 1 |
| Existing standalone read-stage tests | 438 | 1 |

These populations overlap. All 15 new focused Rust controls also pass, as do
17 finite synthetic reader controls. Formatting and the three explicit Clippy
configurations pass. The first Rust qualification had no failed command or
rerun. The root-owned release-profile cache transaction verifies source
stability and first-party recompilation, with the existing 80 GiB filesystem
floor and 16 GiB additional build reservation. All 149 compiled Rust/Cargo/proto
input hashes match the source snapshot after documentation edits.

[Immutable evidence](https://github.com/c4pt0r/kv9/blob/1875e74141753cc6a55f025384b480b71ee84c1a/docs/quorum-trace-source-v1/README.md) retains exact
commands, full logs, source hashes, cache observations, reader revisions and
terminal records. Original Cargo stdout retains its trailing blank lines;
source/guide whitespace checks pass with raw evidence excluded. No hosted CI
was dispatched. Older recovery and Chaos results retain their original source
scope; this stage introduces no new acceptance for either.

The [first production diagnostic build](quorum-trace-release-v1/README.md) also
passes. Its server SHA256 begins `fe19660e`; all 738 source-file hashes and the
exact non-test release feature graph were independently checked. No testing or
alternate RPC feature is enabled. Build success is not runtime acceptance.

## Next execution

1. Use the committed diagnostic server and its verified release binding. Keep
   the previously pinned workload client.
2. Run the existing c1 GET workload on three voters with an uninstrumented
   control and both run orders. Capture only after measurement, retaining
   process lifetimes, fresh drains, full outcomes and original storage/CPU guards.
3. Read each replica's trace independently, account for loss and ambiguous
   contexts, and quantify instrumentation overhead. Instrumented QPS is not
   an optimization result.
4. Change the dominant measured source boundary, then compare uninstrumented
   throughput and latency and apply the appropriate correctness/fault gates.
   Earlier rejected transport/worker experiments need new causal evidence.

[The previous accepted read numbers](RELEASE-THIN-LTO-FULL72.md) remain
28,314.673 c1 GET/s and 375,885.286 c64 GET/s, with 35.206 us and 170.140 us mean
latency respectively. They are not new runs. Redis parity, write-tail regression,
full implementation proofs, broader actual Chaos Mesh/host acceptance, bounded
storage, dynamic multi-Raft and automatic splits remain open. Only the object
store may be a service-critical singleton.
