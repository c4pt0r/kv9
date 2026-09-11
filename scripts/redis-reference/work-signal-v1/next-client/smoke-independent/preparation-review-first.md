# V3 smoke preparation review

Source reviewed: clean `0be806d9671e2c50701a64aa7889c8859b7648ba` in `/tmp/kv9-point-write-measurement-v3`. The first draft of `/tmp/kv9-point-write-v3-smoke-preparation/run.py` was still being prepared. No runtime result was audited at this point.

One source-derived issue was reported to root and the fixture owner before launch. `check_populations` treated the measurement nonce set as a contiguous prefix ending at `warmup_calls + measured_issued`; `dataset_check` used that bound to claim exact issued-write membership. Native and Redis workers allocate a slot before the final cutoff check in `traffic`. A delayed slot may therefore be abandoned after later slots were issued. The aggregate report does not record the issued nonce set. Exact warmup counts are supported; measurement API/outcome totals and deterministic value/maximum configured nonce/write-key membership can be checked without claiming an issued-nonce ledger.

The reviewed draft otherwise retains explicit v3 point/batch selectors, all-success one-attempt checks, source/default-feature bindings, same-port three-voter ownership, three fresh-drain stages per native case, full 128-key plus sentinel final readback, Redis listener/configuration identity, and owned cleanup. These are aggregate-client correctness checks, not a full operation-history, linearizability, Chaos, durability or performance acceptance.

This note records the initial draft finding. A later frozen script and actual first-run result require separate readback; this note is not acceptance.
