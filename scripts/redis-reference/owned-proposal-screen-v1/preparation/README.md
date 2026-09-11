# Owned proposal buffer write-only screen

Frozen candidate `71c9d996e1dcc0897da14be1886e669f0c3ea773` is compared with selected CRC `ca0002c7f8e9ee6f595efcc9f4151085ccce87cb` and unchanged Redis/native clients `0be806d9671e2c50701a64aa7889c8859b7648ba`. Actual original release pins are in `release-pins-first.json`; no executable pins remain pending. Historical pending drafts remain separately retained.

This is an explicit write-only selection screen: c1/c64 × point PUT1/BatchPut64 × CRC/candidate/Redis, with a 12-cell forward inventory followed by its complete reversal (24 timed cohorts, 10 seconds each). The separate correctness smoke is the first 12 cells at 2 seconds. It establishes no read/mixed workload acceptance or promotion.

The accepted broad protocol supplies all applicable outcome, request-attempt, deterministic mutable-data, source/build/process/listener identity, resource, fresh drain, retention and cleanup gates. Wrapper and statistics core bytes are identical. Batch latency remains whole-call latency; success and error populations remain visible. Final deterministic nonce/write-key membership does not identify exact issued nonces or prove full linearizability. The 16 MiB public-byte limit is the inherited benchmark fixture override; production defaults to 64 MiB. No capacity was changed.

Storage guards remain identical: each cohort needs tmpfs ≥32 GiB and retention disk ≥96 GiB; periodic samples stop invalid/incomplete timing below 16/64 GiB. Existing endpoint guards also remain. Shared-host tmpfs results establish neither equal durability nor sustained capacity. The same raw histogram/statistics arithmetic is retained.

All 26 offline contracts passed on their first execution (7 driver, 15 auditor, 4 screen); exact logs and source hashes are retained. `preparation.json` and adjacent diffs describe the finite adaptation. A metadata-only argv-index assertion failure is retained separately and corrected without changing tested helpers. No runtime, actual-result audit or statistics execution occurred during preparation.

`commands.json` contains exact root-only smoke, isolated timing, once-only audit and statistics argument vectors. Replace only `ROOT_TERMINAL_TIMING_SESSION` in the audit invocation with the actual exited timing session; options and values are separate arguments. The statistics reader requires `/tmp/kv9-owned-proposal-screen-root-preparation/audit-terminal-first.json` with `exit_code=0`, `terminal=true`, and the matching `timing_session`. Root must finish all other helper/profile work before the timed command.

Expected full readback inventory: 24 cohorts (16 native/8 Redis), 80 owned lifetimes, 48 fresh drain stages, 48 voter/listener bindings and 96 native endpoint envelopes. Endpoint inventories bind retained inputs; stage-semantic analysis is separate. Preserve any runtime or validation failure; do not rerun to green.
