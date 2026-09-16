# Allocation-only engine interface companion

This standalone executable accompanies the isolated triomphe experiment. It
copies the [engine interface harness](../engine-interface-experiment/README.md),
preserving its operations, state checks, captured views, pass counts and window
boundaries. It emits **no elapsed-time observations**. The latency experiment
uses its separately retained, uninstrumented binaries.

The existing `resident-outlined-mutation/src/counting.rs` allocator wrapper is
reused byte-for-byte. Each window records allocation calls/requested bytes,
reallocation calls/new requested bytes, deallocation calls/requested bytes,
net requested live-byte change and peak extra requested live bytes. These are
observations of this instrumented Rust allocator path, not RSS, jemalloc arena
usage or proof of the uninstrumented executable's physical allocation behavior.

Counter buffers are reserved before windows. Bookkeeping, result serialization
and validation happen after counters stop. Read return values remain alive until
after the pass, matching the timing harness. Input write-batch construction and
old-snapshot acquisition/drop are excluded. Snapshot boxing is counted, and its
drop is outside the window. A separate untimed dynamic-size observation before
snapshot warmup identifies the expected box allocation size.

The build helper checks a strict source projection: after removing only the
allocator/metric implementation, four window-start replacements, output labels
and the untimed size observation, the remaining source must exactly match the
original harness. The counter and query-selection modules must also match their
retained originals. No previous timing source is edited.

Use the [screen helpers](../isolated-outline-performance/README.md) to build,
qualify and execute this companion. Its output unit is
`allocator_counts_per_window`; `operation_window` records the corresponding
timing scope without reporting any counted duration. Do not convert the process
runtime or counting records into a latency or throughput result.
