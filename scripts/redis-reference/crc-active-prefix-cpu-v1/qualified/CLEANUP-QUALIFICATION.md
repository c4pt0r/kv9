# Proposed dummy-process cleanup qualification

The seven original pure prefix contracts passed once, terminal exit 0 (`aa75e2`, PID 814465), with all frozen source hashes unchanged. Their invocation, first log, terminal and before/after maps are retained as `offline-contract-*-first.*`.

`qualify_cleanup.py` is prepared but **not executed**. It exercises the actual frozen `active_prefix.run` child-ownership and cleanup paths using Python dummy processes. The profile adapter, decoder and prefix helper bytes remain unchanged. No KV9 binary, perf, RPC, socket, database, namespace or cluster process is launched.

The proposed command is:

```sh
env PYTHONOPTIMIZE=0 PYTHONDONTWRITEBYTECODE=1 taskset -c 6-15,22-31 /usr/bin/python3 /tmp/kv9-crc-profile-active-prefix-preparation/qualify_cleanup.py --output /tmp/kv9-crc-profile-active-prefix-preparation/cleanup-qualification-first
```

It creates an exclusive new output directory, a local `kv9` symlink to the exact Python executable and a `client` dummy script. Consequently the frozen helper's original argv invokes Python on that local script; its read-like arguments are inert. The exact Python executable device/inode/hash is bound. Three long-lived Python dummy processes supply fake voter lifetimes; perf liveness is explicitly stubbed. This qualifies process cleanup only and cannot establish real sampling or RPC behavior.

Two cases are predeclared:

1. **Exception immediately after spawn:** the existing identity function is replaced in memory only for the newly spawned dummy client to raise a named exception. The helper must retain an incomplete prefix, one tracked identity-incomplete child, empty cleanup errors and an exited/absent process. This targets ownership before `/proc` binding; it does not weaken the real fast-exit identity gate.
2. **Actual timeout and KILL fallback:** the original identity function remains active. Dummy clients print a marker after installing SIGTERM-ignore, then sleep. The unmodified 1,500 ms call deadline must fail the prefix; cleanup must reach SIGKILL for at least one marked TERM-resistant process and reap every tracked child. Existing four-second normal prefix bound and eight-child ceiling remain unchanged; failure cleanup has its separately documented bound.

The harness records real Popen handles through a subclass without replacing process, signal or wait methods. It checks that the helper cleaned every prefix child **before** the qualification's fallback cleanup runs. Shared dummy voter identities must survive both cases, then the qualification stops only those three owned dummies. All final exits, absent PIDs, helper errors, source-before/after hashes and expected negative-prefix summaries are retained. Expected helper failures remain `complete=false`; only the cleanup qualification may pass.

At most 12 dummy lifetimes are expected: three shared voter stand-ins, one exception-path child and up to eight timeout-path children. No test has run yet. An unexpected first failure must remain; do not retry unchanged or patch frozen files in place. Root must coordinate this bounded qualification separately from future real profiling.
