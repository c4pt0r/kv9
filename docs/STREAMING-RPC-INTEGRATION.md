# Normal-port streaming integration checkpoint

The integration candidate promotes the selected tonic streaming adapter into
the ordinary server and `PersistentRawClient::new`. Public unary, streaming
and Raft services share the configured/advertised endpoint. Tarpc remains an
explicit feature-gated control. Native atomic RawBatchGet/RawBatchPut client
semantics and limits are documented in [RAW-BATCH-CLIENT.md](RAW-BATCH-CLIENT.md).

The wire package is now `kv9.point.v1`. Its bounded envelope reuses the existing
public protobuf payloads and backend/admission handlers. Opening headers and
every frame are authenticated. Runtime drop cancels the normal service pumps.
Generated Debug output excludes credentials and payloads.

Workload reports advance to version 2 and always record `rpc_transport`.
Omitting it in input selects `tonic_stream`. Normal unary/stream artifacts do
not require the experimental feature. The validator preserves the original
version-1 experimental-artifact checks. Calibration explicitly selects unary;
normal benchmark and persistent Chaos configurations explicitly select streaming,
preserving exact requested-versus-recorded configuration comparisons.

## Local correctness checkpoint

All commands use the integration checkout, `--locked`, local CPUs 6-31, and
`CARGO_TARGET_DIR=/home/dongxu/kv9/target`. No hosted workflow was dispatched.

| Gate | Result | Retained output |
| --- | --- | --- |
| Server library tests | 191 passed, 0 failed, 1 ignored | `/tmp/kv9-normal-stream-server-tests-first.log` |
| Default workspace tests | 678 passed, 0 failed, 23 ignored | `/tmp/kv9-normal-stream-default-workspace-first.log` |
| Experimental workspace tests | 688 passed, 0 failed, 23 ignored | `/tmp/kv9-normal-stream-feature-workspace-first.log` |
| All-target experimental Clippy, warnings denied | Passed | `/tmp/kv9-normal-stream-clippy-first.log` |
| Python/shell and compatible historical controls | Four genuine v1 histories passed; twenty corruptions rejected | `/tmp/kv9-normal-stream-python-acceptance-first/result.json` |

The adapter tests include normal shared-port default selection, batches with
ordered missing/empty/duplicate results, exact protobuf byte boundaries,
malformed batch cardinality, whole-batch cancellation/deadline reservation,
unknown-write non-replay, and authentication before stream reservation. An
independent read-only production review found no concrete correctness blocker.
This review is not a formal proof or a fault-matrix result.

Two intermediate compile attempts are retained: the first all-target command
encountered the not-yet-created batch test module; the feature all-target
attempt then exposed missing test-only Serialize/KeyspaceId imports after the
codec extraction. Both were fixed before the successful test/Clippy gates.
The first generic legacy-control attempt used an incompatible proposal-queue
metric vocabulary; its failed log remains preserved and compatible untouched
mainline histories subsequently passed. No protocol/runtime test failure was
observed in these gates.

## Remaining promotion gates

The default-feature process fixture and its v2 negative controls must run on
clean retained standalone server/client binaries. Its leader-loss/restart
histories are point-operation evidence, not atomic batch fault coverage.
The local Chaos Mesh environment has been checked read-only and is available;
a fresh candidate-bound build/image/observer overlay must precede the original
21-window point matrix. Historical namespaces and failed attempts remain intact.

A separate atomic batch history format/checker and actual Chaos Mesh batch
fault run remain required. Batch throughput and mean/p50/p95/p99 must be measured
with explicit batch sizes, RPC and input-item counts, and all failure populations.
The previous `40e813f` performance numbers remain evidence for that frozen
point-RPC experiment only. This checkpoint does not assert production readiness,
new performance gains, Redis parity, full formal verification or completed
batch Chaos acceptance. The original issue #9 roadmap checklist is unchanged.
