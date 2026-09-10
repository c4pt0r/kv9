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

The default-feature process fixture and its fifteen v2/legacy negative controls
have now passed on clean retained standalone server/client binaries; see
[the independently audited process checkpoint](STREAMING-RPC-PROCESS-ACCEPTANCE.md).
Its leader-loss/restart histories are point-operation evidence, not atomic
batch fault coverage.
The fresh normal-build [21-window point Chaos Mesh acceptance](STREAMING-RPC-CHAOS-ACCEPTANCE.md)
has passed on exact `f0eaf23`, including independent full histories, effects and
post-history drain observations. Historical namespaces and failed attempts
remain intact. This covers point operations, not native batches.

The separate [atomic batch history checker and three-voter process fixture](NATIVE-BATCH-ACCEPTANCE.md)
have passed, including independent audits and copied-artifact counterexamples.
The scoped [native batch safety proof](NATIVE-BATCH-PROOF.md) also passed its
fresh TLA+/TLAPS gate with ordered mutation/read projection, receipt binding
and retry refinement. Consensus, fencing, engine atomicity and established
read authority remain its explicit premises; whole-adapter verification stays
open. The [client-link preflight](NATIVE-BATCH-LINK-PREFLIGHT.md) qualified
Service VIP fault selectors and same-process socket reset observation.
The [original 21-window matrix with native atomic histories](NATIVE-BATCH-CHAOS-ACCEPTANCE.md)
now has independently accepted evidence on exact `5cc9861`. Its original
fixture exit remains 1 due to a final timestamp parser failure; a separately
checked read-only adapter accepted the unchanged run without repeating faults.
Scoped cleanup completed with historical namespaces preserved. Dedicated
native client-link/reset and quorum-loss acceptance remain required.
Batch throughput and mean/p50/p95/p99 must be measured
with explicit batch sizes, RPC and input-item counts, and all failure populations.
The previous `40e813f` performance numbers remain evidence for that frozen
point-RPC experiment only. This checkpoint does not assert production readiness,
new performance gains, Redis parity, full formal verification or completed
batch Chaos acceptance. The original issue #9 roadmap checklist is unchanged.
