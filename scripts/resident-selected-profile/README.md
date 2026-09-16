# Selected resident-index CPU diagnostic

Offline harness for the production `MemEngine::write_applied` implementation.
It does not run a database server, Raft, WAL, RPC or a QPS benchmark. See the
[profile report](../../docs/RESIDENT-SELECTED-CPU-PROFILE.md).

Input is the same pinned 106-group / 100,096-mutation retained corpus as the
index experiments. Workloads are prepopulated overwrite and unique insertion,
with or without one old snapshot per group. Before recording, every batch state
and optional old view is checked against `BTreeMap`. Each measured pass uses a
fresh engine. Input cloning, initialization, snapshot creation/drop and final
state scans occur outside the recorded monotonic apply spans. Final contents,
applied position and data revision are checked after every pass.

`selected_apply` retains a visible outer call boundary. The only unsafe code in
this harness obtains `CLOCK_MONOTONIC` through a live, correctly typed output
pointer; it does not access the index or bypass ownership. The production engine
sources are unchanged. Timer spans describe this component only, not production
request or Raft positions.

Arguments are `artifact-root output-directory overwrite|unique_insert
pinned|unpinned passes`. The output directory must be freshly created by the
supervisor. After `ready.json`, the harness waits for a `go` file for at most 30
seconds. Do not invoke it without the retained supervisor: recording requires
owned-PID checks, exact source/ELF binding, CPU affinity, recording limits and
independent decoding. The accepted observation build explicitly enables Rust
and C frame pointers; its code generation differs from production.

The [evidence packet](../../docs/resident-selected-profile-v1/README.md) retains
build/capture/decode helpers, plans, original failures, identities and actual
sample coverage. No global perf settings or production configuration changed.
