# TCP fixture port ownership repair

The three-node real-TCP test previously selected ephemeral addresses by
opening temporary listeners and closing them before binding the actual
transports. Another test or process could acquire one of those ports during
that gap. The initial upper-bound diagnostic run hit `AddrInUse`; its isolated
unchanged-source retry passed. The original failed run remains in the
[source checkpoint](../receipt-upper-bound-source-v1/README.md).

The test now binds each real transport directly to `127.0.0.1:0` and retains
every listener. It collects their actual local addresses and registers peers
before starting the driver pump threads. The kernel owns port allocation for
the full transport lifetime. Discovery, negative discovery, election,
replication, status and shutdown assertions are unchanged. Production transport
and Raft logic are unchanged.

The exact diagnostic test passes **1 test, 0 failed, 254 filtered out**, followed
by `cargo fmt --all --check`. Validation ran after the candidate's full Chaos
runtime and all post phases had terminated. It used the retained-build lock,
shared target, offline/locked Cargo, four jobs and the original development
space/time guards. All recorded Rust/Cargo inputs stayed unchanged during
both commands. Actual terminal: **50533/1e1e84/0**.

This repairs the transport listener allocation race on main. It does not
retroactively pass the earlier failed suite or change the frozen experimental
server already tested under Chaos. The original negative discovery probe still
uses its separate no-responder case.

`run.py`, `runtime-inputs.json`, both command logs, `result.json` and
`terminal.json` retain the exact local validation. `inventory.json` binds every
copied original file by length and SHA-256. No test was replayed for publication.
