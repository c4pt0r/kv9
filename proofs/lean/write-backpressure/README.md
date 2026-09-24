# Write-backpressure model

A machine-checked model of end-to-end write backpressure
(`crates/server/src/runtime/range_api.rs`, `KV9_MAX_RAFT_LOG_ENTRIES`,
default 0 = disabled): issue #20's bounded-absolute-log-size item. When
writes outrun commit and compaction the RETAINED committed raft log
(`raft_committed - log_first_index`) would grow without limit; the gate
bounds it.

The engine's own accounting — a committed append advances
`raft_committed` by one, a committed compaction floor raises
`log_first_index` — is a premise from the storage layer and its unit
tests. This model constrains the abstract retained length under those
transitions:

- a write is admitted only STRICTLY below the cap — the gate is
  pre-append and write-only (`permit`, not `view`);
- at or beyond the cap every write is refused with a TYPED, retryable
  refusal (`Error::WriteBackpressure` → `RESOURCE_EXHAUSTED`) that maps
  the state to itself: nothing proposed, nothing queued, the caller
  backs off and retries;
- the retained log therefore NEVER exceeds the cap;
- reads are never gated — the read transition is available at any
  retention, including at or beyond the bound, and changes nothing;
- a committed compaction floor drains the log and reopens writes —
  backpressure releases, it is not a permanent wedge;
- and the bound is tight, not vacuous: the log can actually fill to it.

Ten checked theorems: `initial_safe`, `step_safe`, `run_safe`,
`retained_never_exceeds_the_bound`,
`an_admitted_write_was_below_the_bound`,
`no_write_grows_the_log_at_the_bound`,
`a_full_log_refuses_writes_typed`, `reads_serve_at_any_retention`,
`draining_releases_backpressure`, `the_log_can_fill_to_the_bound`.

`scripts/prove-write-backpressure.py` compiles the model with
`-DwarningAsError=true`, audits every theorem's axiom dependencies down
to `{propext, Classical.choice, Quot.sound}`, and drives five semantic
defect controls (admit at the bound, an append that grows by two, a
refusal that mutates state, reads gated by the bound, a drain that grows
the log) plus the two policy controls (`sorry`, injected `axiom`) — each
must fail to compile. The real-process end-to-end fixture is
`scripts/write-backpressure-e2e.py`.
