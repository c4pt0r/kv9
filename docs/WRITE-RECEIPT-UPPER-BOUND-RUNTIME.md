# Upper-bound receipt lookup: release and ordinary recovery

The exact experimental candidate
[`e2e23cc`](https://github.com/c4pt0r/kv9/commit/e2e23cca5e70a9ea0cc241877b3b35b5b6433d27)
now has accepted default/native and diagnostic releases plus ordinary
three-voter recovery. The subsequent [actual full21 Chaos Mesh gate](WRITE-RECEIPT-UPPER-BOUND-CHAOS.md)
also passes. **CRC remains selected. The complete matched performance screen
and a live schema-2 observer capture remain pending.**
These results do not establish a throughput improvement.

## Retained release identity

Both releases bind the same clean 1,116-file source inventory and the existing
[source-bound proof](WRITE-RECEIPT-UPPER-BOUND.md). The default server and native
correctness client were built first; only the additional diagnostic server was
built afterward. The second build verifies the retained default artifacts
before and after, without rebuilding them or inventing a shared manifest.

| Artifact | SHA-256 |
| --- | --- |
| Default server | `6f074e867eae45274c17b54aa763888e92db2c45d5757974fa52ee0f15936d49` |
| Same-source native correctness client | `44d0f6429fd549fc7d0fac6e71ba67b413a2a8aca0e3e3f0386a64058ac06769` |
| Diagnostic server | `dbfb271302cc079c5fdd249a4c011a28aa759f4ad036b5e3707d446e078bc2e5` |

Actual Cargo records and verbose compiler invocations confirm Rust/Cargo
1.94.0, release opt3, ThinLTO, one codegen unit and the default unwind strategy.
Default production packages have no optional features. Only the diagnostic
server enables `write-path-diagnostics` on the root, Raft and server packages.
The retained build lock, eight-package cache invalidation, offline/locked
four-job builds and protected reference binaries pass readback.

Default/native build terminates at `3119/78ef53/0`; its independent readback is
`debafc/0`. Diagnostic build terminates at `56645/df82cf/0`; its independent
readback is `0bd207/0`. These are local tool receipts, not hosted CI runs.

## Recovery evidence

The retained default server and same-source client run on three ordinary WAL
voters. Both default streaming and explicit unary arms mix Get, Put, Delete,
BatchGet and BatchPut over four shared keys with four workers, batch size 8
and 128-byte values. Each arm kills its current leader with SIGKILL, observes
a replacement leader and restarts the killed voter in its original directory.

| RPC | Completed operations | Successful | Explicit unknown | Overlapping point/batch pairs |
| --- | ---: | ---: | ---: | ---: |
| Default Tonic stream | 180 | 162 | 18 | 117 |
| Explicit Tonic unary | 180 | 168 | 12 | 123 |
| Total | 360 | 330 | 30 | 240 |

Both complete atomic histories independently validate, including all unknown
outcomes. These are completed client observations, not 360 acknowledged writes.
Successful operations occur within both the leader-loss and restart windows;
all five API kinds appear in each window. The audit verifies shared-key overlap,
the original-directory restart, advertised listener ownership, CPU placement
and exact executable/process identities.

All six voter drains have fresh serial exporter advances and zero outstanding
admission/Raft queues. All five server and two client lifetimes exit. Recovery
terminates at `81788/4be379/0`; independent audit is `e09584/0`.

Each phase retains its original 24 GiB + 8 MiB launch minimum, 8 GiB available
floor and 16 GiB maximum sampled decline. [Capacity cleanup](WRITE-CAPACITY-COMPLETION.md) completed before
the builds; every subsequent phase checked fresh availability. These sampled
guards are not hard disk reservations.

This checkpoint covers process failure and restart on one host. It does not
model physical power loss, establish cross-host availability, replace the
actual Chaos Mesh gate or prove all of Raft. The source checkpoint's earlier
diagnostic TCP bind failure remains preserved; this runtime pass does not
repair or relabel that failed test run.

## Remaining acceptance

1. Qualify fresh capacity and run the original 8 smokes and 16 ten-second matched performance cohorts
   against selected CRC, with the fixed native v3 measurement client. Compare
   throughput and latency in both opposite orders.
2. Run the separate schema-2 observer capture to measure actual skipped scans.
   Keep the held receipt-tail hint/fallback comparison separate.
3. Decide promotion from complete evidence, then continue the industrial
   storage and dynamic multi-Raft/split dependencies in issue #9.

[Original release logs, manifests, complete histories, runtime records and audit](receipt-upper-bound-runtime-v1/README.md)
are retained. All CI and tests remain local; no hosted workflow was dispatched.
